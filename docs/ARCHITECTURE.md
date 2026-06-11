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

- Startzeit skaliert mit der Wiki-Größe (Embedding-Berechnung), bis der
  Embedding-Cache implementiert ist.
- Gleichzeitiges Schreiben durch externe Tools wird nicht überwacht (kein
  File-Watcher in v0.1) – der Index aktualisiert sich beim nächsten Start.

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

Puffer-Transformationen folgen dem Muster *replace-on-success*: Der Worker
sendet `ReplaceBuffer` ausschließlich nach erfolgreicher LLM-Antwort; jeder
Fehler (Timeout, Netzabbruch, API-Fehler, Refusal) wird zu `Error` und lässt
den Puffer unangetastet.

## Modell-Schicht

| Modell | Anbindung | Verwendung |
|---|---|---|
| Gemma (lokal) | Ollama HTTP, `OLLAMA_BASE_URL` | Standard; einzige Quelle für Ghost-Text und Embeddings |
| Claude (`claude-opus-4-8`) | Anthropic Messages API, raw HTTP | `/model claude` – komplexe/kreative Aufgaben |
| GPT (`gpt-4o`) | OpenAI Chat Completions | `/model gpt` |

Anthropic-Besonderheit: `stop_reason == "refusal"` wird vor dem Lesen des
Contents geprüft und als Fehler gemeldet (Puffer bleibt erhalten).
