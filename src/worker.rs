//! Hintergrund-Worker: Säule 2 (Inferenz) und Säule 3 (I/O) der
//! 3-Säulen-Architektur. Läuft auf einem Tokio-Multi-Thread-Runtime;
//! langlaufende Aufgaben (Cloud-Calls, Speichern, Indizieren) werden
//! als eigene Tasks gespawnt, damit der Debounce-Loop reaktiv bleibt.

use crate::commands::Command;
use crate::events::{AppEvent, WorkerMsg};
use crate::llm::{prompts, Llm};
use crate::wiki::{index::Index, Meta, WikiStore};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::RwLock;

const DEBOUNCE: Duration = Duration::from_millis(450);
/// Ab dieser Zeilenlänge lohnt sich eine Ghost-Korrektur.
const MIN_GHOST_CHARS: usize = 12;

pub async fn run(mut rx: UnboundedReceiver<WorkerMsg>, tx: UnboundedSender<AppEvent>) {
    let mut llm = Llm::from_env();
    let store = WikiStore::from_env();
    let index = Arc::new(RwLock::new(Index::default()));
    let local_ok = Arc::new(AtomicBool::new(false));

    // Startup: lokale Engine prüfen und Wiki indizieren (I/O-Task).
    {
        let llm = llm.clone();
        let tx = tx.clone();
        let index = index.clone();
        let store = store.clone();
        let local_ok = local_ok.clone();
        tokio::spawn(async move {
            let ok = llm.local_available().await;
            local_ok.store(ok, Ordering::Relaxed);
            if ok {
                let _ = tx.send(AppEvent::Status(format!("✓ {} bereit", llm.label())));
            } else {
                let _ = tx.send(AppEvent::Status(
                    "⚠ Ollama nicht erreichbar – Ghost-Text deaktiviert (/model claude für Cloud)"
                        .into(),
                ));
            }
            let entries = store.load_all();
            if entries.is_empty() {
                return;
            }
            let _ = tx.send(AppEvent::Activity(Some("Wiki-Agent: indiziert…".into())));
            let mut idx = Index::default();
            for entry in &entries {
                let emb = if ok {
                    llm.embed(&format!("{}\n{}", entry.meta.title, entry.body)).await
                } else {
                    None
                };
                idx.add(entry, emb);
            }
            let count = idx.docs.len();
            *index.write().await = idx;
            let _ = tx.send(AppEvent::Activity(None));
            let _ = tx.send(AppEvent::Status(format!("✓ Wiki indiziert: {count} Einträge")));
        });
    }

    let mut pending: Option<(String, usize, u64)> = None;
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                None | Some(WorkerMsg::Shutdown) => break,
                Some(WorkerMsg::TextChanged { text, line, generation }) => {
                    pending = Some((text, line, generation));
                }
                Some(WorkerMsg::RunCommand { cmd, text }) => {
                    handle_command(cmd, text, &mut llm, &store, &index, &tx);
                }
            },
            _ = tokio::time::sleep(DEBOUNCE), if pending.is_some() => {
                let (text, line, generation) = pending.take().unwrap();
                on_idle(text, line, generation, &llm, &index, &local_ok, &tx);
            }
        }
    }
}

/// Tipp-Pause erkannt: Ghost-Korrektur (lokal) + proaktives RAG.
fn on_idle(
    text: String,
    line: usize,
    generation: u64,
    llm: &Llm,
    index: &Arc<RwLock<Index>>,
    local_ok: &Arc<AtomicBool>,
    tx: &UnboundedSender<AppEvent>,
) {
    let local = local_ok.load(Ordering::Relaxed);

    // Ghost-Text: aktuelle Zeile lokal korrigieren.
    let current_line = text.lines().nth(line).unwrap_or("").to_string();
    if local && current_line.trim().chars().count() >= MIN_GHOST_CHARS {
        let llm = llm.clone();
        let tx = tx.clone();
        let original = current_line.clone();
        tokio::spawn(async move {
            let _ = tx.send(AppEvent::Activity(Some("Gemma: korrigiert…".into())));
            let result = llm.complete_local(prompts::CORRECT_LINE, &original).await;
            let _ = tx.send(AppEvent::Activity(None));
            if let Ok(corrected) = result {
                let corrected = corrected.lines().next().unwrap_or("").trim_end().to_string();
                if !corrected.is_empty() && corrected != original.trim_end() {
                    let _ = tx.send(AppEvent::Ghost { generation, line, original, corrected });
                }
            }
        });
    }

    // Wiki-Agent: thematische Überschneidungen mit alten Notizen suchen.
    if text.trim().chars().count() >= 40 {
        let llm = llm.clone();
        let tx = tx.clone();
        let index = index.clone();
        tokio::spawn(async move {
            let idx = index.read().await;
            if idx.docs.is_empty() {
                return;
            }
            let emb = if local { llm.embed(&text).await } else { None };
            let hints = idx.search(&text, emb.as_deref());
            let _ = tx.send(AppEvent::WikiHints(hints));
        });
    }
}

fn handle_command(
    cmd: Command,
    text: String,
    llm: &mut Llm,
    store: &WikiStore,
    index: &Arc<RwLock<Index>>,
    tx: &UnboundedSender<AppEvent>,
) {
    match cmd {
        Command::Model(name) => {
            let event = match llm.switch(&name) {
                Ok(label) => AppEvent::ModelSwitched(label),
                Err(e) => AppEvent::Error(e),
            };
            let _ = tx.send(event);
        }
        Command::Save(title) => {
            let llm = llm.clone();
            let store = store.clone();
            let index = index.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                save_entry(title, text, llm, store, index, tx).await;
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
        // Quit/Help werden in der UI behandelt und landen nie hier.
        Command::Quit | Command::Help => {}
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
async fn save_entry(
    title: Option<String>,
    text: String,
    llm: Llm,
    store: WikiStore,
    index: Arc<RwLock<Index>>,
    tx: UnboundedSender<AppEvent>,
) {
    if text.trim().is_empty() {
        let _ = tx.send(AppEvent::Error("Puffer ist leer – nichts zu speichern".into()));
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
            let entry = crate::wiki::Entry { path: path.clone(), meta, body: text.clone() };
            let emb = llm.embed(&text).await;
            index.write().await.add(&entry, emb);
            let _ = tx.send(AppEvent::Saved { path: path.display().to_string() });
        }
        Err(e) => {
            let _ = tx.send(AppEvent::Error(format!("Speichern fehlgeschlagen: {e}")));
        }
    }
}
