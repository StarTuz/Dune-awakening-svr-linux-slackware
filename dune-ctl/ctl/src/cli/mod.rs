use anyhow::{Context, Result};
use clap::Subcommand;
use dune_ctl_core::{
    backup, battlegroup,
    config::{Config, WorldProfile},
    diagnostics::CheckState,
    fls::FlsTokenState,
    gateway,
    health::HealthSnapshot,
    logs, maps, players, settings, sietches, update,
};

#[derive(Subcommand)]
pub enum Command {
    /// List locally known worlds/BattleGroups
    Worlds {
        #[command(subcommand)]
        action: WorldsCommand,
    },
    /// Show battlegroup status, map phases, FLS token, and RAM
    Status,
    /// Run a compact go/no-go preflight for the selected world
    Preflight {
        /// Treat warnings as failures
        #[arg(long)]
        strict: bool,
    },
    /// Map management
    Maps {
        #[command(subcommand)]
        action: MapsCommand,
    },
    /// Sietch view for the selected world
    Sietches {
        #[command(subcommand)]
        action: SietchesCommand,
    },
    /// Battlegroup lifecycle management
    Battlegroup {
        #[command(subcommand)]
        action: BattlegroupCommand,
    },
    /// Inspect and edit local UserEngine.ini/UserGame.ini settings
    Settings {
        #[command(subcommand)]
        action: SettingsCommand,
    },
    /// Run local deployment diagnostics
    Diagnostics,
    /// Run full update pipeline (steamcmd + funcom-patches + gateway-patch)
    Update,
    /// Re-apply --RMQGameHttpPort=30196 to the gateway Deployment
    GatewayPatch,
    /// Check FLS token expiry; exits non-zero if critical or expired
    TokenCheck,
    /// Create or restore database backups
    Backup {
        #[command(subcommand)]
        action: BackupCommand,
    },
    /// Stream or show logs for a map or infrastructure pod
    Logs {
        /// Map name (e.g. Survival_1) or infra alias: gateway, director, postgres,
        /// rabbitmq, filebrowser, text-router
        target: String,
        /// Follow log output (stream until Ctrl-C)
        #[arg(short, long)]
        follow: bool,
        /// Number of recent lines to show (default 100)
        #[arg(long, default_value = "100")]
        tail: usize,
    },
    /// Show currently online players
    Players,
    /// Start the web interface (requires --features web)
    Web {
        #[arg(long, default_value = "9090")]
        port: u16,
    },
}

#[derive(Subcommand)]
pub enum WorldsCommand {
    /// List ~/.dune world specs that dune-ctl can target
    List,
    /// Create a per-world local UserSettings profile for the selected world
    InitSettings,
}

#[derive(Subcommand)]
pub enum MapsCommand {
    /// List all 28 maps with current phase
    List,
    /// Start a stopped map
    Start {
        name: String,
        /// Override the social-hub guard (SH_* maps are director-managed)
        #[arg(long)]
        force: bool,
    },
    /// Stop a running map
    Stop { name: String },
}

#[derive(Subcommand)]
pub enum SietchesCommand {
    /// List known Sietches in the selected world
    List,
    /// Start the selected world's primary Sietch
    Start,
    /// Stop the selected world's primary Sietch
    Stop,
    /// Restart the selected world's primary Sietch
    Restart,
}

#[derive(Subcommand)]
pub enum BattlegroupCommand {
    /// Start the battlegroup
    Start,
    /// Stop the battlegroup
    Stop,
    /// Restart the battlegroup
    Restart,
}

#[derive(Subcommand)]
pub enum BackupCommand {
    /// List available backup bundles for the current battlegroup
    List,
    /// Create a full backup bundle (DB dump + k8s metadata + settings)
    Run {
        /// Skip the database dump; capture metadata and settings only
        #[arg(long)]
        skip_db: bool,
        /// Database backup filename (default: <bg>-<timestamp>.backup)
        #[arg(long)]
        name: Option<String>,
        /// Keep only the N most recent bundles after a successful run (0 = no pruning)
        #[arg(long, default_value = "0")]
        keep: usize,
    },
    /// Restore from a backup bundle (requires --yes)
    Restore {
        /// Bundle timestamp (e.g. 20260517-142305) or full path
        bundle: String,
        /// Confirm the restore without interactive prompt
        #[arg(long)]
        yes: bool,
    },
    /// Manage the automated nightly backup schedule
    Schedule {
        /// Print the installed schedule without modifying it
        #[arg(long)]
        show: bool,
        /// Remove the scheduled backup job
        #[arg(long)]
        remove: bool,
        /// Cron schedule expression (default: 3am daily)
        #[arg(long, default_value = "0 3 * * *")]
        cron: String,
        /// Bundles to retain when the scheduled job runs
        #[arg(long, default_value = "14")]
        keep: usize,
    },
}

#[derive(Subcommand)]
pub enum SettingsCommand {
    /// List managed settings and current local values
    List,
    /// Set a managed setting in the local config files
    Set { key: String, value: String },
    /// Toggle a boolean managed setting in the local config files
    Toggle { key: String },
    /// Summarize local-vs-deployed setting drift
    Status,
    /// Compare local config files to the deployed UserSettings copy
    Diff,
    /// Replace local UserEngine.ini/UserGame.ini with the deployed copies
    Pull,
    /// Deploy local UserEngine.ini/UserGame.ini to the filebrowser UserSettings path
    Apply,
    /// Deploy local settings, then restart the selected world's primary Sietch
    ApplyRestart,
}

pub async fn run(cmd: Command, cfg: &Config) -> Result<()> {
    match cmd {
        Command::Worlds { action } => cmd_worlds(action, cfg).await,
        Command::Status => cmd_status(cfg).await,
        Command::Preflight { strict } => cmd_preflight(cfg, strict).await,
        Command::Maps { action } => cmd_maps(action, cfg).await,
        Command::Sietches { action } => cmd_sietches(action, cfg).await,
        Command::Battlegroup { action } => cmd_battlegroup(action, cfg).await,
        Command::Settings { action } => cmd_settings(action, cfg).await,
        Command::Diagnostics => cmd_diagnostics(cfg).await,
        Command::Update => cmd_update(cfg).await,
        Command::GatewayPatch => cmd_gateway_patch(cfg).await,
        Command::TokenCheck => cmd_token_check(cfg).await,
        Command::Backup { action } => cmd_backup(action, cfg).await,
        Command::Logs {
            target,
            follow,
            tail,
        } => cmd_logs(cfg, &target, follow, tail).await,
        Command::Players => cmd_players(cfg).await,
        Command::Web { port } => cmd_web(port, cfg).await,
    }
}

async fn cmd_worlds(action: WorldsCommand, cfg: &Config) -> Result<()> {
    match action {
        WorldsCommand::List => {
            let worlds = Config::discover_worlds()?;
            if worlds.is_empty() {
                println!("No world specs found in ~/.dune.");
                return Ok(());
            }
            println!(
                "{:<3} {:<30} {:<22} {:<9} Spec",
                "", "Battlegroup", "Title", "Settings"
            );
            println!("{}", "-".repeat(102));
            for world in worlds {
                print_world_row(cfg, &world);
            }
        }
        WorldsCommand::InitSettings => {
            let dir = cfg.init_world_settings()?;
            println!("World settings profile ready: {}", dir.display());
            println!(
                "settings commands now use this profile for {}",
                cfg.battlegroup
            );
            print_target_summary(cfg);
        }
    }
    Ok(())
}

fn print_world_row(cfg: &Config, world: &WorldProfile) {
    let active = if world.battlegroup == cfg.battlegroup {
        "*"
    } else {
        ""
    };
    println!(
        "{:<3} {:<30} {:<22} {:<9} {}",
        active,
        world.battlegroup,
        world.title.as_deref().unwrap_or("—"),
        if world_settings_dir(&world.battlegroup).exists() {
            "profile"
        } else {
            "shared"
        },
        world.spec_path.display()
    );
}

fn world_settings_dir(battlegroup: &str) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/dune".into());
    std::path::PathBuf::from(home)
        .join(".dune")
        .join("worlds")
        .join(battlegroup)
        .join("UserSettings")
}

async fn cmd_sietches(action: SietchesCommand, cfg: &Config) -> Result<()> {
    match action {
        SietchesCommand::List => {
            let snap = HealthSnapshot::collect(cfg).await?;
            println!(
                "{:<18} {:<14} {:<12} {:<8} {:<8} {:<6} State",
                "Sietch", "Map", "Phase", "Ready", "Players", "Port"
            );
            println!("{}", "-".repeat(82));
            for sietch in &snap.sietches {
                println!(
                    "{:<18} {:<14} {:<12} {:<8} {:<8} {:<6} {}",
                    if sietch.primary {
                        "Primary Sietch"
                    } else {
                        &sietch.name
                    },
                    sietch.map,
                    sietch.phase,
                    format!(
                        "{}/{}",
                        opt_u32(sietch.ready_replicas),
                        opt_u32(sietch.target_replicas)
                    ),
                    opt_u32(sietch.players),
                    opt_u16(sietch.game_port),
                    sietch.consistency.label()
                );
            }
        }
        SietchesCommand::Start => {
            sietches::start_primary(cfg).await?;
            println!(
                "Primary Sietch start triggered for {}.",
                selected_world_label(cfg)
            );
            print_target_summary(cfg);
        }
        SietchesCommand::Stop => {
            sietches::stop_primary(cfg).await?;
            println!(
                "Primary Sietch stop triggered for {}.",
                selected_world_label(cfg)
            );
            print_target_summary(cfg);
        }
        SietchesCommand::Restart => {
            sietches::restart_primary(cfg).await?;
            println!(
                "Primary Sietch restart triggered for {}.",
                selected_world_label(cfg)
            );
            print_target_summary(cfg);
            println!("Follow-up   : verify gateway patch and server browser after rollout.");
        }
    }
    Ok(())
}

fn selected_world_label(cfg: &Config) -> String {
    cfg.title.as_deref().unwrap_or(&cfg.battlegroup).to_string()
}

fn print_target_summary(cfg: &Config) {
    println!("World       : {}", selected_world_label(cfg));
    println!("Battlegroup : {}", cfg.battlegroup);
    println!("Namespace   : {}", cfg.namespace);
    println!(
        "Settings    : {} ({})",
        cfg.user_settings_dir().display(),
        cfg.settings_profile_label()
    );
}

async fn cmd_settings(action: SettingsCommand, cfg: &Config) -> Result<()> {
    match action {
        SettingsCommand::List => {
            let values = settings::list(cfg).await?;
            println!(
                "{:<28} {:<12} {:<8} {:<6} Label",
                "Key", "Value", "File", "Type"
            );
            println!("{}", "-".repeat(78));
            for item in values {
                println!(
                    "{:<28} {:<12} {:<8} {:<6} {}",
                    item.def.key,
                    settings::display_value(&item),
                    item.def.file.label(),
                    settings::kind_label(item.def.kind),
                    item.def.label
                );
            }
        }
        SettingsCommand::Set { key, value } => {
            settings::set(cfg, &key, &value).await?;
            println!(
                "{} updated locally. Run `dune-ctl settings apply` to deploy.",
                key
            );
            print_target_summary(cfg);
        }
        SettingsCommand::Toggle { key } => {
            let value = settings::toggle(cfg, &key).await?;
            println!(
                "{} toggled to {} locally. Run `dune-ctl settings apply` to deploy.",
                key, value
            );
            print_target_summary(cfg);
        }
        SettingsCommand::Status => {
            let drift = settings::drift(cfg).await?;
            let changed = drift.changed_count();
            if drift.deployed_available {
                println!("Settings drift: {} changed managed setting(s).", changed);
            } else {
                println!("Settings drift: deployed settings unavailable.");
                if let Some(error) = &drift.error {
                    println!("Reason: {}", error);
                }
            }
            println!("{:<28} {:<12} {:<12} State", "Key", "Local", "Deployed");
            println!("{}", "-".repeat(72));
            for item in drift.items.iter().filter(|item| item.changed()) {
                println!(
                    "{:<28} {:<12} {:<12} {}",
                    item.def.key,
                    settings::display_drift_local(item),
                    settings::display_drift_deployed(item),
                    if item.changed() { "changed" } else { "clean" }
                );
            }
            if changed == 0 && drift.deployed_available {
                println!("No managed setting drift detected.");
            }
        }
        SettingsCommand::Diff => {
            print!("{}", settings::diff(cfg).await?);
        }
        SettingsCommand::Pull => {
            settings::pull_deployed(cfg).await?;
            println!(
                "Deployed UserEngine.ini and UserGame.ini copied into local settings profile."
            );
            print_target_summary(cfg);
        }
        SettingsCommand::Apply => {
            settings::apply(cfg).await?;
            println!("UserEngine.ini and UserGame.ini deployed to /srv/UserSettings.");
            print_target_summary(cfg);
            println!("Follow-up   : restart primary Sietch if the changed settings require it.");
        }
        SettingsCommand::ApplyRestart => {
            settings::apply(cfg).await?;
            sietches::restart_primary(cfg).await?;
            println!(
                "UserEngine.ini and UserGame.ini deployed; primary Sietch restart triggered for {}.",
                selected_world_label(cfg)
            );
            print_target_summary(cfg);
            println!("Follow-up   : verify gateway patch and server browser after rollout.");
        }
    }
    println!("Local settings: {}", cfg.user_settings_dir().display());
    Ok(())
}

async fn cmd_battlegroup(action: BattlegroupCommand, cfg: &Config) -> Result<()> {
    match action {
        BattlegroupCommand::Start => {
            battlegroup::start(cfg).await?;
            println!("Battlegroup start triggered.");
            print_target_summary(cfg);
        }
        BattlegroupCommand::Stop => {
            battlegroup::stop(cfg).await?;
            println!("Battlegroup stop triggered.");
            print_target_summary(cfg);
        }
        BattlegroupCommand::Restart => {
            battlegroup::restart(cfg).await?;
            println!("Battlegroup restart triggered.");
            print_target_summary(cfg);
            println!("Follow-up   : verify gateway patch and server browser after rollout.");
        }
    }
    Ok(())
}

async fn cmd_diagnostics(cfg: &Config) -> Result<()> {
    let snap = HealthSnapshot::collect(cfg).await?;
    println!("Diagnostics for {}", cfg.battlegroup);
    print_check("firewall backend", &snap.diagnostics.firewall_backend);
    if let Some(gw) = &snap.gateway {
        println!(
            "{:<22} {}",
            "gateway patch",
            if gw.patched { "ok" } else { "missing" }
        );
    }
    Ok(())
}

fn print_check(label: &str, check: &dune_ctl_core::diagnostics::Check) {
    let prefix = match check.state {
        CheckState::Ok => "ok",
        CheckState::Warning => "warn",
        CheckState::Critical => "critical",
        CheckState::Unknown => "unknown",
    };
    println!("{:<22} {:<8} {}", label, prefix, check.message);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreflightState {
    Ok,
    Warn,
    Fail,
}

impl PreflightState {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

struct PreflightRow {
    label: &'static str,
    state: PreflightState,
    message: String,
}

impl PreflightRow {
    fn ok(label: &'static str, message: impl Into<String>) -> Self {
        Self {
            label,
            state: PreflightState::Ok,
            message: message.into(),
        }
    }

    fn warn(label: &'static str, message: impl Into<String>) -> Self {
        Self {
            label,
            state: PreflightState::Warn,
            message: message.into(),
        }
    }

    fn fail(label: &'static str, message: impl Into<String>) -> Self {
        Self {
            label,
            state: PreflightState::Fail,
            message: message.into(),
        }
    }
}

async fn cmd_preflight(cfg: &Config, strict: bool) -> Result<()> {
    let snap = HealthSnapshot::collect(cfg).await?;
    let mut rows = Vec::new();

    rows.push(match snap.diagnostics.firewall_backend.state {
        CheckState::Ok => PreflightRow::ok(
            "firewall backend",
            &snap.diagnostics.firewall_backend.message,
        ),
        CheckState::Warning => PreflightRow::warn(
            "firewall backend",
            &snap.diagnostics.firewall_backend.message,
        ),
        CheckState::Critical => PreflightRow::fail(
            "firewall backend",
            &snap.diagnostics.firewall_backend.message,
        ),
        CheckState::Unknown => PreflightRow::warn(
            "firewall backend",
            &snap.diagnostics.firewall_backend.message,
        ),
    });

    rows.push(match &snap.gateway {
        Some(gateway) if gateway.patched => {
            PreflightRow::ok("gateway patch", "--RMQGameHttpPort=30196 present")
        }
        Some(_) => PreflightRow::fail("gateway patch", "--RMQGameHttpPort=30196 missing"),
        None => PreflightRow::warn("gateway patch", "gateway deployment status unavailable"),
    });

    rows.push(match &snap.fls {
        Some(fls) => match fls.state {
            FlsTokenState::Ok => PreflightRow::ok(
                "FLS token",
                format!(
                    "{}; expires {}",
                    fls.label(),
                    fls.expires_at.format("%Y-%m-%d")
                ),
            ),
            FlsTokenState::WarningSoon => PreflightRow::warn(
                "FLS token",
                format!(
                    "{}; expires {}",
                    fls.label(),
                    fls.expires_at.format("%Y-%m-%d")
                ),
            ),
            FlsTokenState::Critical | FlsTokenState::Expired => PreflightRow::fail(
                "FLS token",
                format!(
                    "{}; expires {}",
                    fls.label(),
                    fls.expires_at.format("%Y-%m-%d")
                ),
            ),
        },
        None => PreflightRow::warn("FLS token", "token status unavailable"),
    });

    rows.push(match snap.sietches.iter().find(|sietch| sietch.primary) {
        Some(sietch)
            if sietch.phase == "Running"
                && sietch.ready_replicas.unwrap_or_default() > 0
                && sietch.consistency == battlegroup::MapConsistency::CleanOn =>
        {
            PreflightRow::ok(
                "primary Sietch",
                format!(
                    "{} running ready {}/{}",
                    sietch.map,
                    opt_u32(sietch.ready_replicas),
                    opt_u32(sietch.target_replicas)
                ),
            )
        }
        Some(sietch) if snap.battlegroup_stopped => PreflightRow::warn(
            "primary Sietch",
            format!("battlegroup stopped; {} phase {}", sietch.map, sietch.phase),
        ),
        Some(sietch) => PreflightRow::warn(
            "primary Sietch",
            format!(
                "{} phase {} ready {}/{} state {}",
                sietch.map,
                sietch.phase,
                opt_u32(sietch.ready_replicas),
                opt_u32(sietch.target_replicas),
                sietch.consistency.label()
            ),
        ),
        None => PreflightRow::fail("primary Sietch", "primary Sietch not found"),
    });

    rows.push(match (snap.ram_used_bytes, snap.ram_total_bytes) {
        (Some(used), Some(total)) => {
            let pct = (used as f64 / total as f64) * 100.0;
            let message = format!(
                "{:.1}/{:.1} GB used ({:.0}%)",
                used as f64 / 1e9,
                total as f64 / 1e9,
                pct
            );
            if pct >= 95.0 {
                PreflightRow::warn("RAM", message)
            } else {
                PreflightRow::ok("RAM", message)
            }
        }
        _ => PreflightRow::warn("RAM", "memory status unavailable"),
    });

    println!("Preflight for {}", selected_world_label(cfg));
    println!("Battlegroup : {}", cfg.battlegroup);
    println!("Namespace   : {}", cfg.namespace);
    println!();
    for row in &rows {
        println!("{:<16} {:<5} {}", row.label, row.state.label(), row.message);
    }

    let fail_count = rows
        .iter()
        .filter(|row| row.state == PreflightState::Fail)
        .count();
    let warn_count = rows
        .iter()
        .filter(|row| row.state == PreflightState::Warn)
        .count();

    println!();
    if fail_count > 0 {
        println!(
            "Summary: FAIL ({} blocker(s), {} warning(s))",
            fail_count, warn_count
        );
        anyhow::bail!("preflight failed");
    }
    if strict && warn_count > 0 {
        println!(
            "Summary: WARN ({} warning(s); --strict enabled)",
            warn_count
        );
        anyhow::bail!("preflight warnings in strict mode");
    }
    if warn_count > 0 {
        println!("Summary: WARN ({} warning(s), no blockers)", warn_count);
    } else {
        println!("Summary: OK");
    }
    Ok(())
}

fn opt_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn opt_u16(value: Option<u16>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".to_string())
}

async fn cmd_status(cfg: &Config) -> Result<()> {
    let snap = HealthSnapshot::collect(cfg).await?;

    println!(
        "World       : {}",
        snap.battlegroup_title
            .as_deref()
            .or(cfg.title.as_deref())
            .unwrap_or("—")
    );
    println!(
        "Battlegroup : {}  Phase: {}",
        cfg.battlegroup, snap.battlegroup_phase
    );
    println!("Namespace   : {}", cfg.namespace);

    if let Some(fls) = &snap.fls {
        println!(
            "FLS token   : {} (expires {})",
            fls.label(),
            fls.expires_at.format("%Y-%m-%d")
        );
    }
    if let (Some(used), Some(total)) = (snap.ram_used_bytes, snap.ram_total_bytes) {
        println!(
            "RAM         : {:.1} / {:.1} GB",
            used as f64 / 1e9,
            total as f64 / 1e9
        );
    }

    println!();
    println!("{:<32} {:<12} {}", "Map", "Phase", "Replicas");
    println!("{}", "-".repeat(50));
    for map in &snap.maps {
        let dot = if map.phase == "Running" { "●" } else { "○" };
        println!(
            "{} {:<30} {:<12} {}",
            dot, map.name, map.phase, map.replicas
        );
    }
    Ok(())
}

async fn cmd_maps(action: MapsCommand, cfg: &Config) -> Result<()> {
    match action {
        MapsCommand::List => {
            let snap = HealthSnapshot::collect(cfg).await?;
            for map in &snap.maps {
                let dot = if map.phase == "Running" { "●" } else { "○" };
                println!("{} {}  ({})", dot, map.name, map.phase);
            }
        }
        MapsCommand::Start { name, force } => {
            println!("Starting {}...", name);
            maps::start(cfg, &name, force).await?;
            println!("{}: start triggered.", name);
            print_target_summary(cfg);
        }
        MapsCommand::Stop { name } => {
            println!("Stopping {}...", name);
            maps::stop(cfg, &name).await?;
            println!("{}: stop triggered.", name);
            print_target_summary(cfg);
        }
    }
    Ok(())
}

async fn cmd_update(cfg: &Config) -> Result<()> {
    println!("Running update pipeline...");
    let out = update::run(cfg).await?;
    print!("{}", out);
    Ok(())
}

async fn cmd_gateway_patch(cfg: &Config) -> Result<()> {
    match gateway::patch(cfg).await? {
        true => println!("gateway: --RMQGameHttpPort=30196 applied."),
        false => println!("gateway: already patched, nothing to do."),
    }
    print_target_summary(cfg);
    Ok(())
}

async fn cmd_token_check(cfg: &Config) -> Result<()> {
    let status = dune_ctl_core::fls::check(cfg).await?;
    println!(
        "FLS token: {} (expires {})",
        status.label(),
        status.expires_at.format("%Y-%m-%d %H:%M UTC")
    );
    match status.state {
        FlsTokenState::Ok => {}
        FlsTokenState::WarningSoon => {
            eprintln!(
                "WARNING: {} days until expiry — rotate token by 2026-08-20.",
                status.days_remaining
            );
        }
        FlsTokenState::Critical => {
            eprintln!("CRITICAL: {} days until expiry!", status.days_remaining);
            std::process::exit(2);
        }
        FlsTokenState::Expired => {
            eprintln!("CRITICAL: FLS token is EXPIRED — server browser will not show this server.");
            std::process::exit(2);
        }
    }
    Ok(())
}

async fn cmd_backup(action: BackupCommand, cfg: &Config) -> Result<()> {
    match action {
        BackupCommand::List => {
            let entries = backup::list(cfg).await?;
            if entries.is_empty() {
                println!("No backups found in /srv/backups/dune/{}.", cfg.battlegroup);
                return Ok(());
            }
            println!("{:<20} {:<5} {:<10} Path", "Timestamp", "DB", "Size");
            println!("{}", "-".repeat(80));
            for e in &entries {
                println!(
                    "{:<20} {:<5} {:<10} {}",
                    e.timestamp,
                    if e.has_db { "yes" } else { "no" },
                    backup::format_size(e.size_bytes),
                    e.path.display()
                );
            }
        }
        BackupCommand::Run { skip_db, name, keep } => {
            println!("Starting backup for {}...", cfg.battlegroup);
            backup::run(cfg, skip_db, name.as_deref()).await?;
            if keep > 0 {
                let removed = backup::prune(cfg, keep).await?;
                if removed.is_empty() {
                    println!("Retention: {} bundles kept, nothing pruned.", keep);
                } else {
                    for path in &removed {
                        println!("Pruned: {}", path.display());
                    }
                    println!("Retention: kept {} most recent bundles.", keep);
                }
            }
        }
        BackupCommand::Schedule { show, remove, cron, keep } => {
            cmd_schedule(show, remove, &cron, keep, cfg)?;
        }
        BackupCommand::Restore { bundle, yes } => {
            if !yes {
                eprintln!(
                    "ERROR: This will OVERWRITE the live database for {}.\n\
                     The battlegroup should be stopped first.\n\
                     Re-run with --yes to confirm.",
                    cfg.battlegroup
                );
                std::process::exit(1);
            }
            println!(
                "Restoring bundle '{}' for {}...",
                bundle, cfg.battlegroup
            );
            backup::restore(cfg, &bundle).await?;
            println!("Restore complete.");
            print_target_summary(cfg);
        }
    }
    Ok(())
}

async fn cmd_logs(cfg: &Config, target: &str, follow: bool, tail: usize) -> Result<()> {
    println!("Resolving target '{}'...", target);
    let pod = logs::resolve_pod(cfg, target).await?;
    println!("Pod: {}", pod);
    if follow {
        logs::stream(cfg, target, tail).await
    } else {
        let lines = logs::tail(cfg, target, tail).await?;
        for line in lines {
            println!("{}", line);
        }
        Ok(())
    }
}

async fn cmd_players(cfg: &Config) -> Result<()> {
    let online = players::list_online(cfg).await?;
    if online.is_empty() {
        println!("No players currently online.");
    } else {
        println!("{} player(s) online:", online.len());
        println!("{:<32} Last login", "Name");
        println!("{}", "-".repeat(56));
        for p in &online {
            println!(
                "{:<32} {}",
                p.display_name,
                p.last_login.as_deref().unwrap_or("—")
            );
        }
    }
    Ok(())
}

const CRON_MARKER: &str = "# dune-ctl-backup";

fn cmd_schedule(show: bool, remove: bool, cron: &str, keep: usize, cfg: &Config) -> Result<()> {
    let current = read_user_crontab()?;

    if show {
        match current.lines().find(|l| l.contains(CRON_MARKER)) {
            Some(line) => println!("{}", line),
            None => println!("No dune-ctl backup schedule installed."),
        }
        return Ok(());
    }

    // Strip any existing dune-ctl-backup line
    let stripped: String = current
        .lines()
        .filter(|l| !l.contains(CRON_MARKER))
        .collect::<Vec<_>>()
        .join("\n");
    let stripped = stripped.trim_end().to_string();

    if remove {
        let new = if stripped.is_empty() {
            String::new()
        } else {
            format!("{}\n", stripped)
        };
        write_user_crontab(&new)?;
        println!("dune-ctl backup schedule removed.");
        return Ok(());
    }

    let bin = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from(
            "/home/dune/dune-server/dune-ctl/target/release/dune-ctl",
        ));
    let entry = format!(
        "{}  DUNE_CTL_WORLD={} {} backup run --keep {}  {}",
        cron,
        cfg.battlegroup,
        bin.display(),
        keep,
        CRON_MARKER,
    );

    let new_crontab = if stripped.is_empty() {
        format!("{}\n", entry)
    } else {
        format!("{}\n{}\n", stripped, entry)
    };
    write_user_crontab(&new_crontab)?;

    println!("Backup schedule installed:");
    println!("  Schedule : {}", cron);
    println!("  Binary   : {}", bin.display());
    println!("  World    : {}", cfg.battlegroup);
    println!("  Keep     : {} most recent bundles", keep);
    println!();
    println!("Verify with: dune-ctl backup schedule --show");
    Ok(())
}

fn read_user_crontab() -> Result<String> {
    let out = std::process::Command::new("crontab")
        .arg("-l")
        .output()
        .context("failed to run crontab -l")?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        // "no crontab for user" exits non-zero — treat as empty
        Ok(String::new())
    }
}

fn write_user_crontab(content: &str) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = std::process::Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to spawn crontab -")?;
    child
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all(content.as_bytes())
        .context("failed to write crontab")?;
    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("crontab - exited with status {}", status);
    }
    Ok(())
}

async fn cmd_web(_port: u16, _cfg: &Config) -> Result<()> {
    #[cfg(feature = "web")]
    {
        crate::web::run(_port, _cfg).await
    }
    #[cfg(not(feature = "web"))]
    {
        anyhow::bail!("web feature not compiled in; rebuild with: cargo build --features web")
    }
}
