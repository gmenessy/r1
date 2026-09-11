//! UI-seitiger Zustand und Tastatur-Logik. Läuft komplett auf dem
//! UI-Thread – keine blockierenden Aufrufe.

use crate::buffer::Buffer;
use crate::commands::{self, Command};
use crate::diff::{self, Op};
use crate::events::{AppEvent, WikiHint, WorkerMsg};
use crate::llm::is_cloud_name;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[derive(PartialEq, Eq)]
pub enum Mode {
    Edit,
    Command,
    /// Diff-Vorschau einer Transformation – modal, bis Enter/Esc.
    Review,
}

pub struct Ghost {
    pub line: usize,
    pub original: String,
    pub corrected: String,
}

/// Vorgeschlagene Transformation samt vorberechnetem Wort-Diff.
pub struct Review {
    pub text: String,
    pub notice: String,
    pub diff: Vec<(Op, String)>,
    pub scroll: u16,
}

struct Snapshot {
    lines: Vec<String>,
    row: usize,
    col: usize,
}

/// Tipp-Bursts innerhalb dieses Fensters teilen sich einen Snapshot.
const SNAPSHOT_COALESCE: Duration = Duration::from_millis(800);
const MAX_UNDO: usize = 200;
const TAB_WIDTH: usize = 4;

pub struct App {
    pub buffer: Buffer,
    pub mode: Mode,
    pub cmdline: String,
    pub completions: Vec<String>,
    pub ghost: Option<Ghost>,
    pub ghost_enabled: bool,
    pub review: Option<Review>,
    pub hints: Vec<WikiHint>,
    pub status: String,
    pub activity: Option<String>,
    pub model: String,
    /// Aktives Modell sendet Puffertexte an einen Cloud-Anbieter.
    pub cloud: bool,
    pub file_path: Option<PathBuf>,
    pub skills: Vec<String>,
    pub spinner: usize,
    pub generation: u64,
    cloud_consented: bool,
    cloud_consent_pending: Option<String>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    last_snapshot: Option<Instant>,
    to_worker: UnboundedSender<WorkerMsg>,
}

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

impl App {
    pub fn new(to_worker: UnboundedSender<WorkerMsg>) -> Self {
        Self {
            buffer: Buffer::new(),
            mode: Mode::Edit,
            cmdline: String::new(),
            completions: Vec::new(),
            ghost: None,
            ghost_enabled: true,
            review: None,
            hints: Vec::new(),
            status: String::new(),
            activity: None,
            model: "Gemma · lokal".into(),
            cloud: false,
            file_path: None,
            skills: Vec::new(),
            spinner: 0,
            generation: 0,
            cloud_consented: false,
            cloud_consent_pending: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_snapshot: None,
            to_worker,
        }
    }

    pub fn spinner_char(&self) -> char {
        SPINNER[self.spinner % SPINNER.len()]
    }

    pub fn tick(&mut self) {
        if self.activity.is_some() {
            self.spinner = self.spinner.wrapping_add(1);
        }
    }

    pub fn file_label(&self) -> String {
        self.file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "(neu)".into())
    }

    /// Datei beim Start laden (CLI-Argument) – ohne Undo-Eintrag.
    pub fn load_initial(&mut self, path: PathBuf, text: &str) {
        self.buffer.set_text(text.trim_end_matches('\n'));
        self.buffer.row = 0;
        self.buffer.col = 0;
        self.file_path = Some(path);
        self.status = format!("✓ Geöffnet: {}", self.file_label());
    }

    /// `true` ⇒ Editor beenden.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match self.mode {
            Mode::Edit => self.handle_edit_key(key),
            Mode::Command => self.handle_command_key(key),
            Mode::Review => self.handle_review_key(key),
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match (key.code, ctrl) {
            (KeyCode::Char('q'), true) => return true,
            (KeyCode::Char('p'), true) => self.open_command_bar(),
            (KeyCode::Char('s'), true) => self.run_command(Command::Save(None), "Speichert…"),
            (KeyCode::Char('z'), true) => self.undo(),
            (KeyCode::Char('y'), true) => self.redo(),
            (KeyCode::Tab, _) => {
                if self.ghost.is_some() {
                    self.accept_ghost();
                } else {
                    self.maybe_snapshot();
                    for _ in 0..TAB_WIDTH {
                        self.buffer.insert_char(' ');
                    }
                    self.on_edit();
                }
            }
            (KeyCode::Esc, _) => self.ghost = None,
            (KeyCode::Char('/'), false) if self.buffer.current_line().is_empty() => {
                self.open_command_bar()
            }
            (KeyCode::Char(c), false) => {
                self.maybe_snapshot();
                self.buffer.insert_char(c);
                self.on_edit();
            }
            (KeyCode::Enter, _) => {
                self.maybe_snapshot();
                self.buffer.newline();
                self.on_edit();
            }
            (KeyCode::Backspace, _) => {
                self.maybe_snapshot();
                self.buffer.backspace();
                self.on_edit();
            }
            (KeyCode::Delete, _) => {
                self.maybe_snapshot();
                self.buffer.delete_forward();
                self.on_edit();
            }
            (KeyCode::Left, _) => self.buffer.move_left(),
            (KeyCode::Right, _) => self.buffer.move_right(),
            (KeyCode::Up, _) => self.buffer.move_up(),
            (KeyCode::Down, _) => self.buffer.move_down(),
            (KeyCode::Home, _) => self.buffer.home(),
            (KeyCode::End, _) => self.buffer.end(),
            _ => {}
        }
        false
    }

    fn handle_command_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Edit;
                self.cmdline.clear();
            }
            KeyCode::Enter => {
                let input = std::mem::take(&mut self.cmdline);
                self.mode = Mode::Edit;
                return self.execute(&input);
            }
            KeyCode::Tab => {
                if let Some(full) = commands::complete_first(&self.cmdline, &self.skills) {
                    self.cmdline = full;
                    self.cmdline.push(' ');
                }
                self.refresh_completions();
            }
            KeyCode::Backspace => {
                if self.cmdline.pop().is_none() {
                    self.mode = Mode::Edit;
                }
                self.refresh_completions();
            }
            KeyCode::Char(c) => {
                self.cmdline.push(c);
                self.refresh_completions();
            }
            _ => {}
        }
        false
    }

    /// Befehl ausführen; `true` ⇒ Editor beenden.
    fn execute(&mut self, input: &str) -> bool {
        match commands::parse(input, &self.skills) {
            Ok(Command::Quit) => return true,
            Ok(Command::Help) => self.status = commands::help_text(&self.skills),
            Ok(Command::Ghost(on)) => {
                self.ghost_enabled = on;
                if !on {
                    self.ghost = None;
                }
                let _ = self.to_worker.send(WorkerMsg::SetGhost(on));
                self.status = if on { "✓ Ghost-Text an" } else { "Ghost-Text aus" }.into();
            }
            Ok(Command::Write(None)) => match self.file_path.clone() {
                Some(path) => self.run_command(
                    Command::Write(Some(path.display().to_string())),
                    "Schreibt…",
                ),
                None => self.status = "✖ Keine Datei geöffnet – /write <datei>".into(),
            },
            Ok(Command::Model(name)) => self.switch_model(name),
            Ok(cmd) => {
                let label = activity_label(&cmd);
                self.run_command(cmd, label);
            }
            Err(e) => self.status = format!("✖ {e}"),
        }
        false
    }

    /// Cloud-Consent: Der erste Wechsel auf ein Cloud-Modell pro Sitzung
    /// muss durch Wiederholen des Befehls bestätigt werden – vorher geht
    /// kein einziges Zeichen des Puffers nach draußen.
    fn switch_model(&mut self, name: String) {
        if is_cloud_name(&name) && !self.cloud_consented {
            if self.cloud_consent_pending.as_deref() != Some(name.as_str()) {
                self.cloud_consent_pending = Some(name.clone());
                self.status = format!(
                    "☁ Cloud-Modell: Puffertexte werden an den Anbieter gesendet. \
Zum Bestätigen erneut /model {name}"
                );
                return;
            }
            self.cloud_consented = true;
        }
        self.cloud_consent_pending = None;
        self.run_command(Command::Model(name), "Wechselt Modell…");
    }

    fn handle_review_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => {
                if let Some(review) = self.review.take() {
                    self.push_snapshot();
                    self.redo_stack.clear();
                    self.buffer.set_text(&review.text);
                    self.generation = self.generation.wrapping_add(1);
                    self.ghost = None;
                    self.status = format!("✓ {} übernommen · Ctrl+Z widerruft", review.notice);
                    self.notify_text_changed();
                }
                self.mode = Mode::Edit;
            }
            KeyCode::Esc => {
                self.review = None;
                self.mode = Mode::Edit;
                self.status = "Vorschlag verworfen – Puffer unverändert".into();
            }
            KeyCode::Up => self.review_scroll(-1),
            KeyCode::Down => self.review_scroll(1),
            KeyCode::PageUp => self.review_scroll(-10),
            KeyCode::PageDown => self.review_scroll(10),
            _ => {}
        }
        false
    }

    fn review_scroll(&mut self, delta: i32) {
        if let Some(review) = &mut self.review {
            review.scroll = review.scroll.saturating_add_signed(delta as i16);
        }
    }

    fn open_command_bar(&mut self) {
        self.mode = Mode::Command;
        self.cmdline.clear();
        self.refresh_completions();
    }

    fn refresh_completions(&mut self) {
        self.completions = commands::completions(&self.cmdline, &self.skills);
    }

    fn run_command(&mut self, cmd: Command, activity: &str) {
        self.activity = Some(activity.to_string());
        let _ = self.to_worker.send(WorkerMsg::RunCommand {
            cmd,
            text: self.buffer.text(),
        });
    }

    fn notify_text_changed(&self) {
        let _ = self.to_worker.send(WorkerMsg::TextChanged {
            text: self.buffer.text(),
            line: self.buffer.row,
            generation: self.generation,
        });
    }

    /// Nach jeder Pufferänderung: Ghost invalidieren, Worker informieren.
    fn on_edit(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.ghost = None;
        self.notify_text_changed();
    }

    // --- Undo / Redo -----------------------------------------------------

    fn current_snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.buffer.lines.clone(),
            row: self.buffer.row,
            col: self.buffer.col,
        }
    }

    /// Unbedingter Snapshot – vor jeder KI-Änderung (Ghost, Transformation).
    fn push_snapshot(&mut self) {
        self.undo_stack.push(self.current_snapshot());
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.last_snapshot = Some(Instant::now());
    }

    /// Snapshot vor Tipp-Eingaben, zeitlich koalesziert: ein schneller
    /// Burst teilt sich einen Eintrag, Ctrl+Z springt an den Burst-Anfang.
    fn maybe_snapshot(&mut self) {
        let stale = self
            .last_snapshot
            .map(|t| t.elapsed() > SNAPSHOT_COALESCE)
            .unwrap_or(true);
        if self.undo_stack.is_empty() || stale {
            self.push_snapshot();
        }
        self.redo_stack.clear();
    }

    fn restore(&mut self, snap: Snapshot) {
        self.buffer.lines = snap.lines;
        self.buffer.row = snap.row;
        self.buffer.col = snap.col;
        self.generation = self.generation.wrapping_add(1);
        self.ghost = None;
        // Nächste Eingabe beginnt einen frischen Undo-Eintrag.
        self.last_snapshot = None;
        self.notify_text_changed();
    }

    fn undo(&mut self) {
        match self.undo_stack.pop() {
            Some(snap) => {
                self.redo_stack.push(self.current_snapshot());
                self.restore(snap);
                self.status = "↶ Rückgängig".into();
            }
            None => self.status = "Nichts rückgängig zu machen".into(),
        }
    }

    fn redo(&mut self) {
        match self.redo_stack.pop() {
            Some(snap) => {
                self.undo_stack.push(self.current_snapshot());
                self.restore(snap);
                self.status = "↷ Wiederholt".into();
            }
            None => self.status = "Nichts zu wiederholen".into(),
        }
    }

    // ----------------------------------------------------------------------

    fn accept_ghost(&mut self) {
        if let Some(ghost) = self.ghost.take() {
            // Nur übernehmen, wenn die Zeile inzwischen nicht verändert wurde.
            if self
                .buffer
                .lines
                .get(ghost.line)
                .map(|l| l == &ghost.original)
                .unwrap_or(false)
            {
                self.push_snapshot();
                self.redo_stack.clear();
                self.buffer.replace_line(ghost.line, &ghost.corrected);
                if self.buffer.row == ghost.line {
                    self.buffer.end();
                }
                self.status = "✓ Ghost-Korrektur übernommen · Ctrl+Z widerruft".into();
                self.on_edit();
            }
        }
    }

    pub fn apply_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Status(s) => self.status = s,
            AppEvent::Activity(a) => self.activity = a,
            AppEvent::Ghost { generation, line, original, corrected } => {
                // Vorschlag nur anzeigen, wenn der Puffer seitdem unverändert
                // ist und die Zeile noch dem Original entspricht.
                let line_unchanged = self
                    .buffer
                    .lines
                    .get(line)
                    .map(|l| l == &original)
                    .unwrap_or(false);
                if self.ghost_enabled && generation == self.generation && line_unchanged {
                    self.ghost = Some(Ghost { line, original, corrected });
                }
            }
            AppEvent::Proposal { text, notice } => {
                self.activity = None;
                if text.trim() == self.buffer.text().trim() {
                    self.status = format!("✓ {notice} – keine Änderungen");
                } else {
                    let diff = diff::word_diff(&self.buffer.text(), &text);
                    self.review = Some(Review { text, notice, diff, scroll: 0 });
                    self.mode = Mode::Review;
                    self.status =
                        "Vorschau: Enter übernehmen · Esc verwerfen · ↑↓ scrollen".into();
                }
            }
            AppEvent::WikiHints(hints) => self.hints = hints,
            AppEvent::Saved { path } => {
                self.activity = None;
                self.status = format!("✓ Gespeichert: {path}");
            }
            AppEvent::Exported { path } => {
                self.activity = None;
                self.status = format!("✓ Exportiert: {path}");
            }
            AppEvent::Opened { path, text } => {
                self.activity = None;
                self.push_snapshot();
                self.redo_stack.clear();
                self.buffer.set_text(text.trim_end_matches('\n'));
                self.buffer.row = 0;
                self.buffer.col = 0;
                self.generation = self.generation.wrapping_add(1);
                self.ghost = None;
                self.file_path = Some(PathBuf::from(&path));
                self.status = format!("✓ Geöffnet: {path} · Ctrl+Z stellt den alten Puffer wieder her");
                self.notify_text_changed();
            }
            AppEvent::Written { path } => {
                self.activity = None;
                self.file_path = Some(PathBuf::from(&path));
                self.status = format!("✓ Geschrieben: {path}");
            }
            AppEvent::ModelSwitched { label, cloud } => {
                self.activity = None;
                self.cloud = cloud;
                self.model = label.clone();
                self.status = format!("✓ Modell: {label}");
            }
            AppEvent::SkillsLoaded(names) => self.skills = names,
            AppEvent::Error(e) => {
                self.activity = None;
                self.status = format!("✖ {e}");
            }
        }
    }
}

fn activity_label(cmd: &Command) -> &'static str {
    match cmd {
        Command::Lang(_) => "Übersetzt…",
        Command::Style(_) => "Formt um…",
        Command::Model(_) => "Wechselt Modell…",
        Command::Summarize => "Fasst zusammen…",
        Command::Todo => "Extrahiert Aufgaben…",
        Command::Expand => "Formuliert aus…",
        Command::Skill(_) => "Skill läuft…",
        Command::Save(_) => "Speichert…",
        Command::Export { .. } => "Exportiert…",
        Command::Open(_) => "Lädt…",
        Command::Write(_) => "Schreibt…",
        Command::Ghost(_) | Command::Help | Command::Quit => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::UnboundedReceiver;

    fn app() -> (App, UnboundedReceiver<WorkerMsg>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (App::new(tx), rx)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn ctrl(app: &mut App, c: char) {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    fn type_command(app: &mut App, cmd: &str) {
        ctrl(app, 'p');
        for c in cmd.chars() {
            press(app, KeyCode::Char(c));
        }
        press(app, KeyCode::Enter);
    }

    #[test]
    fn typing_burst_undoes_as_one_step() {
        let (mut a, _rx) = app();
        for c in "abc".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        assert_eq!(a.buffer.text(), "abc");
        ctrl(&mut a, 'z');
        assert_eq!(a.buffer.text(), "");
    }

    #[test]
    fn redo_restores_undone_text() {
        let (mut a, _rx) = app();
        press(&mut a, KeyCode::Char('x'));
        ctrl(&mut a, 'z');
        assert_eq!(a.buffer.text(), "");
        ctrl(&mut a, 'y');
        assert_eq!(a.buffer.text(), "x");
    }

    #[test]
    fn proposal_needs_confirmation_and_is_undoable() {
        let (mut a, _rx) = app();
        for c in "alt".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        a.apply_event(AppEvent::Proposal { text: "neu".into(), notice: "Test".into() });
        assert!(a.mode == Mode::Review);
        assert_eq!(a.buffer.text(), "alt", "Puffer bleibt bis Enter unangetastet");

        press(&mut a, KeyCode::Enter);
        assert_eq!(a.buffer.text(), "neu");

        ctrl(&mut a, 'z');
        assert_eq!(a.buffer.text(), "alt", "Ctrl+Z widerruft die Transformation");
    }

    #[test]
    fn rejected_proposal_leaves_buffer_untouched() {
        let (mut a, _rx) = app();
        press(&mut a, KeyCode::Char('a'));
        a.apply_event(AppEvent::Proposal { text: "neu".into(), notice: "Test".into() });
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.buffer.text(), "a");
        assert!(a.mode == Mode::Edit);
        assert!(a.review.is_none());
    }

    #[test]
    fn identical_proposal_skips_review() {
        let (mut a, _rx) = app();
        press(&mut a, KeyCode::Char('a'));
        a.apply_event(AppEvent::Proposal { text: "a".into(), notice: "Test".into() });
        assert!(a.review.is_none());
        assert!(a.mode == Mode::Edit);
    }

    #[test]
    fn tab_inserts_spaces_without_ghost_and_delete_removes_forward() {
        let (mut a, _rx) = app();
        press(&mut a, KeyCode::Tab);
        assert_eq!(a.buffer.text(), "    ");
        press(&mut a, KeyCode::Home);
        press(&mut a, KeyCode::Delete);
        assert_eq!(a.buffer.text(), "   ");
    }

    #[test]
    fn cloud_model_requires_confirmation_before_worker_is_asked() {
        let (mut a, mut rx) = app();
        type_command(&mut a, "model claude");
        assert!(a.status.starts_with("☁"), "erste Anfrage nur warnen: {}", a.status);
        assert!(
            !matches!(rx.try_recv(), Ok(WorkerMsg::RunCommand { .. })),
            "kein RunCommand vor Consent"
        );

        type_command(&mut a, "model claude");
        assert!(matches!(
            rx.try_recv(),
            Ok(WorkerMsg::RunCommand { cmd: Command::Model(m), .. }) if m == "claude"
        ));

        // Lokal braucht nie Consent; erneutes Cloud in derselben Sitzung auch nicht.
        type_command(&mut a, "model gemma");
        assert!(matches!(rx.try_recv(), Ok(WorkerMsg::RunCommand { cmd: Command::Model(_), .. })));
        type_command(&mut a, "model gpt");
        assert!(matches!(rx.try_recv(), Ok(WorkerMsg::RunCommand { cmd: Command::Model(m), .. }) if m == "gpt"));
    }

    #[test]
    fn ghost_toggle_reaches_worker_and_suppresses_suggestions() {
        let (mut a, mut rx) = app();
        type_command(&mut a, "ghost off");
        assert!(matches!(rx.try_recv(), Ok(WorkerMsg::SetGhost(false))));
        press(&mut a, KeyCode::Char('x'));
        a.apply_event(AppEvent::Ghost {
            generation: a.generation,
            line: 0,
            original: "x".into(),
            corrected: "X".into(),
        });
        assert!(a.ghost.is_none());
    }

    #[test]
    fn write_without_open_file_is_an_error_and_open_sets_path() {
        let (mut a, mut rx) = app();
        type_command(&mut a, "write");
        assert!(a.status.contains("Keine Datei"));
        assert!(rx.try_recv().is_err());

        a.apply_event(AppEvent::Opened { path: "notes.md".into(), text: "inhalt\n".into() });
        assert_eq!(a.buffer.text(), "inhalt");
        assert_eq!(a.file_label(), "notes.md");

        type_command(&mut a, "write");
        // Opened löst zuerst ein TextChanged aus – das RunCommand folgt danach.
        let mut found = false;
        while let Ok(msg) = rx.try_recv() {
            if matches!(&msg, WorkerMsg::RunCommand { cmd: Command::Write(Some(p)), .. } if p == "notes.md") {
                found = true;
            }
        }
        assert!(found, "RunCommand Write(notes.md) erwartet");
    }

    #[test]
    fn loaded_skills_become_commands() {
        let (mut a, mut rx) = app();
        a.apply_event(AppEvent::SkillsLoaded(vec!["simplify".into()]));
        type_command(&mut a, "simplify");
        assert!(matches!(
            rx.try_recv(),
            Ok(WorkerMsg::RunCommand { cmd: Command::Skill(s), .. }) if s == "simplify"
        ));
    }
}
