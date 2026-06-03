use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use dune_ctl_core::{
    backup,
    config::{Config, WorldProfile},
    health::HealthSnapshot,
    logs, maintenance, maps, settings, sietches, update,
};
use ratatui::{backend::Backend, Terminal};
use tokio::task::JoinHandle;

use super::ui;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const EVENT_TIMEOUT: Duration = Duration::from_millis(200);
type RefreshResult = Result<(HealthSnapshot, Vec<settings::SettingValue>)>;
type LogsResult = Result<Vec<String>>;
type BackupListResult = Result<Vec<backup::BackupEntry>>;

pub struct App {
    pub cfg: Config,
    pub started_at: Instant,
    pub snapshot: Option<HealthSnapshot>,
    pub settings: Vec<settings::SettingValue>,
    pub worlds: Vec<WorldProfile>,
    pub world_selected: usize,
    pub refresh_task: Option<JoinHandle<RefreshResult>>,
    pub logs_task: Option<JoinHandle<LogsResult>>,
    pub log: VecDeque<String>,
    pub log_lines: Vec<String>,
    pub logs_selected: usize,
    pub backup_entries: Vec<backup::BackupEntry>,
    pub backup_selected: usize,
    pub backup_schedule: Option<backup::ScheduleInfo>,
    pub backup_list_task: Option<JoinHandle<BackupListResult>>,
    pub backup_task: Option<JoinHandle<Result<()>>>,
    pub backup_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    pub backup_lines: Vec<String>,
    pub backup_list_loading: bool,
    pub update_task: Option<JoinHandle<Result<()>>>,
    pub update_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    pub update_lines: Vec<String>,
    pub shutdown_task: Option<JoinHandle<Result<()>>>,
    pub shutdown_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    pub shutdown_lines: Vec<String>,
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
    Logs,
    Backups,
    Sietches,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingAction {
    StartSietch,
    StopSietch,
    RestartSietch,
    ApplySettings,
    ApplySettingsAndRestart,
    PullDeployedSettings,
    InitWorldSettings,
    ClearSietchPassword,
    RunBackup,
    RunUpdate,
    CleanShutdown,
    DeleteBackup,
    RemoveSchedule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    SetSetting,
    SetBackupCron,
    SetBackupKeep,
}

#[derive(Debug, Clone)]
pub struct InputMode {
    pub key: String,
    pub label: String,
    pub value: String,
    pub action: InputAction,
}

impl PendingAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::StartSietch => "start world / primary sietch",
            Self::StopSietch => "stop world / primary sietch",
            Self::RestartSietch => "restart world / primary sietch",
            Self::ApplySettings => "deploy settings",
            Self::ApplySettingsAndRestart => "deploy settings and restart primary sietch",
            Self::PullDeployedSettings => "pull deployed settings to local",
            Self::InitWorldSettings => "initialize world settings profile",
            Self::ClearSietchPassword => "clear sietch password",
            Self::RunBackup => "run full backup",
            Self::RunUpdate => "update world and start after",
            Self::CleanShutdown => "cleanly shut down Dune for host reboot",
            Self::DeleteBackup => "delete backup bundle",
            Self::RemoveSchedule => "remove backup schedule",
        }
    }

    pub fn risk(self) -> &'static str {
        match self {
            Self::StartSietch => {
                "Starts the selected World/BattleGroup. Current self-hosting maps this to the primary Sietch start."
            }
            Self::StopSietch => {
                "Stops the selected World/BattleGroup through the primary Sietch lifecycle, disconnecting players."
            }
            Self::RestartSietch => {
                "Restarts the selected World/BattleGroup through the primary Sietch lifecycle. Gateway patch may need verification after rollout."
            }
            Self::ApplySettings => {
                "Copies local UserEngine.ini and UserGame.ini into /srv/UserSettings."
            }
            Self::ApplySettingsAndRestart => {
                "Copies local UserEngine.ini and UserGame.ini into /srv/UserSettings, then restarts the primary Sietch. Connected players will be disconnected."
            }
            Self::PullDeployedSettings => {
                "Replaces local UserEngine.ini and UserGame.ini with the deployed copies from /srv/UserSettings. Live server state is not changed."
            }
            Self::InitWorldSettings => {
                "Creates a per-world UserSettings profile. Future settings edits for this world will use that profile."
            }
            Self::ClearSietchPassword => {
                "Sets the local Sietch password to an empty string. Deploy settings to make it live."
            }
            Self::RunBackup => {
                "Runs dune-backup.sh: DB dump, k8s metadata, and settings snapshot. Takes 1–3 minutes. Output streams in the lower pane."
            }
            Self::RunUpdate => {
                "Runs the live capsule-aware update: backup, SteamCMD validate, image import, capsule refresh, apply, start, gateway patch, and readiness checks. Connected players will be disconnected."
            }
            Self::CleanShutdown => {
                "Runs the planned-maintenance sequence: full backup, BattleGroup stop, then waits until game servers are stopped. The host is not rebooted."
            }
            Self::DeleteBackup => {
                "Permanently deletes the selected backup bundle from disk. Cannot be undone."
            }
            Self::RemoveSchedule => {
                "Removes the nightly backup cron job. Existing backup data is not deleted."
            }
        }
    }
}

impl App {
    fn new(cfg: Config) -> Self {
        let worlds = Config::discover_worlds().unwrap_or_default();
        let world_selected = worlds
            .iter()
            .position(|world| world.battlegroup == cfg.battlegroup)
            .unwrap_or(0);
        Self {
            cfg,
            started_at: Instant::now(),
            snapshot: None,
            settings: Vec::new(),
            worlds,
            world_selected,
            refresh_task: None,
            logs_task: None,
            log: VecDeque::with_capacity(64),
            log_lines: Vec::new(),
            logs_selected: 0,
            backup_entries: Vec::new(),
            backup_selected: 0,
            backup_schedule: backup::read_schedule(),
            backup_list_task: None,
            backup_task: None,
            backup_rx: None,
            backup_lines: Vec::new(),
            backup_list_loading: false,
            update_task: None,
            update_rx: None,
            update_lines: Vec::new(),
            shutdown_task: None,
            shutdown_rx: None,
            shutdown_lines: Vec::new(),
            view: View::Dashboard,
            selected: 0,
            settings_selected: 0,
            pending: None,
            input: None,
            loading: true,
            running: true,
        }
    }

    pub fn selected_world(&self) -> Option<&WorldProfile> {
        self.worlds.get(self.world_selected)
    }

    pub fn retarget_world(&mut self, index: usize) -> bool {
        if self.update_task.is_some() || self.shutdown_task.is_some() {
            self.push_log("operation running; world retarget disabled");
            return false;
        }
        let Some(world) = self.worlds.get(index).cloned() else {
            return false;
        };
        if world.battlegroup == self.cfg.battlegroup {
            self.world_selected = index;
            return false;
        }

        self.cfg = Config::load(Some(&world.battlegroup)).unwrap_or_else(|_| self.cfg.clone());
        self.world_selected = index;
        self.snapshot = None;
        self.settings.clear();
        self.selected = 0;
        self.settings_selected = 0;
        self.logs_selected = 0;
        self.backup_selected = 0;
        self.log_lines.clear();
        self.backup_lines.clear();
        self.refresh_task = None;
        self.logs_task = None;
        self.backup_list_task = None;
        self.backup_task = None;
        self.backup_rx = None;
        self.shutdown_task = None;
        self.shutdown_rx = None;
        self.shutdown_lines.clear();
        self.loading = true;
        self.push_log(format!(
            "retargeted to {} / {}",
            world.title.as_deref().unwrap_or(&world.battlegroup),
            world.battlegroup
        ));
        self.push_target_log();
        true
    }

    pub fn push_log(&mut self, msg: impl Into<String>) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        let line = format!("{} {}", ts, msg.into());
        self.log.push_back(line);
        while self.log.len() > 64 {
            self.log.pop_front();
        }
    }

    fn push_target_log(&mut self) {
        self.push_log(format!(
            "target {} / {}",
            self.cfg
                .title
                .as_deref()
                .unwrap_or(self.cfg.battlegroup.as_str()),
            self.cfg.namespace
        ));
        self.push_log(format!(
            "settings {} ({})",
            self.cfg.user_settings_dir().display(),
            self.cfg.settings_profile_label()
        ));
    }
}

/// Returns the ordered list of log targets for the Logs view left pane.
/// Infra services always listed first, then running maps from the snapshot.
pub fn build_log_targets(app: &App) -> Vec<String> {
    let mut targets: Vec<String> = vec![
        "gateway".into(),
        "director".into(),
        "postgres".into(),
        "rabbitmq".into(),
        "filebrowser".into(),
        "text-router".into(),
    ];
    if let Some(snap) = &app.snapshot {
        for map in &snap.maps {
            if map.replicas > 0 {
                targets.push(map.name.clone());
            }
        }
    }
    targets
}

pub async fn run_loop<B: Backend>(terminal: &mut Terminal<B>, cfg: &Config) -> Result<()> {
    let mut app = App::new(cfg.clone());
    app.push_log("dune-ctl started");

    start_refresh(&mut app);
    let mut last_poll = Instant::now();

    while app.running {
        finish_refresh(&mut app).await;
        finish_logs_task(&mut app).await;
        finish_backup_list_task(&mut app).await;
        finish_backup_task(&mut app).await;
        finish_update_task(&mut app).await;
        finish_shutdown_task(&mut app).await;
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(EVENT_TIMEOUT)? {
            if let Event::Key(key) = event::read()? {
                handle_key(&mut app, key.code, key.modifiers).await;
            }
        }

        if last_poll.elapsed() >= POLL_INTERVAL {
            start_refresh(&mut app);
            if app.view == View::Logs {
                start_logs_refresh(&mut app);
            }
            if app.view == View::Backups {
                start_backup_list_refresh(&mut app);
            }
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

fn refresh_world_context(app: &mut App) {
    start_refresh(app);
    if app.view == View::Logs {
        app.logs_task = None;
        start_logs_refresh(app);
    }
    if app.view == View::Backups {
        app.backup_list_task = None;
        start_backup_list_refresh(app);
    }
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
            // Clamp logs_selected after snapshot update (target list may have changed)
            let log_count = build_log_targets(app).len();
            if log_count > 0 && app.logs_selected >= log_count {
                app.logs_selected = log_count - 1;
            }
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

fn start_logs_refresh(app: &mut App) {
    if app.logs_task.is_some() {
        return;
    }
    let targets = build_log_targets(app);
    let target = match targets.get(app.logs_selected) {
        Some(t) => t.clone(),
        None => return,
    };
    let cfg = app.cfg.clone();
    app.logs_task = Some(tokio::spawn(
        async move { logs::tail(&cfg, &target, 150).await },
    ));
}

async fn finish_logs_task(app: &mut App) {
    if !app
        .logs_task
        .as_ref()
        .map(|t| t.is_finished())
        .unwrap_or(false)
    {
        return;
    }
    let Some(task) = app.logs_task.take() else {
        return;
    };
    match task.await {
        Ok(Ok(lines)) => app.log_lines = lines,
        Ok(Err(e)) => app.log_lines = vec![format!("error: {:#}", e)],
        Err(e) => app.log_lines = vec![format!("task error: {}", e)],
    }
}

fn start_backup_list_refresh(app: &mut App) {
    if app.backup_list_task.is_some() {
        return;
    }
    app.backup_list_loading = true;
    let cfg = app.cfg.clone();
    app.backup_list_task = Some(tokio::spawn(async move { backup::list(&cfg).await }));
}

async fn finish_backup_list_task(app: &mut App) {
    if !app
        .backup_list_task
        .as_ref()
        .map(|t| t.is_finished())
        .unwrap_or(false)
    {
        return;
    }
    let Some(task) = app.backup_list_task.take() else {
        return;
    };
    match task.await {
        Ok(Ok(entries)) => {
            app.backup_entries = entries;
            if app.backup_selected >= app.backup_entries.len() {
                app.backup_selected = app.backup_entries.len().saturating_sub(1);
            }
            app.backup_list_loading = false;
        }
        Ok(Err(e)) => {
            app.push_log(format!("backup list error: {:#}", e));
            app.backup_list_loading = false;
        }
        Err(e) => {
            app.push_log(format!("backup list task error: {}", e));
            app.backup_list_loading = false;
        }
    }
}

pub fn start_backup_run(app: &mut App) {
    if app.backup_task.is_some() {
        return;
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    app.backup_rx = Some(rx);
    app.backup_lines.clear();
    let cfg = app.cfg.clone();
    app.backup_task = Some(tokio::spawn(async move {
        backup::run_streamed(&cfg, false, None, tx).await
    }));
    app.push_log("backup run started");
}

pub fn start_update_run(app: &mut App) {
    if app.update_task.is_some() {
        app.push_log("update already running");
        return;
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    app.update_rx = Some(rx);
    app.update_lines.clear();
    let cfg = app.cfg.clone();
    app.update_task = Some(tokio::spawn(async move {
        update::run_streamed(&cfg, update::UpdateOptions { start_after: true }, tx).await
    }));
    app.push_log("update run started");
    app.push_log("update will refresh/apply the capsule and verify readiness");
}

pub fn start_clean_shutdown(app: &mut App) {
    if app.shutdown_task.is_some() {
        app.push_log("clean shutdown already running");
        return;
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    app.shutdown_rx = Some(rx);
    app.shutdown_lines.clear();
    let cfg = app.cfg.clone();
    app.shutdown_task = Some(tokio::spawn(async move {
        maintenance::shutdown_for_reboot_streamed(&cfg, maintenance::ShutdownOptions::default(), tx)
            .await
    }));
    app.push_log("clean shutdown started");
    app.push_log("shutdown will back up, stop BattleGroup, and wait for game servers");
}

async fn finish_backup_task(app: &mut App) {
    // Drain any new output lines each tick regardless of task completion
    if let Some(rx) = app.backup_rx.as_mut() {
        while let Ok(line) = rx.try_recv() {
            app.backup_lines.push(line);
        }
    }
    if !app
        .backup_task
        .as_ref()
        .map(|t| t.is_finished())
        .unwrap_or(false)
    {
        return;
    }
    // Final drain after task finishes
    if let Some(rx) = app.backup_rx.as_mut() {
        while let Ok(line) = rx.try_recv() {
            app.backup_lines.push(line);
        }
    }
    let Some(task) = app.backup_task.take() else {
        return;
    };
    app.backup_rx = None;
    match task.await {
        Ok(Ok(())) => {
            app.push_log("backup complete");
            start_backup_list_refresh(app);
        }
        Ok(Err(e)) => app.push_log(format!("backup error: {:#}", e)),
        Err(e) => app.push_log(format!("backup task error: {}", e)),
    }
}

async fn finish_update_task(app: &mut App) {
    if let Some(rx) = app.update_rx.as_mut() {
        while let Ok(line) = rx.try_recv() {
            app.update_lines.push(line);
        }
    }
    if !app
        .update_task
        .as_ref()
        .map(|t| t.is_finished())
        .unwrap_or(false)
    {
        return;
    }
    if let Some(rx) = app.update_rx.as_mut() {
        while let Ok(line) = rx.try_recv() {
            app.update_lines.push(line);
        }
    }
    let Some(task) = app.update_task.take() else {
        return;
    };
    app.update_rx = None;
    match task.await {
        Ok(Ok(())) => {
            app.push_log("update complete");
            app.push_log("follow-up: verify gateway IP and server browser");
            app.worlds = Config::discover_worlds().unwrap_or_default();
            start_refresh(app);
        }
        Ok(Err(e)) => app.push_log(format!("update error: {:#}", e)),
        Err(e) => app.push_log(format!("update task error: {}", e)),
    }
}

async fn finish_shutdown_task(app: &mut App) {
    if let Some(rx) = app.shutdown_rx.as_mut() {
        while let Ok(line) = rx.try_recv() {
            app.shutdown_lines.push(line);
        }
    }
    if !app
        .shutdown_task
        .as_ref()
        .map(|t| t.is_finished())
        .unwrap_or(false)
    {
        return;
    }
    if let Some(rx) = app.shutdown_rx.as_mut() {
        while let Ok(line) = rx.try_recv() {
            app.shutdown_lines.push(line);
        }
    }
    let Some(task) = app.shutdown_task.take() else {
        return;
    };
    app.shutdown_rx = None;
    match task.await {
        Ok(Ok(())) => {
            app.push_log("clean shutdown complete");
            app.push_log("host reboot can be run outside dune-ctl");
            start_refresh(app);
            start_backup_list_refresh(app);
        }
        Ok(Err(e)) => app.push_log(format!("clean shutdown error: {:#}", e)),
        Err(e) => app.push_log(format!("clean shutdown task error: {}", e)),
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

    if app.update_task.is_some() || app.shutdown_task.is_some() {
        let op = if app.shutdown_task.is_some() {
            "clean shutdown"
        } else {
            "update"
        };
        match code {
            KeyCode::Char('q') | KeyCode::Esc => {
                app.push_log(format!("{} running; quit disabled until it finishes", op));
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                app.push_log(format!(
                    "{} running; interrupt disabled until it finishes",
                    op
                ));
            }
            KeyCode::Tab => {
                app.view = match app.view {
                    View::Worlds => View::Dashboard,
                    View::Dashboard => View::Maps,
                    View::Maps => View::Settings,
                    View::Settings => View::Logs,
                    View::Logs => View::Backups,
                    View::Backups => View::Sietches,
                    View::Sietches => View::Worlds,
                };
            }
            KeyCode::Char('1') => app.view = View::Worlds,
            KeyCode::Char('2') => app.view = View::Dashboard,
            KeyCode::Char('3') => app.view = View::Maps,
            KeyCode::Char('4') => app.view = View::Settings,
            KeyCode::Char('5') => app.view = View::Logs,
            KeyCode::Char('6') => app.view = View::Backups,
            KeyCode::Char('7') => app.view = View::Sietches,
            KeyCode::Char('r') => {
                app.push_log("refreshing...");
                refresh_world_context(app);
            }
            _ => app.push_log(format!("{} running; action disabled until it finishes", op)),
        }
        return;
    }

    let map_count = app.snapshot.as_ref().map(|s| s.maps.len()).unwrap_or(0);
    let log_target_count = build_log_targets(app).len();
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
                View::Settings => View::Logs,
                View::Logs => View::Backups,
                View::Backups => View::Sietches,
                View::Sietches => View::Worlds,
            };
            if app.view == View::Logs {
                start_logs_refresh(app);
            }
            if app.view == View::Backups {
                start_backup_list_refresh(app);
            }
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
        KeyCode::Char('5') => {
            app.view = View::Logs;
            start_logs_refresh(app);
        }
        KeyCode::Char('6') => {
            app.view = View::Backups;
            start_backup_list_refresh(app);
        }
        KeyCode::Char('7') => {
            app.view = View::Sietches;
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
        KeyCode::Char('u') => {
            if app.view == View::Dashboard {
                if app.update_task.is_some() {
                    app.push_log("update already running");
                } else if app.backup_task.is_some() {
                    app.push_log("backup running; wait before starting update");
                } else {
                    app.pending = Some(PendingAction::RunUpdate);
                }
            }
        }
        KeyCode::Char('Q') => {
            if app.view == View::Dashboard {
                if app.backup_task.is_some() {
                    app.push_log("backup running; wait before clean shutdown");
                } else {
                    app.pending = Some(PendingAction::CleanShutdown);
                }
            }
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
            View::Worlds if !app.worlds.is_empty() => {
                let next = (app.world_selected + 1) % app.worlds.len();
                if app.retarget_world(next) {
                    refresh_world_context(app);
                }
            }
            View::Logs if log_target_count > 0 => {
                app.logs_selected = (app.logs_selected + 1) % log_target_count;
                app.logs_task = None;
                start_logs_refresh(app);
            }
            View::Settings if !app.settings.is_empty() => {
                app.settings_selected = (app.settings_selected + 1) % app.settings.len();
            }
            View::Backups if !app.backup_entries.is_empty() => {
                app.backup_selected = (app.backup_selected + 1).min(app.backup_entries.len() - 1);
            }
            _ if map_count > 0 => {
                app.view = View::Maps;
                app.selected = (app.selected + 1) % map_count;
            }
            _ => {}
        },
        KeyCode::Up | KeyCode::Char('k') => match app.view {
            View::Worlds if !app.worlds.is_empty() => {
                let next = if app.world_selected == 0 {
                    app.worlds.len() - 1
                } else {
                    app.world_selected - 1
                };
                if app.retarget_world(next) {
                    refresh_world_context(app);
                }
            }
            View::Logs if log_target_count > 0 && app.logs_selected > 0 => {
                app.logs_selected -= 1;
                app.logs_task = None;
                start_logs_refresh(app);
            }
            View::Settings if !app.settings.is_empty() && app.settings_selected > 0 => {
                app.settings_selected -= 1;
            }
            View::Backups if app.backup_selected > 0 => {
                app.backup_selected -= 1;
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
            } else if app.view == View::Backups {
                begin_backup_cron_edit(app);
            }
        }
        KeyCode::Char('K') => {
            if app.view == View::Backups {
                begin_backup_keep_edit(app);
            }
        }
        KeyCode::Char('X') => {
            if app.view == View::Backups && app.backup_schedule.is_some() {
                app.pending = Some(PendingAction::RemoveSchedule);
            }
        }
        KeyCode::Char('d') => {
            if app.view == View::Backups
                && !app.backup_entries.is_empty()
                && app.backup_task.is_none()
            {
                app.pending = Some(PendingAction::DeleteBackup);
            }
        }
        KeyCode::Char('a') => {
            if app.view == View::Settings {
                app.pending = Some(PendingAction::ApplySettings);
            }
        }
        KeyCode::Char('D') => {
            if app.view == View::Settings {
                app.pending = Some(PendingAction::ApplySettingsAndRestart);
            }
        }
        KeyCode::Char('U') => {
            if app.view == View::Settings {
                app.pending = Some(PendingAction::PullDeployedSettings);
            }
        }
        KeyCode::Char('s') => {
            if let Some(name) = selected_map(app) {
                app.push_log(format!("starting {}...", name));
                match maps::start(&app.cfg, &name, false).await {
                    Ok(()) => {
                        app.push_log(format!("{}: start triggered", name));
                        app.push_target_log();
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
                        app.push_target_log();
                        start_refresh(app);
                    }
                    Err(e) => app.push_log(format!("stop error: {:#}", e)),
                }
            }
        }
        KeyCode::Char('r') => {
            if app.view == View::Backups && app.backup_task.is_none() {
                app.pending = Some(PendingAction::RunBackup);
            } else {
                app.push_log("refreshing...");
                refresh_world_context(app);
            }
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
            match input.action {
                InputAction::SetSetting => {
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
                InputAction::SetBackupCron => {
                    let keep = app.backup_schedule.as_ref().map(|s| s.keep).unwrap_or(14);
                    let bin = current_exe_path();
                    match backup::write_schedule(&app.cfg.battlegroup, &bin, &input.value, keep) {
                        Ok(()) => {
                            app.backup_schedule = backup::read_schedule();
                            app.push_log(format!("schedule cron set to {}", input.value));
                        }
                        Err(e) => {
                            app.push_log(format!("schedule error: {:#}", e));
                            app.input = Some(input);
                        }
                    }
                }
                InputAction::SetBackupKeep => match input.value.trim().parse::<usize>() {
                    Ok(keep) => {
                        let cron = app
                            .backup_schedule
                            .as_ref()
                            .map(|s| s.cron.clone())
                            .unwrap_or_else(|| "0 3 * * *".to_string());
                        let bin = current_exe_path();
                        match backup::write_schedule(&app.cfg.battlegroup, &bin, &cron, keep) {
                            Ok(()) => {
                                app.backup_schedule = backup::read_schedule();
                                app.push_log(format!("schedule keep set to {}", keep));
                            }
                            Err(e) => {
                                app.push_log(format!("schedule error: {:#}", e));
                                app.input = Some(input);
                            }
                        }
                    }
                    Err(_) => {
                        app.push_log("keep must be a whole number (0 = no pruning)");
                        app.input = Some(input);
                    }
                },
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
        action: InputAction::SetSetting,
    });
}

fn begin_backup_cron_edit(app: &mut App) {
    let current = app
        .backup_schedule
        .as_ref()
        .map(|s| s.cron.clone())
        .unwrap_or_else(|| "0 3 * * *".to_string());
    app.input = Some(InputMode {
        key: "backup_cron".to_string(),
        label:
            "Cron schedule (e.g. '0 3 * * *' = daily 3am, '0 */6 * * *' = every 6h). Enter to save."
                .to_string(),
        value: current,
        action: InputAction::SetBackupCron,
    });
}

fn begin_backup_keep_edit(app: &mut App) {
    let current = app
        .backup_schedule
        .as_ref()
        .map(|s| s.keep.to_string())
        .unwrap_or_else(|| "14".to_string());
    app.input = Some(InputMode {
        key: "backup_keep".to_string(),
        label: "Number of bundles to retain (0 = no pruning). Enter to save.".to_string(),
        value: current,
        action: InputAction::SetBackupKeep,
    });
}

fn current_exe_path() -> String {
    std::env::current_exe()
        .unwrap_or_else(|_| {
            std::path::PathBuf::from("/home/dune/dune-server/dune-ctl/target/release/dune-ctl")
        })
        .to_string_lossy()
        .into_owned()
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
        PendingAction::ApplySettingsAndRestart => match settings::apply(&app.cfg).await {
            Ok(()) => sietches::restart_primary(&app.cfg).await,
            Err(e) => Err(e),
        },
        PendingAction::PullDeployedSettings => settings::pull_deployed(&app.cfg).await,
        PendingAction::InitWorldSettings => app.cfg.init_world_settings().map(|_| ()),
        PendingAction::ClearSietchPassword => settings::set(&app.cfg, "sietch_password", "").await,
        PendingAction::RunBackup => {
            start_backup_run(app);
            Ok(())
        }
        PendingAction::RunUpdate => {
            start_update_run(app);
            Ok(())
        }
        PendingAction::CleanShutdown => {
            start_clean_shutdown(app);
            Ok(())
        }
        PendingAction::DeleteBackup => {
            if let Some(entry) = app.backup_entries.get(app.backup_selected) {
                let path = entry.path.clone();
                backup::delete_bundle(&path).await
            } else {
                Ok(())
            }
        }
        PendingAction::RemoveSchedule => backup::remove_schedule(),
    };
    match result {
        Ok(()) => {
            app.push_log(format!("{} triggered", action.label()));
            // backup-specific follow-up (no cluster refresh needed)
            if action == PendingAction::DeleteBackup {
                if app.backup_selected > 0
                    && app.backup_selected >= app.backup_entries.len().saturating_sub(1)
                {
                    app.backup_selected = app.backup_selected.saturating_sub(1);
                }
                start_backup_list_refresh(app);
                return;
            }
            if action == PendingAction::RemoveSchedule {
                app.backup_schedule = backup::read_schedule();
                return;
            }
            if matches!(
                action,
                PendingAction::RunUpdate | PendingAction::CleanShutdown
            ) {
                return;
            }
            app.push_target_log();
            if matches!(
                action,
                PendingAction::RestartSietch | PendingAction::ApplySettingsAndRestart
            ) {
                app.push_log("follow-up: verify gateway patch and server browser");
            } else if action == PendingAction::ApplySettings {
                app.push_log("follow-up: restart primary Sietch if settings require it");
            }
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
