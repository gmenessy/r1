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
mod events;
mod llm;
mod ui;
mod wiki;
mod worker;

use app::App;
use events::{AppEvent, WorkerMsg};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

const FRAME_BUDGET: Duration = Duration::from_millis(16); // ~60 FPS

fn main() -> anyhow::Result<()> {
    let (to_worker, worker_rx) = unbounded_channel::<WorkerMsg>();
    let (to_ui, mut ui_rx) = unbounded_channel::<AppEvent>();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.spawn(worker::run(worker_rx, to_ui));

    let mut terminal = ratatui::init();
    let mut app = App::new(to_worker.clone());
    let result = run_ui(&mut terminal, &mut app, &mut ui_rx);
    ratatui::restore();

    let _ = to_worker.send(WorkerMsg::Shutdown);
    runtime.shutdown_timeout(Duration::from_millis(500));
    result
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
