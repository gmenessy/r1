//! Rendering der Full-Screen-TUI (ratatui). Reines Zeichnen – kein Zustand.

use crate::app::{App, Mode, Review};
use crate::diff::Op;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

pub fn draw(frame: &mut Frame, app: &App) {
    let ghost_height = if app.ghost.is_some() { 3 } else { 0 };
    let cmd_height = if app.mode == Mode::Command { 3 } else { 0 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(ghost_height),
            Constraint::Length(1),
            Constraint::Length(cmd_height),
        ])
        .split(frame.area());

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
        .split(rows[0]);

    match &app.review {
        Some(review) if app.mode == Mode::Review => draw_review(frame, review, main[0]),
        _ => draw_editor(frame, app, main[0]),
    }
    draw_hints(frame, app, main[1]);
    if app.ghost.is_some() {
        draw_ghost(frame, app, rows[1]);
    }
    draw_status(frame, app, rows[2]);
    if app.mode == Mode::Command {
        draw_command_bar(frame, app, rows[3]);
    }
}

fn draw_editor(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Vibe-Editor · {} · {} ", app.file_label(), app.model));
    let inner = block.inner(area);

    // Scrolling: Cursor immer sichtbar halten – vertikal und horizontal.
    let height = inner.height.max(1) as usize;
    let scroll = app.buffer.row.saturating_sub(height - 1);
    // Anzeigebreite bis zum Cursor (CJK/Emoji sind 2 Spalten breit).
    let prefix: String = app.buffer.current_line().chars().take(app.buffer.col).collect();
    let display_col = UnicodeWidthStr::width(prefix.as_str());
    let width = inner.width.max(1) as usize;
    let hscroll = display_col.saturating_sub(width - 1);

    let lines: Vec<Line> = app
        .buffer
        .lines
        .iter()
        .skip(scroll)
        .take(height)
        .enumerate()
        .map(|(i, content)| {
            let absolute = scroll + i;
            let ghost_here = app.ghost.as_ref().map(|g| g.line == absolute).unwrap_or(false);
            if ghost_here {
                Line::from(Span::styled(
                    content.clone(),
                    Style::default().add_modifier(Modifier::UNDERLINED),
                ))
            } else {
                Line::from(content.clone())
            }
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).block(block).scroll((0, hscroll as u16)),
        area,
    );

    if app.mode == Mode::Edit {
        let x = inner.x + (display_col - hscroll) as u16;
        let y = inner.y + (app.buffer.row - scroll) as u16;
        frame.set_cursor_position(Position::new(x, y));
    }
}

/// Wort-Diff-Vorschau: grün = neu, rot durchgestrichen = entfernt.
/// Der Puffer wird erst mit Enter ersetzt – Esc verwirft den Vorschlag.
fn draw_review(frame: &mut Frame, review: &Review, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Vorschau · {} — Enter übernehmen · Esc verwerfen ",
            review.notice
        ))
        .border_style(Style::default().fg(Color::Yellow));
    let paragraph = Paragraph::new(diff_lines(&review.diff))
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((review.scroll, 0));
    frame.render_widget(paragraph, area);
}

fn diff_lines(diff: &[(Op, String)]) -> Vec<Line<'static>> {
    let insert = Style::default().fg(Color::Green);
    let delete = Style::default()
        .fg(Color::Red)
        .add_modifier(Modifier::CROSSED_OUT);

    let mut lines = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    for (op, text) in diff {
        let style = match op {
            Op::Equal => Style::default(),
            Op::Insert => insert,
            Op::Delete => delete,
        };
        let mut parts = text.split('\n').peekable();
        while let Some(part) = parts.next() {
            if !part.is_empty() {
                spans.push(Span::styled(part.to_string(), style));
            }
            if parts.peek().is_some() {
                if *op == Op::Delete {
                    // Entfernter Zeilenumbruch: Marker statt echtem Umbruch,
                    // sonst verschiebt der alte Umbruch das neue Layout.
                    spans.push(Span::styled("⏎", delete));
                } else {
                    lines.push(Line::from(std::mem::take(&mut spans)));
                }
            }
        }
    }
    lines.push(Line::from(spans));
    lines
}

fn draw_hints(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Wiki · Kontext ");
    let items: Vec<ListItem> = if app.hints.is_empty() {
        vec![ListItem::new(Span::styled(
            "Keine thematischen Treffer",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.hints
            .iter()
            .map(|hint| {
                let title = Line::from(Span::styled(
                    format!("◆ {} ({:.0}%)", hint.title, hint.score * 100.0),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ));
                let summary = Line::from(Span::styled(
                    hint.summary.clone(),
                    Style::default().fg(Color::Gray),
                ));
                ListItem::new(vec![title, summary, Line::from("")])
            })
            .collect()
    };
    frame.render_widget(List::new(items).block(block), area);
}

fn draw_ghost(frame: &mut Frame, app: &App, area: Rect) {
    let Some(ghost) = &app.ghost else { return };
    let line = Line::from(vec![
        Span::styled("✨ ", Style::default().fg(Color::Yellow)),
        Span::styled(ghost.corrected.clone(), Style::default().fg(Color::DarkGray)),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Ghost-Vorschlag · Tab übernehmen · Esc verwerfen ")
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(Paragraph::new(line).block(block), area);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    // Datenpfad-Badge: Auf einen Blick sehen, ob Text das Gerät verlässt.
    let badge = if app.cloud {
        Span::styled(" ☁ CLOUD ", Style::default().fg(Color::Black).bg(Color::Yellow))
    } else {
        Span::styled(" 🔒 LOKAL ", Style::default().fg(Color::Black).bg(Color::Green))
    };
    let mut spans = vec![
        badge,
        Span::styled(
            format!(" {} ", app.model),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
    ];
    if !app.ghost_enabled {
        spans.push(Span::styled(" Ghost aus ", Style::default().fg(Color::DarkGray)));
    }
    if let Some(activity) = &app.activity {
        spans.push(Span::styled(
            format!(" {} {} ", app.spinner_char(), activity),
            Style::default().fg(Color::Yellow),
        ));
    }
    let message = if app.status.is_empty() {
        " Ctrl+P Befehle · Tab Ghost · Ctrl+Z Undo · Ctrl+S Wiki · /write Datei · Ctrl+Q beenden"
            .to_string()
    } else {
        format!(" {}", app.status)
    };
    spans.push(Span::styled(message, Style::default().fg(Color::Gray)));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_command_bar(frame: &mut Frame, app: &App, area: Rect) {
    let completions = if app.completions.is_empty() {
        String::new()
    } else {
        format!("   ⇥ {}", app.completions.join("  "))
    };
    let line = Line::from(vec![
        Span::styled("/", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(app.cmdline.clone()),
        Span::styled("▌", Style::default().fg(Color::Cyan)),
        Span::styled(completions, Style::default().fg(Color::DarkGray)),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Befehl ")
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(Paragraph::new(line).block(block), area);
}
