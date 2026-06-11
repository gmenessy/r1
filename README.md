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
| `Ctrl+Q` | Beenden |

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
| `/help`, `/quit` | Hilfe / Beenden |

## Konfiguration (Umgebungsvariablen)

| Variable | Default | Zweck |
|---|---|---|
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Lokale Inferenz-Engine |
| `VIBE_GEMMA_MODEL` | `gemma3` | Lokales Korrektur-/Skill-Modell |
| `VIBE_EMBED_MODEL` | `nomic-embed-text` | Embeddings für das Wiki-RAG |
| `VIBE_WIKI_DIR` | `./vibe-wiki` | Ablageort der Wiki-Einträge |
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
- [ ] Inline-Ghost-Rendering direkt in der Zeile (statt Vorschlags-Panel)
- [ ] Context Sniffer für Einfügen aus der Zwischenablage (Bracketed Paste)
- [ ] Persistenter Embedding-Cache (`.vibe/index.json`) statt Rebuild beim Start
- [ ] Streaming-Antworten für Cloud-Transformationen
- [ ] Plugin-Schnittstelle für eigene Skills
