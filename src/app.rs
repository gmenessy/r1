//! UI-seitiger Zustand und Tastatur-Logik. Läuft komplett auf dem
//! UI-Thread – keine blockierenden Aufrufe.

use crate::buffer::Buffer;
use crate::commands::{self, Command};
use crate::diff::{self, Op};
use crate::events::{AppEvent, WikiHint, WorkerMsg};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

pub struct App {
    pub buffer: Buffer,
    pub mode: Mode,
    pub cmdline: String,
    pub completions: Vec<&'static str>,
    pub ghost: Option<Ghost>,
    pub review: Option<Review>,
    pub hints: Vec<WikiHint>,
    pub status: String,
    pub activity: Option<String>,
    pub model: String,
    pub spinner: usize,
    pub generation: u64,
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
            review: None,
            hints: Vec::new(),
            status: String::new(),
            activity: None,
            model: "Gemma · lokal".into(),
            spinner: 0,
            generation: 0,
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
            (KeyCode::Tab, _) => self.accept_ghost(),
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
                match commands::parse(&input) {
                    Ok(Command::Quit) => return true,
                    Ok(Command::Help) => self.status = commands::help_text(),
                    Ok(cmd) => {
                        let label = activity_label(&cmd);
                        self.run_command(cmd, label);
                    }
                    Err(e) => self.status = format!("✖ {e}"),
                }
            }
            KeyCode::Tab => {
                if let Some(full) = commands::complete_first(&self.cmdline) {
                    self.cmdline = full;
                    self.cmdline.push(' ');
                }
                self.completions = commands::completions(&self.cmdline);
            }
            KeyCode::Backspace => {
                if self.cmdline.pop().is_none() {
                    self.mode = Mode::Edit;
                }
                self.completions = commands::completions(&self.cmdline);
            }
            KeyCode::Char(c) => {
                self.cmdline.push(c);
                self.completions = commands::completions(&self.cmdline);
            }
            _ => {}
        }
        false
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
                    let _ = self.to_worker.send(WorkerMsg::TextChanged {
                        text: self.buffer.text(),
                        line: self.buffer.row,
                        generation: self.generation,
                    });
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
        self.completions = commands::completions("");
    }

    fn run_command(&mut self, cmd: Command, activity: &str) {
        self.activity = Some(activity.to_string());
        let _ = self.to_worker.send(WorkerMsg::RunCommand {
            cmd,
            text: self.buffer.text(),
        });
    }

    /// Nach jeder Pufferänderung: Ghost invalidieren, Worker informieren.
    fn on_edit(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.ghost = None;
        let _ = self.to_worker.send(WorkerMsg::TextChanged {
            text: self.buffer.text(),
            line: self.buffer.row,
            generation: self.generation,
        });
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
        let _ = self.to_worker.send(WorkerMsg::TextChanged {
            text: self.buffer.text(),
            line: self.buffer.row,
            generation: self.generation,
        });
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
                if generation == self.generation && line_unchanged {
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
            AppEvent::ModelSwitched(label) => {
                self.activity = None;
                self.model = label.clone();
                self.status = format!("✓ Modell: {label}");
            }
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
        Command::Save(_) => "Speichert…",
        Command::Export { .. } => "Exportiert…",
        Command::Help | Command::Quit => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(tx)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn ctrl(app: &mut App, c: char) {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    #[test]
    fn typing_burst_undoes_as_one_step() {
        let mut a = app();
        for c in "abc".chars() {
            press(&mut a, KeyCode::Char(c));
        }
        assert_eq!(a.buffer.text(), "abc");
        ctrl(&mut a, 'z');
        assert_eq!(a.buffer.text(), "");
    }

    #[test]
    fn redo_restores_undone_text() {
        let mut a = app();
        press(&mut a, KeyCode::Char('x'));
        ctrl(&mut a, 'z');
        assert_eq!(a.buffer.text(), "");
        ctrl(&mut a, 'y');
        assert_eq!(a.buffer.text(), "x");
    }

    #[test]
    fn proposal_needs_confirmation_and_is_undoable() {
        let mut a = app();
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
        let mut a = app();
        press(&mut a, KeyCode::Char('a'));
        a.apply_event(AppEvent::Proposal { text: "neu".into(), notice: "Test".into() });
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.buffer.text(), "a");
        assert!(a.mode == Mode::Edit);
        assert!(a.review.is_none());
    }

    #[test]
    fn identical_proposal_skips_review() {
        let mut a = app();
        press(&mut a, KeyCode::Char('a'));
        a.apply_event(AppEvent::Proposal { text: "a".into(), notice: "Test".into() });
        assert!(a.review.is_none());
        assert!(a.mode == Mode::Edit);
    }
}
