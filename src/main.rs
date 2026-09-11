//! Vibe-Editor CLI – Einstiegspunkt.
//!
//! 3-Säulen-Architektur:
//! 1. UI-Thread (dieser OS-Thread): 60-FPS-Event-Loop, niemals blockiert.
//! 2. Inferenz: Tokio-Tasks (lokal via Ollama, Cloud via reqwest).
//! 3. I/O: Tokio-Tasks für Wiki-Storage und Indizierung.
//!
//! Kommunikation ausschließlich über Unbounded-Channels.

mod app;
mod buffer;
mod commands;
mod diff;
mod events;
mod export;
mod llm;
mod skills;
mod ui;
mod wiki;
mod worker;

use app::App;
use events::{AppEvent, WorkerMsg};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

const FRAME_BUDGET: Duration = Duration::from_millis(16); // ~60 FPS

fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!("Nutzung: vibe [datei]\n\nOhne Datei startet ein leerer Puffer; ein Autosave\nnach Absturz wird automatisch wiederhergestellt.");
        return Ok(());
    }

    let (to_worker, worker_rx) = unbounded_channel::<WorkerMsg>();
    let (to_ui, mut ui_rx) = unbounded_channel::<AppEvent>();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.spawn(worker::run(worker_rx, to_ui));

    let mut app = App::new(to_worker.clone());
    open_initial_buffer(&mut app, arg)?;

    let mut terminal = ratatui::init();
    let result = run_ui(&mut terminal, &mut app, &mut ui_rx);
    ratatui::restore();

    // Sauberes Ende: Autosave ist nur für Abstürze gedacht.
    if result.is_ok() {
        let _ = std::fs::remove_file(worker::autosave_path());
    }
    let _ = to_worker.send(WorkerMsg::Shutdown);
    runtime.shutdown_timeout(Duration::from_millis(500));
    result
}

/// CLI-Argument öffnen; ohne Argument ggf. Autosave-Recovery.
fn open_initial_buffer(app: &mut App, arg: Option<String>) -> anyhow::Result<()> {
    if let Some(path) = arg {
        let path = PathBuf::from(path);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => anyhow::bail!("{}: {e}", path.display()),
        };
        app.load_initial(path, &text);
        return Ok(());
    }
    let autosave = worker::autosave_path();
    if let Ok(text) = std::fs::read_to_string(&autosave) {
        if !text.trim().is_empty() {
            app.buffer.set_text(&text);
            app.status =
                "♻ Puffer aus Autosave wiederhergestellt – /write <datei> zum Sichern".into();
        }
    }
    Ok(())
}

fn run_ui(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    ui_rx: &mut UnboundedReceiver<AppEvent>,
) -> anyhow::Result<()> {
    loop {
        // Worker-Ereignisse einsammeln – nicht-blockierend.
        while let Ok(ev) = ui_rx.try_recv() {
            app.apply_event(ev);
        }
        app.tick();
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Auf Eingabe warten, aber höchstens einen Frame lang.
        if event::poll(FRAME_BUDGET)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && app.handle_key(key) {
                    return Ok(());
                }
            }
        }
    }
}
