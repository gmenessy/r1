//! UI-seitiger Zustand und Tastatur-Logik. Läuft komplett auf dem
//! UI-Thread – keine blockierenden Aufrufe.

use crate::buffer::Buffer;
use crate::commands::{self, Command};
use crate::events::{AppEvent, WikiHint, WorkerMsg};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedSender;

#[derive(PartialEq, Eq)]
pub enum Mode {
    Edit,
    Command,
}

pub struct Ghost {
    pub line: usize,
    pub original: String,
    pub corrected: String,
}

pub struct App {
    pub buffer: Buffer,
    pub mode: Mode,
    pub cmdline: String,
    pub completions: Vec<&'static str>,
    pub ghost: Option<Ghost>,
    pub hints: Vec<WikiHint>,
    pub status: String,
    pub activity: Option<String>,
    pub model: String,
    pub spinner: usize,
    pub generation: u64,
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
            hints: Vec::new(),
            status: String::new(),
            activity: None,
            model: "Gemma · lokal".into(),
            spinner: 0,
            generation: 0,
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
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match (key.code, ctrl) {
            (KeyCode::Char('q'), true) => return true,
            (KeyCode::Char('p'), true) => self.open_command_bar(),
            (KeyCode::Char('s'), true) => self.run_command(Command::Save(None), "Speichert…"),
            (KeyCode::Tab, _) => self.accept_ghost(),
            (KeyCode::Esc, _) => self.ghost = None,
            (KeyCode::Char('/'), false) if self.buffer.current_line().is_empty() => {
                self.open_command_bar()
            }
            (KeyCode::Char(c), false) => {
                self.buffer.insert_char(c);
                self.on_edit();
            }
            (KeyCode::Enter, _) => {
                self.buffer.newline();
                self.on_edit();
            }
            (KeyCode::Backspace, _) => {
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
                self.buffer.replace_line(ghost.line, &ghost.corrected);
                if self.buffer.row == ghost.line {
                    self.buffer.end();
                }
                self.status = "✓ Ghost-Korrektur übernommen".into();
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
            AppEvent::ReplaceBuffer { text, notice } => {
                self.buffer.set_text(&text);
                self.generation = self.generation.wrapping_add(1);
                self.ghost = None;
                self.activity = None;
                self.status = format!("✓ {notice}");
            }
            AppEvent::WikiHints(hints) => self.hints = hints,
            AppEvent::Saved { path } => {
                self.activity = None;
                self.status = format!("✓ Gespeichert: {path}");
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
        Command::Help | Command::Quit => "",
    }
}
