# Architektur-Entscheidungen

## ADR-0001: Wiki-Speicherung – Markdown-Dateien als Source of Truth

**Status:** angenommen · 2026-06-11

### Frage

Sollen Wiki-Einträge als reine Markdown-Dateien in einem lokalen Ordner
gespeichert werden, oder in einer vollständig integrierten Datenbanklösung
(LanceDB, Qdrant, eingebettetes ChromaDB)?

### Entscheidung

**Hybrid: Markdown ist die Source of Truth, der Vektorindex ist abgeleitet.**

1. **Persistenz:** Jeder Eintrag ist eine `.md`-Datei mit YAML-Frontmatter
   (Titel, Timestamp, Tags, Summary, verwendetes Modell) in `VIBE_WIKI_DIR`.
2. **Semantik:** Embeddings werden beim Start (und bei jedem `/save`) aus den
   Dateien berechnet und nur im Speicher gehalten. Ein persistenter Cache
   (`.vibe/index.json`) ist Roadmap – er bleibt aber immer *Cache*, nie
   Wahrheit.

### Begründung

- **Lesbarkeit ohne Tool** (explizite Anforderung): `cat`, Obsidian, VS Code,
  `grep` – alles funktioniert sofort. Eine DB-Datei wäre opak.
- **Datenschutz & Offline-NFR:** Klartextdateien im Nutzerverzeichnis sind
  trivial zu auditieren, zu sichern und zu synchronisieren (git, Syncthing).
- **Robustheit:** Ein korrupter Index ist folgenlos – `rm -rf .vibe` und der
  Index wird aus den Markdown-Dateien rekonstruiert. Umgekehrt (DB als
  Wahrheit) wäre Datenverlust endgültig.
- **Ressourceneffizienz-NFR:** Kein eingebetteter DB-Prozess, keine zweite
  Speicherhierarchie. Für die Zielgröße eines persönlichen Wikis (10²–10⁴
  Notizen) ist lineare Cosine-Suche im RAM im Mikrosekundenbereich.
- **Migrationspfad:** Wächst das Wiki über ~10⁵ Einträge, kann der abgeleitete
  Index 1:1 durch LanceDB ersetzt werden, ohne dass sich am Dateiformat oder
  an der `Index`-API etwas ändert.

### Konsequenzen

- Embeddings liegen in `.vibe/embeddings.json`, geschlüsselt über
  Hash(Embedding-Modell, Inhalt). Unveränderte Einträge kosten beim Start
  keinen Inferenz-Call; ein Modellwechsel invalidiert den Cache implizit.
- Gleichzeitiges Schreiben durch externe Tools wird nicht überwacht (kein
  File-Watcher in v0.1) – der Index aktualisiert sich beim nächsten Start.

### RAG-Query

Als Suchanfrage dient das Fenster von ±6 Zeilen um den Cursor, nicht der
Gesamttext – sonst verwässert ein langes Dokument das aktuelle Thema.
nomic-Modelle bekommen die Task-Präfixe `search_document:` /
`search_query:`; der Cosine-Schwellwert liegt bei 0,6, weil diese Modelle
auch für Unverwandtes ~0,5 liefern. Lieber ein leeres Panel als Rauschen.

## Datenpfad-Transparenz

- Statuszeile zeigt permanent `🔒 LOKAL` oder `☁ CLOUD` (aus dem
  `ModelSwitched`-Event, nicht aus dem Label geraten).
- Cloud-Consent: Der erste `/model claude|gpt` pro Sitzung wird in der UI
  abgefangen und muss wiederholt werden; erst dann geht der Befehl an den
  Worker. Ghost-Text (`complete_local`) und Embeddings nutzen ausschließlich
  Ollama – unabhängig vom gewählten Hauptmodell.
- `/ghost off` schaltet die Hintergrund-Inferenz ab; eine laufende
  Ghost-Korrektur wird bei jeder neuen Eingabe abgebrochen (`JoinHandle::abort`).

## Crash-Recovery

In jeder Tipp-Pause schreibt der Worker den Puffer nach `.vibe/autosave.md`.
Ein sauberes Ende (Ctrl+Q) löscht die Datei; existiert sie beim nächsten
Start, wird sie in den Puffer geladen.

---

## 3-Säulen-Concurrency-Modell

| Säule (Spezifikation) | Umsetzung |
|---|---|
| UI-Thread | Haupt-OS-Thread: ratatui-Renderloop, 16 ms Event-Poll (~60 FPS). Liest `AppEvent` per `try_recv` – blockiert nie. |
| Inferenz-Thread | Tokio-Multi-Thread-Runtime: Debounce-Loop (450 ms `select!`-Timer) + pro Anfrage gespawnte Tasks. Lokal: Ollama (`/api/chat`, `/api/embeddings`); die Engine nutzt CUDA/Metal selbst. |
| I/O-Thread (Cloud & Storage) | Eigene Tokio-Tasks: Cloud-Calls (reqwest, rustls), Wiki-Persistenz, Index-Aufbau. Der Index liegt hinter `Arc<RwLock>` – Suchen blockieren Schreiben nicht. |

Kommunikation ausschließlich über zwei `unbounded_channel`:
`WorkerMsg` (UI→Worker) und `AppEvent` (Worker→UI). Es gibt keinen geteilten
veränderlichen Zustand zwischen UI und Workern.

### Ghost-Text-Invalidierung

Jede Pufferänderung erhöht einen `generation`-Zähler. Ein Ghost-Vorschlag wird
nur angezeigt, wenn (a) seine Generation noch aktuell ist und (b) die
Originalzeile im Puffer unverändert vorliegt – damit kann ein verspäteter
Inferenz-Task niemals veralteten Text einschleusen. Übernahme per `Tab` prüft
dieselbe Invariante erneut.

### Stabilitätsgarantie (NFR)

Puffer-Transformationen folgen dem Muster *propose-confirm-replace*:

1. Der Worker sendet `Proposal` ausschließlich nach erfolgreicher
   LLM-Antwort; jeder Fehler (Timeout, Netzabbruch, API-Fehler, Refusal)
   wird zu `Error` und lässt den Puffer unangetastet.
2. Die UI zeigt den Vorschlag als Wort-Diff (LCS über Token, `src/diff.rs`)
   und ersetzt den Puffer erst nach Bestätigung per Enter. Bei sehr großen
   Texten (> 4M DP-Zellen) fällt der Diff auf eine Grob-Anzeige zurück,
   damit der UI-Thread nie spürbar rechnet.
3. Vor jeder Übernahme (Transformation wie Ghost-Korrektur) wird ein
   Snapshot auf den Undo-Stack gelegt – `Ctrl+Z` widerruft jede KI-Änderung
   in genau einem Schritt. Tipp-Eingaben werden zeitlich koalesziert
   (800 ms-Fenster), der Stack ist auf 200 Einträge begrenzt.

## Modell-Schicht

| Modell | Anbindung | Verwendung |
|---|---|---|
| Gemma (lokal) | Ollama HTTP, `OLLAMA_BASE_URL` | Standard; einzige Quelle für Ghost-Text und Embeddings |
| Claude (`claude-opus-4-8`) | Anthropic Messages API, raw HTTP | `/model claude` – komplexe/kreative Aufgaben |
| GPT (`gpt-4o`) | OpenAI Chat Completions | `/model gpt` |

Anthropic-Besonderheit: `stop_reason == "refusal"` wird vor dem Lesen des
Contents geprüft und als Fehler gemeldet (Puffer bleibt erhalten).
