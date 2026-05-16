use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use dune_ctl_core::{
    config::{Config, WorldProfile},
    gateway,
    health::HealthSnapshot,
    maps, settings, sietches,
};
use ratatui::{backend::Backend, Terminal};
use tokio::task::JoinHandle;

use super::ui;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const EVENT_TIMEOUT: Duration = Duration::from_millis(200);
type RefreshResult = Result<(HealthSnapshot, Vec<settings::SettingValue>)>;

pub struct App {
    pub cfg: Config,
    pub started_at: Instant,
    pub snapshot: Option<HealthSnapshot>,
    pub settings: Vec<settings::SettingValue>,
    pub worlds: Vec<WorldProfile>,
    pub refresh_task: Option<JoinHandle<RefreshResult>>,
    pub log: VecDeque<String>,
    pub view: View,
    pub selected: usize,
    pub settings_selected: usize,
    pub pending: Option<PendingAction>,
    pub input: Option<InputMode>,
    pub loading: bool,
    pub running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Worlds,
    Dashboard,
    Maps,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingAction {
    StartSietch,
    StopSietch,
    RestartSietch,
    ApplySettings,
    InitWorldSettings,
    ClearSietchPassword,
}

#[derive(Debug, Clone)]
pub struct InputMode {
    pub key: String,
    pub label: String,
    pub value: String,
}

impl PendingAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::StartSietch => "start primary sietch",
            Self::StopSietch => "stop primary sietch",
            Self::RestartSietch => "restart primary sietch",
            Self::ApplySettings => "deploy settings",
            Self::InitWorldSettings => "initialize world settings profile",
            Self::ClearSietchPassword => "clear sietch password",
        }
    }

    pub fn risk(self) -> &'static str {
        match self {
            Self::StartSietch => {
                "Starts the selected world's primary Sietch. Current self-hosting maps this to BattleGroup start."
            }
            Self::StopSietch => {
                "Stops the selected world's primary Sietch. Current self-hosting maps this to BattleGroup stop, disconnecting players."
            }
            Self::RestartSietch => {
                "Restarts the selected world's primary Sietch by restarting the BattleGroup. Gateway patch may need verification after rollout."
            }
            Self::ApplySettings => {
                "Copies local UserEngine.ini and UserGame.ini into /srv/UserSettings. Some changes need a map or battlegroup restart."
            }
            Self::InitWorldSettings => {
                "Creates a per-world UserSettings profile. Future settings edits for this world will use that profile."
            }
            Self::ClearSietchPassword => {
                "Sets the local Sietch password to an empty string. Deploy settings to make it live."
            }
        }
    }
}

impl App {
    fn new(cfg: Config) -> Self {
        let worlds = Config::discover_worlds().unwrap_or_default();
        Self {
            cfg,
            started_at: Instant::now(),
            snapshot: None,
            settings: Vec::new(),
            worlds,
            refresh_task: None,
            log: VecDeque::with_capacity(64),
            view: View::Dashboard,
            selected: 0,
            settings_selected: 0,
            pending: None,
            input: None,
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

    start_refresh(&mut app);
    let mut last_poll = Instant::now();

    while app.running {
        finish_refresh(&mut app).await;
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(EVENT_TIMEOUT)? {
            if let Event::Key(key) = event::read()? {
                handle_key(&mut app, key.code, key.modifiers).await;
            }
        }

        if last_poll.elapsed() >= POLL_INTERVAL {
            start_refresh(&mut app);
            last_poll = Instant::now();
        }
    }
    Ok(())
}

fn start_refresh(app: &mut App) {
    if app.refresh_task.is_some() {
        return;
    }
    app.loading = true;
    let cfg = app.cfg.clone();
    app.refresh_task = Some(tokio::spawn(async move {
        let snap = HealthSnapshot::collect(&cfg).await?;
        let settings = settings::list(&cfg).await.unwrap_or_default();
        Ok((snap, settings))
    }));
}

async fn finish_refresh(app: &mut App) {
    if !app
        .refresh_task
        .as_ref()
        .map(|task| task.is_finished())
        .unwrap_or(false)
    {
        return;
    }

    let Some(task) = app.refresh_task.take() else {
        return;
    };

    match task.await {
        Ok(Ok((snap, settings))) => {
            let map_len = snap.maps.len();
            if app.selected >= map_len {
                app.selected = map_len.saturating_sub(1);
            }
            if app.settings_selected >= settings.len() {
                app.settings_selected = settings.len().saturating_sub(1);
            }
            app.settings = settings;
            app.snapshot = Some(snap);
            app.loading = false;
        }
        Ok(Err(e)) => {
            app.push_log(format!("refresh error: {:#}", e));
            app.loading = false;
        }
        Err(e) => {
            app.push_log(format!("refresh task error: {}", e));
            app.loading = false;
        }
    }
}

async fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.input.is_some() {
        handle_input_key(app, code).await;
        return;
    }

    if app.pending.is_some() {
        match code {
            KeyCode::Char('y') | KeyCode::Enter => execute_pending(app).await,
            KeyCode::Char('n') | KeyCode::Esc => {
                app.push_log("action cancelled");
                app.pending = None;
            }
            _ => {}
        }
        return;
    }

    let map_count = app.snapshot.as_ref().map(|s| s.maps.len()).unwrap_or(0);
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.running = false;
        }
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.running = false;
        }
        KeyCode::Tab => {
            app.view = match app.view {
                View::Worlds => View::Dashboard,
                View::Dashboard => View::Maps,
                View::Maps => View::Settings,
                View::Settings => View::Worlds,
            };
        }
        KeyCode::Char('1') => {
            app.view = View::Worlds;
        }
        KeyCode::Char('2') => {
            app.view = View::Dashboard;
        }
        KeyCode::Char('3') => {
            app.view = View::Maps;
        }
        KeyCode::Char('4') => {
            app.view = View::Settings;
        }
        KeyCode::Char('A') => {
            app.pending = Some(PendingAction::StartSietch);
        }
        KeyCode::Char('Z') => {
            app.pending = Some(PendingAction::StopSietch);
        }
        KeyCode::Char('R') => {
            app.pending = Some(PendingAction::RestartSietch);
        }
        KeyCode::Char('N') => {
            app.view = View::Settings;
            begin_setting_edit_by_key(app, "sietch_name");
        }
        KeyCode::Char('P') => {
            app.view = View::Settings;
            begin_setting_edit_by_key(app, "sietch_password");
        }
        KeyCode::Char('C') => {
            app.pending = Some(PendingAction::ClearSietchPassword);
        }
        KeyCode::Char('I') => {
            app.pending = Some(PendingAction::InitWorldSettings);
        }
        KeyCode::Down | KeyCode::Char('j') => match app.view {
            View::Settings if !app.settings.is_empty() => {
                app.settings_selected = (app.settings_selected + 1) % app.settings.len();
            }
            _ if map_count > 0 => {
                app.view = View::Maps;
                app.selected = (app.selected + 1) % map_count;
            }
            _ => {}
        },
        KeyCode::Up | KeyCode::Char('k') => match app.view {
            View::Settings if !app.settings.is_empty() && app.settings_selected > 0 => {
                app.settings_selected -= 1;
            }
            _ if map_count > 0 && app.selected > 0 => {
                app.view = View::Maps;
                app.selected -= 1;
            }
            _ => {}
        },
        KeyCode::Char('t') => {
            if app.view == View::Settings {
                toggle_selected_setting(app).await;
            }
        }
        KeyCode::Char('e') => {
            if app.view == View::Settings {
                begin_setting_edit(app);
            }
        }
        KeyCode::Char('a') => {
            if app.view == View::Settings {
                app.pending = Some(PendingAction::ApplySettings);
            }
        }
        KeyCode::Char('s') => {
            if let Some(name) = selected_map(app) {
                app.push_log(format!("starting {}...", name));
                match maps::start(&app.cfg, &name).await {
                    Ok(()) => {
                        app.push_log(format!("{}: start triggered", name));
                        start_refresh(app);
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
                        start_refresh(app);
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
            start_refresh(app);
        }
        _ => {}
    }
}

async fn handle_input_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.input = None;
            app.push_log("setting edit cancelled");
        }
        KeyCode::Enter => {
            let Some(input) = app.input.take() else {
                return;
            };
            match settings::set(&app.cfg, &input.key, &input.value).await {
                Ok(()) => {
                    if input.key == "sietch_password" {
                        app.push_log("sietch_password updated locally");
                    } else {
                        app.push_log(format!("{} set to {}", input.key, input.value));
                    }
                    start_refresh(app);
                }
                Err(e) => {
                    app.push_log(format!("settings edit error: {:#}", e));
                    app.input = Some(input);
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(input) = app.input.as_mut() {
                input.value.pop();
            }
        }
        KeyCode::Delete => {
            if let Some(input) = app.input.as_mut() {
                input.value.clear();
            }
        }
        KeyCode::Char(c) if !c.is_control() => {
            if let Some(input) = app.input.as_mut() {
                input.value.push(c);
            }
        }
        _ => {}
    }
}

fn begin_setting_edit(app: &mut App) {
    let Some(item) = app.settings.get(app.settings_selected) else {
        app.push_log("no setting selected");
        return;
    };
    let value = if item.def.secret {
        String::new()
    } else if item.def.key == "sietch_name" {
        item.value
            .clone()
            .or_else(|| {
                app.snapshot
                    .as_ref()
                    .and_then(|snap| snap.battlegroup_title.clone())
            })
            .unwrap_or_default()
    } else {
        item.value.clone().unwrap_or_default()
    };
    app.input = Some(InputMode {
        key: item.def.key.to_string(),
        label: item.def.label.to_string(),
        value,
    });
}

fn begin_setting_edit_by_key(app: &mut App, key: &str) {
    let Some(index) = app.settings.iter().position(|item| item.def.key == key) else {
        app.push_log(format!("{} setting not loaded", key));
        return;
    };
    app.settings_selected = index;
    begin_setting_edit(app);
}

async fn execute_pending(app: &mut App) {
    let Some(action) = app.pending.take() else {
        return;
    };
    app.push_log(format!("confirming {}...", action.label()));
    let result = match action {
        PendingAction::StartSietch => sietches::start_primary(&app.cfg).await,
        PendingAction::StopSietch => sietches::stop_primary(&app.cfg).await,
        PendingAction::RestartSietch => sietches::restart_primary(&app.cfg).await,
        PendingAction::ApplySettings => settings::apply(&app.cfg).await,
        PendingAction::InitWorldSettings => app.cfg.init_world_settings().map(|_| ()),
        PendingAction::ClearSietchPassword => settings::set(&app.cfg, "sietch_password", "").await,
    };
    match result {
        Ok(()) => {
            app.push_log(format!("{} triggered", action.label()));
            app.worlds = Config::discover_worlds().unwrap_or_default();
            start_refresh(app);
        }
        Err(e) => app.push_log(format!("{} error: {:#}", action.label(), e)),
    }
}

async fn toggle_selected_setting(app: &mut App) {
    let Some(item) = app.settings.get(app.settings_selected) else {
        app.push_log("no setting selected");
        return;
    };
    if !settings::is_bool(item.def.kind) {
        app.push_log(format!("{} is not a boolean setting", item.def.key));
        return;
    }
    let key = item.def.key;
    app.push_log(format!("toggling {}...", key));
    match settings::toggle(&app.cfg, key).await {
        Ok(value) => {
            app.push_log(format!("{} toggled to {}", key, value));
            start_refresh(app);
        }
        Err(e) => app.push_log(format!("settings toggle error: {:#}", e)),
    }
}

fn selected_map(app: &App) -> Option<String> {
    app.snapshot
        .as_ref()?
        .maps
        .get(app.selected)
        .map(|m| m.name.clone())
}
