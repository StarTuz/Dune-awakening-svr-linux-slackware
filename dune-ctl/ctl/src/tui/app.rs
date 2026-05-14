use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use dune_ctl_core::{config::Config, gateway, health::HealthSnapshot, maps};
use ratatui::{backend::Backend, Terminal};

use super::ui;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const EVENT_TIMEOUT: Duration = Duration::from_millis(200);

pub struct App {
    pub cfg: Config,
    pub snapshot: Option<HealthSnapshot>,
    pub log: VecDeque<String>,
    pub selected: usize,
    pub loading: bool,
    pub running: bool,
}

impl App {
    fn new(cfg: Config) -> Self {
        Self {
            cfg,
            snapshot: None,
            log: VecDeque::with_capacity(64),
            selected: 0,
            loading: true,
            running: true,
        }
    }

    pub fn push_log(&mut self, msg: impl Into<String>) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        let line = format!("{} {}", ts, msg.into());
        self.log.push_back(line);
        while self.log.len() > 64 {
            self.log.pop_front();
        }
    }
}

pub async fn run_loop<B: Backend>(terminal: &mut Terminal<B>, cfg: &Config) -> Result<()> {
    let mut app = App::new(cfg.clone());
    app.push_log("dune-ctl started");

    refresh(&mut app).await;
    let mut last_poll = Instant::now();

    while app.running {
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(EVENT_TIMEOUT)? {
            if let Event::Key(key) = event::read()? {
                handle_key(&mut app, key.code, key.modifiers).await;
            }
        }

        if last_poll.elapsed() >= POLL_INTERVAL {
            refresh(&mut app).await;
            last_poll = Instant::now();
        }
    }
    Ok(())
}

async fn refresh(app: &mut App) {
    app.loading = true;
    match HealthSnapshot::collect(&app.cfg).await {
        Ok(snap) => {
            app.snapshot = Some(snap);
            app.loading = false;
        }
        Err(e) => {
            app.push_log(format!("refresh error: {:#}", e));
            app.loading = false;
        }
    }
}

async fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    let map_count = app.snapshot.as_ref().map(|s| s.maps.len()).unwrap_or(0);
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.running = false;
        }
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.running = false;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if map_count > 0 {
                app.selected = (app.selected + 1) % map_count;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if map_count > 0 && app.selected > 0 {
                app.selected -= 1;
            }
        }
        KeyCode::Char('s') => {
            if let Some(name) = selected_map(app) {
                app.push_log(format!("starting {}...", name));
                match maps::start(&app.cfg, &name).await {
                    Ok(()) => {
                        app.push_log(format!("{}: start triggered", name));
                        refresh(app).await;
                    }
                    Err(e) => app.push_log(format!("start error: {:#}", e)),
                }
            }
        }
        KeyCode::Char('x') => {
            if let Some(name) = selected_map(app) {
                app.push_log(format!("stopping {}...", name));
                match maps::stop(&app.cfg, &name).await {
                    Ok(()) => {
                        app.push_log(format!("{}: stop triggered", name));
                        refresh(app).await;
                    }
                    Err(e) => app.push_log(format!("stop error: {:#}", e)),
                }
            }
        }
        KeyCode::Char('g') => {
            app.push_log("applying gateway patch...");
            match gateway::patch(&app.cfg).await {
                Ok(true) => app.push_log("gateway: --RMQGameHttpPort=30196 applied"),
                Ok(false) => app.push_log("gateway: already patched"),
                Err(e) => app.push_log(format!("gateway patch error: {:#}", e)),
            }
        }
        KeyCode::Char('r') => {
            app.push_log("refreshing...");
            refresh(app).await;
        }
        _ => {}
    }
}

fn selected_map(app: &App) -> Option<String> {
    app.snapshot.as_ref()?.maps.get(app.selected).map(|m| m.name.clone())
}
