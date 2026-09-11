//! Hintergrund-Worker: Säule 2 (Inferenz) und Säule 3 (I/O) der
//! 3-Säulen-Architektur. Läuft auf einem Tokio-Multi-Thread-Runtime;
//! langlaufende Aufgaben (Cloud-Calls, Speichern, Indizieren) werden
//! als eigene Tasks gespawnt, damit der Debounce-Loop reaktiv bleibt.

use crate::commands::Command;
use crate::events::{AppEvent, WorkerMsg};
use crate::llm::{prompts, Llm};
use crate::skills::{load_skills, Skill};
use crate::wiki::cache::{content_key, EmbeddingCache};
use crate::wiki::{index::Index, Entry, Meta, WikiStore};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

const DEBOUNCE: Duration = Duration::from_millis(450);
/// Ab dieser Zeilenlänge lohnt sich eine Ghost-Korrektur.
const MIN_GHOST_CHARS: usize = 12;
/// Zeilen ober-/unterhalb des Cursors, die als RAG-Query dienen.
const RAG_WINDOW: usize = 6;

/// Crash-Recovery: Der Puffer wird in jeder Tipp-Pause hierhin gesichert.
pub fn autosave_path() -> PathBuf {
    PathBuf::from(std::env::var("VIBE_CACHE_DIR").unwrap_or_else(|_| ".vibe".into()))
        .join("autosave.md")
}

/// Geteilter Zustand für gespawnte Tasks.
#[derive(Clone)]
struct Shared {
    tx: UnboundedSender<AppEvent>,
    index: Arc<RwLock<Index>>,
    cache: Arc<Mutex<EmbeddingCache>>,
    skills: Arc<RwLock<BTreeMap<String, Skill>>>,
    local_ok: Arc<AtomicBool>,
}

pub async fn run(mut rx: UnboundedReceiver<WorkerMsg>, tx: UnboundedSender<AppEvent>) {
    let mut llm = Llm::from_env();
    let store = WikiStore::from_env();
    let shared = Shared {
        tx,
        index: Arc::new(RwLock::new(Index::default())),
        cache: Arc::new(Mutex::new(EmbeddingCache::load(EmbeddingCache::default_path()))),
        skills: Arc::new(RwLock::new(BTreeMap::new())),
        local_ok: Arc::new(AtomicBool::new(false)),
    };

    tokio::spawn(startup(llm.clone(), store.clone(), shared.clone()));

    let mut ghost_enabled = true;
    let mut ghost_task: Option<JoinHandle<()>> = None;
    let mut pending: Option<(String, usize, u64)> = None;
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                None | Some(WorkerMsg::Shutdown) => break,
                Some(WorkerMsg::TextChanged { text, line, generation }) => {
                    pending = Some((text, line, generation));
                }
                Some(WorkerMsg::SetGhost(on)) => {
                    ghost_enabled = on;
                    if !on {
                        cancel_ghost(&mut ghost_task, &shared.tx);
                    }
                }
                Some(WorkerMsg::RunCommand { cmd, text }) => {
                    handle_command(cmd, text, &mut llm, &store, &shared);
                }
            },
            _ = tokio::time::sleep(DEBOUNCE), if pending.is_some() => {
                let (text, line, generation) = pending.take().unwrap();
                on_idle(text, line, generation, ghost_enabled, &mut ghost_task, &llm, &shared);
            }
        }
    }
    shared.cache.lock().unwrap().flush();
}

/// Startup (I/O-Task): lokale Engine prüfen, Skills laden, Wiki indizieren.
async fn startup(llm: Llm, store: WikiStore, shared: Shared) {
    let ok = llm.local_available().await;
    shared.local_ok.store(ok, Ordering::Relaxed);
    let _ = shared.tx.send(AppEvent::Status(if ok {
        format!("✓ {} bereit", llm.label())
    } else {
        "⚠ Ollama nicht erreichbar – Ghost-Text deaktiviert (/model claude für Cloud)".into()
    }));

    let skills = load_skills();
    let names: Vec<String> = skills.keys().cloned().collect();
    if !skills.is_empty() {
        let listing: Vec<String> = skills
            .values()
            .map(|s| {
                if s.description.is_empty() {
                    format!("/{}", s.name)
                } else {
                    format!("/{} ({})", s.name, s.description)
                }
            })
            .collect();
        let _ = shared.tx.send(AppEvent::Status(format!("✓ Skills: {}", listing.join(" · "))));
    }
    *shared.skills.write().await = skills;
    let _ = shared.tx.send(AppEvent::SkillsLoaded(names));

    let entries = store.load_all();
    if entries.is_empty() {
        return;
    }
    let _ = shared.tx.send(AppEvent::Activity(Some("Wiki-Agent: indiziert…".into())));
    let mut idx = Index::default();
    let mut cache_hits = 0usize;
    for entry in &entries {
        let emb = embed_entry(&llm, entry, &shared.cache, ok, &mut cache_hits).await;
        idx.add(entry, emb);
    }
    let count = idx.docs.len();
    *shared.index.write().await = idx;
    shared.cache.lock().unwrap().flush();
    let _ = shared.tx.send(AppEvent::Activity(None));
    let _ = shared.tx.send(AppEvent::Status(format!(
        "✓ Wiki indiziert: {count} Einträge ({cache_hits} aus Cache)"
    )));
}

/// Embedding aus dem Cache oder frisch berechnet (und dann gecacht).
async fn embed_entry(
    llm: &Llm,
    entry: &Entry,
    cache: &Arc<Mutex<EmbeddingCache>>,
    local: bool,
    hits: &mut usize,
) -> Option<Vec<f32>> {
    let text = format!("{}\n{}", entry.meta.title, entry.body);
    let key = content_key(llm.embed_model(), &text);
    if let Some(cached) = cache.lock().unwrap().get(key).cloned() {
        *hits += 1;
        return Some(cached);
    }
    if !local {
        return None;
    }
    let emb = llm.embed_document(&text).await?;
    cache.lock().unwrap().insert(key, emb.clone());
    Some(emb)
}

fn cancel_ghost(task: &mut Option<JoinHandle<()>>, tx: &UnboundedSender<AppEvent>) {
    if let Some(handle) = task.take() {
        if !handle.is_finished() {
            handle.abort();
            let _ = tx.send(AppEvent::Activity(None));
        }
    }
}

/// Tipp-Pause erkannt: Autosave, Ghost-Korrektur (lokal) + proaktives RAG.
fn on_idle(
    text: String,
    line: usize,
    generation: u64,
    ghost_enabled: bool,
    ghost_task: &mut Option<JoinHandle<()>>,
    llm: &Llm,
    shared: &Shared,
) {
    let local = shared.local_ok.load(Ordering::Relaxed);

    // Crash-Recovery: Puffer sichern (I/O-Task, Fehler unkritisch).
    {
        let snapshot = text.clone();
        tokio::spawn(async move {
            let path = autosave_path();
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let _ = tokio::fs::write(path, snapshot).await;
        });
    }

    // Ghost-Text: aktuelle Zeile lokal korrigieren; eine noch laufende
    // Korrektur ist durch die neue Eingabe veraltet und wird abgebrochen.
    cancel_ghost(ghost_task, &shared.tx);
    let lines: Vec<&str> = text.lines().collect();
    let current_line = lines.get(line).copied().unwrap_or("").to_string();
    if ghost_enabled && local && current_line.trim().chars().count() >= MIN_GHOST_CHARS {
        let llm = llm.clone();
        let tx = shared.tx.clone();
        let original = current_line.clone();
        *ghost_task = Some(tokio::spawn(async move {
            let _ = tx.send(AppEvent::Activity(Some("Gemma: korrigiert…".into())));
            let result = llm.complete_local(prompts::CORRECT_LINE, &original).await;
            let _ = tx.send(AppEvent::Activity(None));
            if let Ok(corrected) = result {
                let corrected = corrected.lines().next().unwrap_or("").trim_end().to_string();
                if !corrected.is_empty() && corrected != original.trim_end() {
                    let _ = tx.send(AppEvent::Ghost { generation, line, original, corrected });
                }
            }
        }));
    }

    // Wiki-Agent: Query ist das Fenster um den Cursor, nicht der Gesamttext –
    // sonst verwässert ein langes Dokument das aktuelle Thema.
    let start = line.saturating_sub(RAG_WINDOW);
    let end = (line + RAG_WINDOW + 1).min(lines.len());
    let window = lines.get(start..end).unwrap_or(&[]).join("\n");
    if window.trim().chars().count() >= 40 {
        let llm = llm.clone();
        let tx = shared.tx.clone();
        let index = shared.index.clone();
        tokio::spawn(async move {
            let idx = index.read().await;
            if idx.docs.is_empty() {
                return;
            }
            let emb = if local { llm.embed_query(&window).await } else { None };
            let hints = idx.search(&window, emb.as_deref());
            let _ = tx.send(AppEvent::WikiHints(hints));
        });
    }
}

fn handle_command(cmd: Command, text: String, llm: &mut Llm, store: &WikiStore, shared: &Shared) {
    let tx = &shared.tx;
    match cmd {
        Command::Model(name) => {
            let event = match llm.switch(&name) {
                Ok(label) => AppEvent::ModelSwitched { label, cloud: llm.is_cloud() },
                Err(e) => AppEvent::Error(e),
            };
            let _ = tx.send(event);
        }
        Command::Save(title) => {
            let llm = llm.clone();
            let store = store.clone();
            let shared = shared.clone();
            tokio::spawn(async move {
                save_entry(title, text, llm, store, shared).await;
            });
        }
        Command::Lang(lang) => {
            transform(llm, tx, text, prompts::translate(&lang), format!("Übersetzt nach '{lang}'"));
        }
        Command::Style(style) => {
            transform(llm, tx, text, prompts::restyle(&style), format!("Stil: '{style}'"));
        }
        Command::Summarize => {
            transform(llm, tx, text, prompts::SUMMARIZE.into(), "Zusammengefasst".into());
        }
        Command::Todo => {
            transform(llm, tx, text, prompts::TODO.into(), "Action-Items extrahiert".into());
        }
        Command::Expand => {
            transform(llm, tx, text, prompts::EXPAND.into(), "Notizen ausformuliert".into());
        }
        Command::Skill(name) => {
            let llm = llm.clone();
            let shared = shared.clone();
            tokio::spawn(async move {
                let prompt = shared.skills.read().await.get(&name).map(|s| s.prompt.clone());
                match prompt {
                    Some(prompt) => transform(&llm, &shared.tx, text, prompt, format!("Skill '{name}'")),
                    None => {
                        let _ = shared.tx.send(AppEvent::Error(format!("Skill '{name}' nicht gefunden")));
                    }
                }
            });
        }
        Command::Export { format, target } => {
            let tx = tx.clone();
            tokio::spawn(async move {
                match crate::export::export(format, &text, target.as_deref()).await {
                    Ok(path) => {
                        let _ = tx.send(AppEvent::Exported { path: path.display().to_string() });
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(format!("Export fehlgeschlagen: {e}")));
                    }
                }
            });
        }
        Command::Open(path) => {
            let tx = tx.clone();
            tokio::spawn(async move {
                match tokio::fs::read_to_string(&path).await {
                    Ok(content) => {
                        let _ = tx.send(AppEvent::Opened { path, text: content });
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(format!("{path}: {e}")));
                    }
                }
            });
        }
        Command::Write(Some(path)) => {
            let tx = tx.clone();
            tokio::spawn(async move {
                match tokio::fs::write(&path, &text).await {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::Written { path });
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(format!("{path}: {e}")));
                    }
                }
            });
        }
        Command::Write(None) => {
            let _ = tx.send(AppEvent::Error("Keine Datei geöffnet – /write <datei>".into()));
        }
        // In der UI behandelt – landen nie hier.
        Command::Ghost(_) | Command::Quit | Command::Help => {}
    }
}

/// Puffer-Transformation: Das Ergebnis geht als Vorschlag (Diff-Vorschau)
/// an die UI – ersetzt wird erst nach Bestätigung, und ein abgebrochener
/// Cloud-Call darf den Puffer niemals verlieren (Stabilitäts-NFR).
fn transform(
    llm: &Llm,
    tx: &UnboundedSender<AppEvent>,
    text: String,
    system: String,
    notice: String,
) {
    if text.trim().is_empty() {
        let _ = tx.send(AppEvent::Error("Puffer ist leer".into()));
        return;
    }
    // Modell im Hinweis festhalten: Ein späterer /model-Wechsel ändert
    // nichts mehr an einem bereits laufenden Task.
    let notice = format!("{notice} · {}", llm.short_label());
    let llm = llm.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        match llm.complete(&system, &text).await {
            Ok(result) => {
                let _ = tx.send(AppEvent::Proposal { text: result, notice });
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(e.to_string()));
            }
        }
    });
}

/// Hoch-Aggregation beim Speichern: Titel, Summary und Tags kommen vom
/// LLM (best effort – ohne Modell wird trotzdem gespeichert).
async fn save_entry(title: Option<String>, text: String, llm: Llm, store: WikiStore, shared: Shared) {
    if text.trim().is_empty() {
        let _ = shared.tx.send(AppEvent::Error("Puffer ist leer – nichts zu speichern".into()));
        return;
    }
    let mut meta = Meta {
        title: title.unwrap_or_default(),
        created: chrono::Local::now().to_rfc3339(),
        model: llm.label(),
        ..Meta::default()
    };

    if let Ok(raw) = llm.complete(prompts::WIKI_META, &text).await {
        let cleaned = raw.trim().trim_start_matches("```json").trim_matches('`').trim();
        if let Ok(v) = serde_json::from_str::<Value>(cleaned) {
            if meta.title.is_empty() {
                meta.title = v["title"].as_str().unwrap_or_default().to_string();
            }
            meta.summary = v["summary"].as_str().unwrap_or_default().to_string();
            meta.tags = v["tags"]
                .as_array()
                .map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect())
                .unwrap_or_default();
        }
    }
    if meta.title.is_empty() {
        meta.title = text.lines().next().unwrap_or("Notiz").chars().take(60).collect();
    }

    match store.save(&meta, &text) {
        Ok(path) => {
            let entry = Entry { path: path.clone(), meta, body: text.clone() };
            let local = shared.local_ok.load(Ordering::Relaxed);
            let mut hits = 0;
            let emb = embed_entry(&llm, &entry, &shared.cache, local, &mut hits).await;
            shared.index.write().await.add(&entry, emb);
            shared.cache.lock().unwrap().flush();
            let _ = shared.tx.send(AppEvent::Saved { path: path.display().to_string() });
        }
        Err(e) => {
            let _ = shared.tx.send(AppEvent::Error(format!("Speichern fehlgeschlagen: {e}")));
        }
    }
}
