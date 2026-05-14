use dune_ctl_core::fls::FlsTokenState;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use super::app::App;

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header bar
            Constraint::Min(5),    // map table
            Constraint::Length(8), // log pane
            Constraint::Length(1), // key hints
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_maps(f, app, chunks[1]);
    draw_log(f, app, chunks[2]);
    draw_hints(f, chunks[3]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let phase = app
        .snapshot
        .as_ref()
        .map(|s| s.battlegroup_phase.as_str())
        .unwrap_or("—");
    let loading = if app.loading { " [loading]" } else { "" };

    let fls_span = app
        .snapshot
        .as_ref()
        .and_then(|s| s.fls.as_ref())
        .map(|f| {
            let color = match f.state {
                FlsTokenState::Ok => Color::Green,
                FlsTokenState::WarningSoon => Color::Yellow,
                FlsTokenState::Critical | FlsTokenState::Expired => Color::Red,
            };
            Span::styled(format!("  FLS:{} ", f.label()), Style::default().fg(color))
        })
        .unwrap_or_else(|| Span::raw("  FLS:— "));

    let ram_span = app
        .snapshot
        .as_ref()
        .and_then(|s| s.ram_used_bytes.zip(s.ram_total_bytes))
        .map(|(used, total)| {
            Span::raw(format!(
                " RAM:{:.1}/{:.1}GB ",
                used as f64 / 1e9,
                total as f64 / 1e9
            ))
        })
        .unwrap_or_else(|| Span::raw(" RAM:— "));

    let line = Line::from(vec![
        Span::styled(
            format!(" dune-ctl  {}  Phase:{}{}", app.cfg.battlegroup, phase, loading),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        fls_span,
        ram_span,
    ]);

    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_maps(f: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec![
        Cell::from("  Map").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Phase").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Rep").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);

    let rows: Vec<Row> = app
        .snapshot
        .as_ref()
        .map(|s| s.maps.as_slice())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(i, map)| {
            let dot = if map.phase == "Running" { "● " } else { "○ " };
            let phase_color = match map.phase.as_str() {
                "Running" => Color::Green,
                "Stopped" => Color::DarkGray,
                "Starting" | "Stopping" => Color::Yellow,
                _ => Color::White,
            };
            let row_style = if i == app.selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(format!("{}{}", dot, map.name)),
                Cell::from(map.phase.clone()).style(Style::default().fg(phase_color)),
                Cell::from(map.replicas.to_string()),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [Constraint::Min(32), Constraint::Length(12), Constraint::Length(4)],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("Maps"));

    let mut state = TableState::default();
    if app
        .snapshot
        .as_ref()
        .map(|s| !s.maps.is_empty())
        .unwrap_or(false)
    {
        state.select(Some(app.selected));
    }
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_log(f: &mut Frame, app: &App, area: Rect) {
    let visible = (area.height as usize).saturating_sub(2); // minus borders
    let lines: Vec<Line> = app
        .log
        .iter()
        .rev()
        .take(visible)
        .rev()
        .map(|l| Line::from(l.as_str()))
        .collect();

    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Log")),
        area,
    );
}

fn draw_hints(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(
            " [j/k↑↓] select  [s] start  [x] stop  [g] gateway-patch  [r] refresh  [q] quit",
        )
        .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
