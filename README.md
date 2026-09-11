# Vibe-Editor CLI

AI-Powered Text Workbench im Terminal: lokales Gemma-Modell für latenzfreie
Live-Korrekturen (Ghost-Text), Cloud-Modelle für komplexe Transformationen,
und ein autonomes Markdown-Wiki mit proaktivem RAG – alles in einer flüssigen,
asynchronen TUI.

## Quickstart

```sh
# Voraussetzungen (optional, aber empfohlen): Ollama mit Gemma + Embedding-Modell
ollama pull gemma3
ollama pull nomic-embed-text

cargo run --release
```

Ohne Ollama startet der Editor trotzdem – Ghost-Text ist dann deaktiviert und
per `/model claude` (bzw. `/model gpt`) wechselst du auf Cloud-Inferenz.

## Bedienung

| Taste | Funktion |
|---|---|
| `Ctrl+P` oder `/` (auf leerer Zeile) | Command-Bar öffnen |
| `Tab` | Ghost-Vorschlag übernehmen / Befehl vervollständigen |
| `Esc` | Ghost-Vorschlag verwerfen / Command-Bar schließen |
| `Ctrl+Z` / `Ctrl+Y` | Rückgängig / Wiederholen (auch für KI-Änderungen) |
| `Ctrl+S` | Puffer ins Wiki speichern (mit Auto-Titel/-Tags/-Summary) |
| `Tab` (ohne Ghost) / `Entf` | 4 Leerzeichen einfügen / vorwärts löschen |
| `Ctrl+Q` | Beenden |

`vibe notizen.md` öffnet eine Datei direkt (fehlt sie, wird sie beim ersten
`/write` angelegt). In jeder Tipp-Pause sichert der Editor den Puffer nach
`.vibe/autosave.md`; nach einem Absturz wird er beim nächsten Start
automatisch wiederhergestellt.

**Datenpfad-Badge:** Die Statuszeile zeigt permanent `🔒 LOKAL` oder
`☁ CLOUD`. Der erste Wechsel auf ein Cloud-Modell pro Sitzung muss durch
Wiederholen von `/model …` bestätigt werden – vorher verlässt kein Zeichen
das Gerät. Ghost-Text und Embeddings laufen *immer* lokal.

**Transformationen ersetzen den Puffer nie direkt:** Das Ergebnis von
`/lang`, `/style`, `/summarize`, `/todo` und `/expand` erscheint als
Wort-Diff-Vorschau (grün = neu, rot durchgestrichen = entfernt) und wird
erst mit `Enter` übernommen – `Esc` verwirft. Jede übernommene KI-Änderung
ist per `Ctrl+Z` widerrufbar.

### Befehle

| Befehl | Wirkung |
|---|---|
| `/lang en` | Puffer nahtlos übersetzen |
| `/style professional` | Register/Stil live umformen |
| `/summarize` | Hochverdichtete Zusammenfassung |
| `/todo` | Action-Items als Checkliste extrahieren |
| `/expand` | Stichpunkte ausformulieren |
| `/model gemma\|claude\|gpt` | Modell zur Laufzeit wechseln |
| `/save [titel]` | In das Wiki speichern |
| `/export pdf\|docx\|html\|md\|txt [datei]` | Puffer exportieren |
| `/open <datei>` / `/write [datei]` | Datei laden / Puffer in Datei schreiben |
| `/ghost on\|off` | Ghost-Text-Korrekturen an-/abschalten |
| `/<skill>` | Eigener Prompt-Skill (siehe unten) |
| `/help`, `/quit` | Hilfe / Beenden |

### Eigene Skills

Jede Markdown-Datei in `~/.config/vibe/skills/`, `./.vibe/skills/` oder
`$VIBE_SKILLS_DIR` wird zum Befehl `/<dateiname>`. Der Inhalt ist der
System-Prompt, der auf den Puffer angewendet wird – das Ergebnis erscheint
wie jede Transformation als Diff-Vorschau. Optionales Frontmatter liefert
die Beschreibung für `/help`:

```markdown
---
description: "Entschärft passiv-aggressive Formulierungen"
---
Rewrite the text so it stays factual and friendly. Keep the language,
remove blame and sarcasm, return only the rewritten text.
```

**Export:** Word (docx), HTML, Markdown und Text entstehen komplett offline
in-process. PDF delegiert an ein installiertes [pandoc](https://pandoc.org)
– ohne pandoc weicht man auf `/export html` aus und druckt im Browser als
PDF. Ohne Dateinamen wird `<erste-zeile-als-slug>-<timestamp>.<ext>` im
aktuellen Verzeichnis (bzw. `VIBE_EXPORT_DIR`) angelegt.

## Konfiguration (Umgebungsvariablen)

| Variable | Default | Zweck |
|---|---|---|
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Lokale Inferenz-Engine |
| `VIBE_GEMMA_MODEL` | `gemma3` | Lokales Skill-Modell |
| `VIBE_GHOST_MODEL` | = `VIBE_GEMMA_MODEL` | Kleineres/schnelleres Modell nur für Ghost-Text (z. B. `gemma3:1b`) |
| `VIBE_EMBED_MODEL` | `nomic-embed-text` | Embeddings für das Wiki-RAG (nomic-Task-Präfixe werden automatisch gesetzt) |
| `VIBE_WIKI_DIR` | `./vibe-wiki` | Ablageort der Wiki-Einträge |
| `VIBE_CACHE_DIR` | `./.vibe` | Embedding-Cache (`embeddings.json`) und Autosave |
| `VIBE_EXPORT_DIR` | `.` | Zielordner für `/export` ohne Dateiname |
| `VIBE_SKILLS_DIR` | – | Zusätzlicher Skill-Ordner |
| `ANTHROPIC_API_KEY` | – | aktiviert `/model claude` |
| `OPENAI_API_KEY` | – | aktiviert `/model gpt` |

## Architektur (3 Säulen)

```
┌────────────────────┐   WorkerMsg (mpsc)   ┌──────────────────────────────┐
│  UI-Thread (main)  │ ───────────────────▶ │  Tokio-Runtime (2 Threads)   │
│  ratatui, 60 FPS   │                      │  ├─ Debounce-Loop (450 ms)   │
│  niemals blockiert │ ◀─────────────────── │  ├─ Inferenz-Tasks (Ollama / │
└────────────────────┘   AppEvent (mpsc)    │  │   Anthropic / OpenAI)     │
                                            │  └─ I/O-Tasks (Wiki, Index)  │
                                            └──────────────────────────────┘
```

- **UI-Thread**: zeichnet jeden Frame neu, liest Eingaben mit 16 ms-Poll und
  konsumiert Worker-Ereignisse nicht-blockierend (`try_recv`).
- **Inferenz**: Tipp-Pausen werden im Worker debounced; Ghost-Korrekturen
  laufen **immer lokal** (Datenschutz, Kosten), unabhängig vom gewählten
  Hauptmodell.
- **Stabilität**: Transformationen ersetzen den Puffer nur bei Erfolg – ein
  abgebrochener Cloud-Call kann weder crashen noch Text verlieren.

Details und die Datenformat-Entscheidung (Markdown vs. Datenbank):
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Wiki-Format

Jeder Eintrag ist eine eigenständige Markdown-Datei mit YAML-Frontmatter:

```markdown
---
title: Projektidee Vibe-Editor
created: 2026-06-11T19:42:00+00:00
tags: [editor, rust, llm]
summary: CLI-Workbench mit lokalem Gemma und Cloud-Fallback.
model: Gemma · lokal (gemma3)
---

…der eigentliche Notizinhalt…
```

Der semantische Index (Embeddings) wird beim Start aus diesen Dateien
aufgebaut – die Dateien selbst bleiben die einzige Source of Truth.

## Roadmap

- [x] Diff-Vorschau + Undo/Redo für alle KI-Transformationen
- [x] Persistenter Embedding-Cache (`.vibe/embeddings.json`)
- [x] Eigene Prompt-Skills aus Markdown-Dateien
- [x] Datei öffnen/schreiben, Autosave + Crash-Recovery
- [x] Datenpfad-Badge (🔒/☁) und Cloud-Consent
- [ ] Inline-Ghost-Rendering direkt in der Zeile (statt Vorschlags-Panel)
- [ ] Streaming-Antworten für Cloud-Transformationen (Fortschritt sichtbar)
- [ ] Token-/Kostenzähler pro Sitzung in der Statuszeile
- [ ] Context Sniffer für Einfügen aus der Zwischenablage (Bracketed Paste)
- [ ] Hybrid-Retrieval (BM25 + Embeddings) und Re-Ranking für das Wiki
- [ ] PII-Prüfung vor Cloud-Calls (lokal, per Gemma)
