//! Nachrichten zwischen UI-Thread und den Hintergrund-Workern.
//!
//! Die UI darf niemals auf Inferenz oder I/O warten – sie schickt
//! `WorkerMsg` (fire-and-forget) und konsumiert `AppEvent` pro Frame
//! per `try_recv`.

use crate::commands::Command;

/// UI → Worker
#[derive(Debug)]
pub enum WorkerMsg {
    /// Der Puffer hat sich geändert (Debouncing passiert im Worker).
    TextChanged {
        text: String,
        line: usize,
        generation: u64,
    },
    /// Ein Befehl aus der Command-Bar, zusammen mit dem aktuellen Puffer.
    RunCommand { cmd: Command, text: String },
    /// Ghost-Text-Inferenz an-/abschalten.
    SetGhost(bool),
    Shutdown,
}

/// Worker → UI
#[derive(Debug)]
pub enum AppEvent {
    /// Statuszeile (persistente Meldung).
    Status(String),
    /// Ambiente Aktivitätsanzeige (animierter Spinner solange `Some`).
    Activity(Option<String>),
    /// Ghost-Text-Vorschlag für eine Zeile.
    Ghost {
        generation: u64,
        line: usize,
        original: String,
        corrected: String,
    },
    /// Ergebnis einer Transformation – die UI zeigt es als Diff-Vorschau
    /// und übernimmt es erst nach Bestätigung (Enter).
    Proposal { text: String, notice: String },
    /// Proaktive RAG-Treffer aus dem Wiki.
    WikiHints(Vec<WikiHint>),
    /// Eintrag gespeichert.
    Saved { path: String },
    /// Puffer exportiert (pdf/docx/html/md/txt).
    Exported { path: String },
    /// Datei geladen – ersetzt den Puffer (mit Undo-Snapshot).
    Opened { path: String, text: String },
    /// Puffer in Datei geschrieben.
    Written { path: String },
    /// Modellwechsel bestätigt; `cloud` steuert das 🔒/☁ -Badge.
    ModelSwitched { label: String, cloud: bool },
    /// Namen der geladenen Custom-Skills (für Autocomplete/Parser).
    SkillsLoaded(Vec<String>),
    /// Fehler – Puffer bleibt unangetastet (Stabilitäts-NFR).
    Error(String),
}

#[derive(Debug, Clone)]
pub struct WikiHint {
    pub title: String,
    pub summary: String,
    pub score: f32,
}
