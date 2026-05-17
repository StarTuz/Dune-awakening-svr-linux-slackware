use anyhow::Result;
use clap::Subcommand;
use dune_ctl_core::{
    battlegroup,
    config::{Config, WorldProfile},
    diagnostics::CheckState,
    fls::FlsTokenState,
    gateway,
    health::HealthSnapshot,
    maps, settings, sietches, update,
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
    Start { name: String },
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
    Apply {
        /// Allow deploy even when local settings differ from deployed settings
        #[arg(long)]
        force: bool,
    },
    /// Deploy local settings, then restart the selected world's primary Sietch
    ApplyRestart {
        /// Allow deploy+restart even when local settings differ from deployed settings
        #[arg(long)]
        force: bool,
    },
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
        SettingsCommand::Apply { force } => {
            guard_settings_apply(cfg, force).await?;
            settings::apply(cfg).await?;
            println!("UserEngine.ini and UserGame.ini deployed to /srv/UserSettings.");
            print_target_summary(cfg);
            println!("Follow-up   : restart primary Sietch if the changed settings require it.");
        }
        SettingsCommand::ApplyRestart { force } => {
            guard_settings_apply(cfg, force).await?;
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

async fn guard_settings_apply(cfg: &Config, force: bool) -> Result<()> {
    let drift = settings::drift(cfg).await?;
    if !drift.deployed_available || drift.changed_count() == 0 {
        return Ok(());
    }
    if force {
        eprintln!(
            "WARNING: deploying despite {} local-vs-deployed managed setting difference(s).",
            drift.changed_count()
        );
        return Ok(());
    }

    eprintln!(
        "Refusing to deploy: {} local-vs-deployed managed setting difference(s) detected.",
        drift.changed_count()
    );
    eprintln!("Changed managed settings:");
    for item in drift.items.iter().filter(|item| item.changed()) {
        let marker = if matches!(item.def.key, "sietch_name" | "sietch_password") {
            " !"
        } else {
            "  "
        };
        eprintln!(
            "{} {:<24} local={:<12} deployed={}",
            marker,
            item.def.key,
            settings::display_drift_local(item),
            settings::display_drift_deployed(item)
        );
    }
    eprintln!("Run `dune-ctl settings pull` if deployed settings are the source of truth.");
    eprintln!("Run this command again with `--force` to overwrite deployed settings anyway.");
    anyhow::bail!("settings drift guard blocked deploy")
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
    print_check("stale nft firewalld", &snap.diagnostics.stale_nft_firewalld);
    if let Some(gw) = &snap.gateway {
        println!(
            "{:<22} {}",
            "gateway patch",
            if gw.patched { "ok" } else { "missing" }
        );
    }
    println!("nft tables: {}", snap.diagnostics.nft_tables.join(", "));
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
    let settings_drift = settings::drift(cfg).await?;
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

    rows.push(match snap.diagnostics.stale_nft_firewalld.state {
        CheckState::Ok => {
            PreflightRow::ok("stale nft", &snap.diagnostics.stale_nft_firewalld.message)
        }
        CheckState::Warning => {
            PreflightRow::warn("stale nft", &snap.diagnostics.stale_nft_firewalld.message)
        }
        CheckState::Critical => {
            PreflightRow::fail("stale nft", &snap.diagnostics.stale_nft_firewalld.message)
        }
        CheckState::Unknown => {
            PreflightRow::warn("stale nft", &snap.diagnostics.stale_nft_firewalld.message)
        }
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

    rows.push(if settings_drift.deployed_available {
        let changed = settings_drift.changed_count();
        if changed == 0 {
            PreflightRow::ok("settings drift", "0 changed managed setting(s)")
        } else {
            PreflightRow::warn(
                "settings drift",
                format!(
                    "{} changed managed setting(s); review before deploy",
                    changed
                ),
            )
        }
    } else {
        PreflightRow::warn(
            "settings drift",
            settings_drift
                .error
                .clone()
                .unwrap_or_else(|| "deployed settings unavailable".to_string()),
        )
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
        MapsCommand::Start { name } => {
            println!("Starting {}...", name);
            maps::start(cfg, &name).await?;
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
