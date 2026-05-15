use dune_ctl_core::{
    battlegroup::{MapConsistency, MapEntry},
    diagnostics::{Check, CheckState},
    fls::FlsTokenState,
    health::HealthSnapshot,
    settings,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Tabs, Wrap},
    Frame,
};

use super::app::{App, View};

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header bar
            Constraint::Length(3), // tabs
            Constraint::Min(10),   // active view
            Constraint::Length(7), // log pane
            Constraint::Length(1), // key hints
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_tabs(f, app, chunks[1]);
    match app.view {
        View::Dashboard => draw_dashboard(f, app, chunks[2]),
        View::Maps => draw_maps_view(f, app, chunks[2]),
        View::Settings => draw_settings_view(f, app, chunks[2]),
    }
    draw_log(f, app, chunks[3]);
    draw_hints(f, app, chunks[4]);
    if app.pending.is_some() {
        draw_confirmation(f, app);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let snap = app.snapshot.as_ref();
    let phase = snap.map(|s| s.battlegroup_phase.as_str()).unwrap_or("—");
    let title = snap
        .and_then(|s| s.battlegroup_title.as_deref())
        .unwrap_or("dune-ctl");
    let loading = if app.loading { " [loading]" } else { "" };

    let fls_span = snap
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

    let ram_span = snap
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
            format!(
                " {}  {}  Phase:{}{}",
                title, app.cfg.battlegroup, phase, loading
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        fls_span,
        ram_span,
    ]);

    if area.width < 96 {
        f.render_widget(
            Paragraph::new(line).block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    }

    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(30)])
        .split(inner);

    f.render_widget(Paragraph::new(line), header_chunks[0]);
    f.render_widget(
        Paragraph::new(mascot_line(app))
            .alignment(Alignment::Right)
            .style(Style::default().fg(Color::Yellow)),
        header_chunks[1],
    );
}

fn mascot_line(app: &App) -> &'static str {
    const FRAMES: [&str; 6] = [
        r" /\_/\  __/^^^^\__",
        r"( o.o )__/^^^^\_ ",
        r" > ^ <  _/^^^^\__",
        r"( o.o )___/^^^\_",
        r" /\_/\  __/^^^^\__",
        r"( -.- )__/^^^^\_ ",
    ];

    let frame = (app.started_at.elapsed().as_millis() / 400) as usize % FRAMES.len();
    FRAMES[frame]
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let selected = match app.view {
        View::Dashboard => 0,
        View::Maps => 1,
        View::Settings => 2,
    };
    let titles = ["1 Dashboard", "2 Maps", "3 Settings"];
    f.render_widget(
        Tabs::new(titles)
            .select(selected)
            .block(Block::default().borders(Borders::ALL))
            .style(Style::default().fg(Color::DarkGray))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        area,
    );
}

fn draw_dashboard(f: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(8)])
        .split(columns[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Min(8),
        ])
        .split(columns[1]);

    draw_overview(f, app.snapshot.as_ref(), left[0]);
    draw_runtime_servers(f, app.snapshot.as_ref(), left[1]);
    draw_gateway_panel(f, app.snapshot.as_ref(), right[0]);
    draw_diagnostics(f, app.snapshot.as_ref(), right[1]);
    draw_utilities(f, app.snapshot.as_ref(), right[2]);
}

fn draw_overview(f: &mut Frame, snap: Option<&HealthSnapshot>, area: Rect) {
    let Some(snap) = snap else {
        f.render_widget(
            panel(
                "Overview",
                vec![Line::from("Collecting health snapshot...")],
            ),
            area,
        );
        return;
    };

    let running_maps = snap.maps.iter().filter(|m| m.replicas > 0).count();
    let split_maps = snap
        .maps
        .iter()
        .filter(|m| m.consistency == MapConsistency::Split)
        .count();
    let players: u32 = snap.maps.iter().filter_map(|m| m.players).sum();
    let stopped = if snap.battlegroup_stopped {
        "yes"
    } else {
        "no"
    };
    let uptime = snap.battlegroup_started_at.as_deref().unwrap_or("unknown");

    let lines = vec![
        Line::from(vec![
            Span::styled("Phase ", Style::default().fg(Color::DarkGray)),
            status_span(&snap.battlegroup_phase),
            Span::raw(format!("   Stopped: {}", stopped)),
        ]),
        Line::from(format!(
            "Maps: {} desired / {} total   Active servers: {}   Players: {}",
            running_maps,
            snap.maps.len(),
            snap.battlegroup_size.unwrap_or(0),
            players
        )),
        Line::from(format!("Started: {}", uptime)),
        Line::from(vec![
            Span::styled("Consistency: ", Style::default().fg(Color::DarkGray)),
            if split_maps == 0 {
                Span::styled("clean", Style::default().fg(Color::Green))
            } else {
                Span::styled(
                    format!("{} split map(s)", split_maps),
                    Style::default().fg(Color::Red),
                )
            },
        ]),
    ];
    f.render_widget(panel("Overview", lines), area);
}

fn draw_gateway_panel(f: &mut Frame, snap: Option<&HealthSnapshot>, area: Rect) {
    let lines = if let Some(gw) = snap.and_then(|s| s.gateway.as_ref()) {
        vec![
            Line::from(vec![Span::raw("RMQ HTTP patch: "), bool_span(gw.patched)]),
            Line::from(format!(
                "Ready replicas: {}   Updated: {}",
                opt_u32(gw.ready_replicas),
                opt_u32(gw.updated_replicas)
            )),
            Line::from("Expected GameRmqHttpAddress: 47.145.51.160:30196"),
        ]
    } else {
        vec![Line::from("Gateway deployment status unavailable.")]
    };
    f.render_widget(panel("Gateway", lines), area);
}

fn draw_diagnostics(f: &mut Frame, snap: Option<&HealthSnapshot>, area: Rect) {
    let Some(snap) = snap else {
        f.render_widget(
            panel("Diagnostics", vec![Line::from("Collecting diagnostics...")]),
            area,
        );
        return;
    };

    let lines = vec![
        check_line("firewalld", &snap.diagnostics.firewall_backend),
        check_line("nft stale", &snap.diagnostics.stale_nft_firewalld),
        Line::from(format!(
            "nft tables: {}",
            if snap.diagnostics.nft_tables.is_empty() {
                "none/unknown".to_string()
            } else {
                snap.diagnostics.nft_tables.join(", ")
            }
        )),
    ];
    f.render_widget(panel("Diagnostics", lines), area);
}

fn draw_runtime_servers(f: &mut Frame, snap: Option<&HealthSnapshot>, area: Rect) {
    let rows: Vec<Row> = snap
        .map(|s| s.runtime_servers.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|server| {
            Row::new(vec![
                Cell::from(server.map.clone()),
                Cell::from(opt_u32(server.partition)),
                Cell::from(opt_u16(server.port)),
                Cell::from(server.phase.clone())
                    .style(Style::default().fg(phase_color(&server.phase))),
                Cell::from(if server.ready { "yes" } else { "no" }),
                Cell::from(opt_u32(server.restarts)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(18),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(8),
        ],
    )
    .header(header_row(vec![
        "Map", "Part", "Port", "Phase", "Ready", "Restart",
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Runtime Servers"),
    );
    f.render_widget(table, area);
}

fn draw_utilities(f: &mut Frame, snap: Option<&HealthSnapshot>, area: Rect) {
    let rows: Vec<Row> = snap
        .map(|s| s.utilities.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|utility| {
            Row::new(vec![
                Cell::from(utility.name.clone()),
                Cell::from(utility.phase.clone())
                    .style(Style::default().fg(phase_color(&utility.phase))),
                Cell::from(utility.address.clone().unwrap_or_else(|| "—".to_string())),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Min(16),
        ],
    )
    .header(header_row(vec!["Service", "Phase", "Address"]))
    .block(Block::default().borders(Borders::ALL).title("Utilities"));
    f.render_widget(table, area);
}

fn draw_maps_view(f: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);
    draw_maps_table(f, app, columns[0]);
    draw_map_detail(f, app, columns[1]);
}

fn draw_maps_table(f: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec![
        Cell::from("  Map").style(header_style()),
        Cell::from("Type").style(header_style()),
        Cell::from("Desired").style(header_style()),
        Cell::from("Scale").style(header_style()),
        Cell::from("Phase").style(header_style()),
        Cell::from("Ready").style(header_style()),
        Cell::from("Players").style(header_style()),
        Cell::from("Port").style(header_style()),
        Cell::from("State").style(header_style()),
    ]);

    let rows: Vec<Row> = app
        .snapshot
        .as_ref()
        .map(|s| s.maps.as_slice())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(i, map)| {
            let dot = map_dot(map);
            let row_style = if i == app.selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(format!("{}{}", dot, map.name)),
                Cell::from(map.category.label()),
                Cell::from(map.replicas.to_string()),
                Cell::from(opt_u32(map.scale_replicas)),
                Cell::from(map.phase.clone()).style(Style::default().fg(phase_color(&map.phase))),
                Cell::from(format!(
                    "{}/{}",
                    opt_u32(map.ready_replicas),
                    opt_u32(map.target_replicas)
                )),
                Cell::from(opt_u32(map.players)),
                Cell::from(opt_u16(map.game_port)),
                Cell::from(map.consistency.label())
                    .style(Style::default().fg(consistency_color(map.consistency))),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(24),
            Constraint::Length(11),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(12),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(10),
        ],
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

fn draw_map_detail(f: &mut Frame, app: &App, area: Rect) {
    let map = app.snapshot.as_ref().and_then(|s| s.maps.get(app.selected));

    let Some(map) = map else {
        f.render_widget(
            panel("Map Detail", vec![Line::from("No map selected.")]),
            area,
        );
        return;
    };

    let lines = vec![
        Line::from(Span::styled(
            map.name.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("Type: {}", map.category.label())),
        Line::from(vec![Span::raw("Phase: "), status_span(&map.phase)]),
        Line::from(format!(
            "Desired/Scale: {} / {}",
            map.replicas,
            opt_u32(map.scale_replicas)
        )),
        Line::from(format!(
            "Ready/Target: {} / {}",
            opt_u32(map.ready_replicas),
            opt_u32(map.target_replicas)
        )),
        Line::from(format!("Consistency: {}", map.consistency.label())),
        Line::from(format!("Players: {}", opt_u32(map.players))),
        Line::from(format!("Port: {}", opt_u16(map.game_port))),
        Line::from(format!("SFPS: {}", map.sfps.as_deref().unwrap_or("—"))),
        Line::from(format!(
            "Memory: {} / {}",
            map.memory_request.as_deref().unwrap_or("—"),
            map.memory_limit.as_deref().unwrap_or("—")
        )),
        Line::from(format!(
            "Partitions: {}",
            if map.partitions.is_empty() {
                "—".to_string()
            } else {
                map.partitions
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            }
        )),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Map Detail"))
            .wrap(Wrap { trim: true }),
        area,
    );
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

fn draw_hints(f: &mut Frame, app: &App, area: Rect) {
    let view_hint = match app.view {
        View::Dashboard => "[Tab/2] maps",
        View::Maps => "[Tab/3] settings",
        View::Settings => "[Tab/1] dashboard",
    };
    let view_actions = match app.view {
        View::Settings => " [t] toggle  [a] apply settings ",
        _ => " [s/x] map ",
    };
    f.render_widget(
        Paragraph::new(format!(
            " {}  [A] start BG  [Z] stop BG  [R] restart BG {} [g] gateway  [r] refresh  [q] quit",
            view_hint, view_actions
        ))
        .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn draw_settings_view(f: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);
    draw_settings_table(f, app, columns[0]);
    draw_settings_detail(f, app, columns[1]);
}

fn draw_settings_table(f: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec![
        Cell::from("  Key").style(header_style()),
        Cell::from("Value").style(header_style()),
        Cell::from("File").style(header_style()),
        Cell::from("Type").style(header_style()),
        Cell::from("Label").style(header_style()),
    ]);

    let rows: Vec<Row> = app
        .settings
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let row_style = if i == app.settings_selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            let is_bool = settings::is_bool(item.def.kind);
            let dot = if is_bool {
                match item.value.as_deref().map(boolish) {
                    Some(true) => "● ",
                    Some(false) => "○ ",
                    None => "? ",
                }
            } else {
                "  "
            };
            Row::new(vec![
                Cell::from(format!("{}{}", dot, item.def.key)),
                Cell::from(item.value.clone().unwrap_or_else(|| "—".to_string())),
                Cell::from(item.def.file.label()),
                Cell::from(settings::kind_label(item.def.kind)),
                Cell::from(item.def.label),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(30),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Managed Settings"),
    );

    let mut state = TableState::default();
    if !app.settings.is_empty() {
        state.select(Some(app.settings_selected));
    }
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_settings_detail(f: &mut Frame, app: &App, area: Rect) {
    let Some(item) = app.settings.get(app.settings_selected) else {
        f.render_widget(
            panel("Setting Detail", vec![Line::from("No setting selected.")]),
            area,
        );
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(
            item.def.key,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(item.def.label),
        Line::from(""),
        Line::from(format!("Value: {}", item.value.as_deref().unwrap_or("—"))),
        Line::from(format!("File: {}", item.def.file.filename())),
        Line::from(format!("Section: [{}]", item.def.section)),
        Line::from(format!("INI key: {}", item.def.ini_key)),
        Line::from(format!("Type: {}", settings::kind_label(item.def.kind))),
    ];
    if settings::is_bool(item.def.kind) {
        lines.push(Line::from(""));
        lines.push(Line::from("[t] toggles this setting locally"));
    }
    lines.push(Line::from("[a] deploys both User*.ini files"));

    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Setting Detail"),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_confirmation(f: &mut Frame, app: &App) {
    let Some(action) = app.pending else {
        return;
    };
    let area = centered_rect(62, 9, f.area());
    let lines = vec![
        Line::from(Span::styled(
            format!("Confirm {}", action.label()),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(action.risk()),
        Line::from(""),
        Line::from(vec![
            Span::styled("[y/Enter] confirm", Style::default().fg(Color::Green)),
            Span::raw("    "),
            Span::styled("[n/Esc] cancel", Style::default().fg(Color::Red)),
        ]),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Confirm")),
        area,
    );
}

fn panel<'a>(title: &'a str, lines: Vec<Line<'a>>) -> Paragraph<'a> {
    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: true })
}

fn header_row(labels: Vec<&str>) -> Row<'static> {
    Row::new(
        labels
            .into_iter()
            .map(|label| Cell::from(label.to_string()).style(header_style()))
            .collect::<Vec<_>>(),
    )
}

fn header_style() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn phase_color(phase: &str) -> Color {
    match phase {
        "Healthy" | "Ready" | "Running" => Color::Green,
        "Stopped" => Color::DarkGray,
        "Starting" | "Stopping" | "Pending" => Color::Yellow,
        "Failed" | "Error" => Color::Red,
        _ => Color::White,
    }
}

fn status_span(value: &str) -> Span<'static> {
    Span::styled(value.to_string(), Style::default().fg(phase_color(value)))
}

fn bool_span(value: bool) -> Span<'static> {
    if value {
        Span::styled("yes", Style::default().fg(Color::Green))
    } else {
        Span::styled("no", Style::default().fg(Color::Red))
    }
}

fn check_line(label: &str, check: &Check) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<10}", label),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{:<8}", check.state.label()),
            Style::default().fg(check_color(check.state)),
        ),
        Span::raw(check.message.clone()),
    ])
}

fn check_color(state: CheckState) -> Color {
    match state {
        CheckState::Ok => Color::Green,
        CheckState::Warning => Color::Yellow,
        CheckState::Critical => Color::Red,
        CheckState::Unknown => Color::White,
    }
}

fn consistency_color(consistency: MapConsistency) -> Color {
    match consistency {
        MapConsistency::CleanOn => Color::Green,
        MapConsistency::CleanOff => Color::DarkGray,
        MapConsistency::Starting | MapConsistency::Stopping => Color::Yellow,
        MapConsistency::Split => Color::Red,
        MapConsistency::Unknown => Color::White,
    }
}

fn map_dot(map: &MapEntry) -> &'static str {
    match map.consistency {
        MapConsistency::CleanOn => "● ",
        MapConsistency::CleanOff => "○ ",
        MapConsistency::Starting | MapConsistency::Stopping => "◐ ",
        MapConsistency::Split => "! ",
        MapConsistency::Unknown => "? ",
    }
}

fn boolish(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn opt_u32(value: Option<u32>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn opt_u16(value: Option<u16>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}
