use anyhow::Result;
use clap::Subcommand;
use dune_ctl_core::{
    backup, battlegroup, capsules,
    config::{Config, WorldProfile},
    diagnostics::CheckState,
    fls::FlsTokenState,
    gateway,
    health::HealthSnapshot,
    logs, maintenance, maps, players, public_ip, settings, sietches, update,
};

#[derive(Subcommand)]
pub enum Command {
    /// List locally known worlds/BattleGroups
    Worlds {
        #[command(subcommand)]
        action: WorldsCommand,
    },
    /// Inventory and activate world capsules
    Capsules {
        #[command(subcommand)]
        action: CapsulesCommand,
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
    /// Cleanly stop the selected Dune world for planned host maintenance
    Shutdown {
        /// Confirm the shutdown without refusing at the safety gate
        #[arg(long)]
        yes: bool,
        /// Skip the recommended pre-shutdown backup
        #[arg(long)]
        skip_backup: bool,
        /// Seconds to wait for game servers to stop
        #[arg(long, default_value = "300")]
        timeout: u64,
    },
    /// Re-apply --RMQGameHttpPort=30196 to the gateway Deployment
    GatewayPatch,
    /// Check FLS token expiry; exits non-zero if critical or expired
    TokenCheck,
    /// Inspect or update the selected world's advertised public Internet IP
    PublicIp {
        #[command(subcommand)]
        action: PublicIpCommand,
    },
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
pub enum CapsulesCommand {
    /// Print current capsule inventory and host/package isolation state
    Inventory,
    /// Render a capsule without applying it to Kubernetes
    Create {
        /// Capsule environment
        #[arg(long, default_value = "live")]
        env: String,
        /// World title; prompts when omitted
        #[arg(long)]
        name: Option<String>,
        /// Sietch name; prompts when omitted
        #[arg(long)]
        sietch_name: Option<String>,
        /// Farm region; prompts when omitted
        #[arg(long)]
        region: Option<String>,
        /// Self-hosting token
        #[arg(long)]
        token: Option<String>,
        /// Read self-hosting token from a file
        #[arg(long)]
        token_file: Option<String>,
        /// Package root containing server/scripts/setup
        #[arg(long)]
        package_root: Option<String>,
        /// Battlegroup id; generated from token when omitted
        #[arg(long)]
        world_id: Option<String>,
        /// Public host IP advertised to FLS
        #[arg(long)]
        host_ip: Option<String>,
        /// Overwrite an existing capsule directory
        #[arg(long)]
        force: bool,
    },
    /// Refresh an existing capsule from its package root
    Refresh {
        /// Capsule environment
        #[arg(long, default_value = "live")]
        env: String,
        /// Capsule battlegroup id
        #[arg(long)]
        world_id: String,
        /// Package root containing server/scripts/setup
        #[arg(long)]
        package_root: Option<String>,
        /// Steam app id
        #[arg(long)]
        app_id: Option<String>,
        /// Allow refresh to render an older image tag
        #[arg(long)]
        allow_downgrade: bool,
    },
    /// Package management for a capsule environment
    Package {
        #[command(subcommand)]
        action: CapsulePackageCommand,
    },
    /// Import package images into k3s/containerd
    Images {
        #[command(subcommand)]
        action: CapsuleImagesCommand,
    },
    /// Dry-run or apply a rendered capsule
    Activate {
        /// Capsule environment
        #[arg(long, default_value = "live")]
        env: String,
        /// Capsule battlegroup id
        #[arg(long)]
        world_id: String,
        /// Apply namespace, secrets, and BattleGroup
        #[arg(long)]
        apply: bool,
        /// Allow apply while other battlegroups exist
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum CapsulePackageCommand {
    /// Download a package with SteamCMD, then validate it
    Install {
        /// Capsule environment
        #[arg(long, default_value = "live")]
        env: String,
        /// Steam app id
        #[arg(long)]
        app_id: Option<String>,
        /// Install root
        #[arg(long)]
        package_root: Option<String>,
        /// SteamCMD script path
        #[arg(long)]
        steamcmd: Option<String>,
    },
    /// Validate an installed package root
    Validate {
        /// Capsule environment
        #[arg(long, default_value = "live")]
        env: String,
        /// Steam app id
        #[arg(long)]
        app_id: Option<String>,
        /// Package root to validate
        #[arg(long)]
        package_root: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum CapsuleImagesCommand {
    /// Import package images into k3s/containerd
    Load {
        /// Capsule environment
        #[arg(long, default_value = "live")]
        env: String,
        /// Package root to import from
        #[arg(long)]
        package_root: Option<String>,
        /// Steam app id
        #[arg(long)]
        app_id: Option<String>,
    },
    /// Verify package images are registered in k3s/containerd
    Verify {
        /// Capsule environment
        #[arg(long, default_value = "live")]
        env: String,
        /// Package root to verify
        #[arg(long)]
        package_root: Option<String>,
        /// Steam app id
        #[arg(long)]
        app_id: Option<String>,
    },
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
    /// Toggle director-managed persistence (MinServers) for a map.
    ///
    /// Persistence is separate from start/stop: --on does not start the map
    /// now, and --off is required before a stop will stick. Writes the live
    /// BattleGroup CR and mirrors the capsule source.
    Persist {
        name: String,
        /// Make the map director-persistent (MinServers=1)
        #[arg(long)]
        on: bool,
        /// Remove director persistence (MinServers=0)
        #[arg(long)]
        off: bool,
        /// Confirm the live change (disconnects nobody, but edits the CR)
        #[arg(long)]
        yes: bool,
    },
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
    /// Open the Battlegroup Editor (bg-util) to manage Sietches/dimensions, names, and memory
    Edit {
        /// Open the raw BattleGroup YAML in the default editor instead of bg-util
        #[arg(long)]
        advanced: bool,
    },
    /// Add a Sietch (provisions a world partition + raises the active count)
    Add {
        /// Apply without the confirmation gate
        #[arg(long)]
        yes: bool,
        /// Show the CR patch without applying
        #[arg(long)]
        dry_run: bool,
        /// Skip the automatic pre-change backup
        #[arg(long)]
        skip_backup: bool,
    },
    /// Set the number of active Sietches (replicas) for the primary map
    Scale {
        /// Desired active Sietch count (must be <= existing world partitions)
        count: u32,
        /// Apply without the confirmation gate
        #[arg(long)]
        yes: bool,
        /// Show the result without applying
        #[arg(long)]
        dry_run: bool,
    },
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

#[derive(Subcommand)]
pub enum PublicIpCommand {
    /// Show configured public IP values from local files and live Kubernetes
    Show,
    /// Detect the current WAN IP from HTTPS providers and compare with config
    Check {
        /// Provider URLs returning a plain-text IP; may be repeated
        #[arg(long = "provider")]
        providers: Vec<String>,
    },
    /// Set the selected world's advertised public IP
    Set {
        ip: String,
        /// Print planned changes without writing files or patching Kubernetes
        #[arg(long)]
        dry_run: bool,
        /// Apply without refusing at the confirmation gate
        #[arg(long)]
        yes: bool,
        /// Do not update local ~/.dune world/capsule files
        #[arg(long)]
        skip_files: bool,
        /// Do not patch the live BattleGroup or gateway Deployment
        #[arg(long)]
        skip_live: bool,
    },
    /// Detect the WAN IP and apply it after provider quorum
    ApplyDetected {
        /// Provider URLs returning a plain-text IP; may be repeated
        #[arg(long = "provider")]
        providers: Vec<String>,
        /// Print planned changes without writing files or patching Kubernetes
        #[arg(long)]
        dry_run: bool,
        /// Apply without refusing at the confirmation gate
        #[arg(long)]
        yes: bool,
        /// Do not update local ~/.dune world/capsule files
        #[arg(long)]
        skip_files: bool,
        /// Do not patch the live BattleGroup or gateway Deployment
        #[arg(long)]
        skip_live: bool,
    },
}

pub async fn run(cmd: Command, cfg: &Config) -> Result<()> {
    match cmd {
        Command::Worlds { action } => cmd_worlds(action, cfg).await,
        Command::Capsules { action } => cmd_capsules(action, cfg).await,
        Command::Status => cmd_status(cfg).await,
        Command::Preflight { strict } => cmd_preflight(cfg, strict).await,
        Command::Maps { action } => cmd_maps(action, cfg).await,
        Command::Sietches { action } => cmd_sietches(action, cfg).await,
        Command::Battlegroup { action } => cmd_battlegroup(action, cfg).await,
        Command::Settings { action } => cmd_settings(action, cfg).await,
        Command::Diagnostics => cmd_diagnostics(cfg).await,
        Command::Update => cmd_update(cfg).await,
        Command::Shutdown {
            yes,
            skip_backup,
            timeout,
        } => cmd_shutdown(cfg, yes, skip_backup, timeout).await,
        Command::GatewayPatch => cmd_gateway_patch(cfg).await,
        Command::TokenCheck => cmd_token_check(cfg).await,
        Command::PublicIp { action } => cmd_public_ip(action, cfg).await,
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

async fn cmd_public_ip(action: PublicIpCommand, cfg: &Config) -> Result<()> {
    match action {
        PublicIpCommand::Show => {
            let summary = public_ip::show(cfg).await?;
            println!("Public IP summary");
            print_target_summary(cfg);
            println!(
                "Local files : {}",
                display_list(&summary.local_ips, "none found")
            );
            println!(
                "Live spec   : {}",
                display_list(&summary.live_ips, "unavailable or none found")
            );
            println!(
                "Gateway RMQ : {}",
                summary.gateway_hostname.as_deref().unwrap_or("unavailable")
            );
            println!(
                "RMQ HTTP    : {}",
                match summary.gateway_http_patched {
                    Some(true) => "patched",
                    Some(false) => "missing",
                    None => "unavailable",
                }
            );
        }
        PublicIpCommand::Check { providers } => {
            let detection = public_ip::detect(&providers).await?;
            let summary = public_ip::show(cfg).await?;
            print_detection_summary(&detection);
            println!(
                "Configured  : local [{}], live [{}], gateway [{}]",
                display_list(&summary.local_ips, "none"),
                display_list(&summary.live_ips, "none"),
                summary.gateway_hostname.as_deref().unwrap_or("none")
            );
            if configured_matches_detected(&summary, &detection.detected_ip) {
                println!("State       : detected IP matches configured IP.");
            } else {
                println!("State       : detected IP differs from configured IP.");
                std::process::exit(1);
            }
        }
        PublicIpCommand::Set {
            ip,
            dry_run,
            yes,
            skip_files,
            skip_live,
        } => {
            let plan = public_ip::plan_set(cfg, &ip, skip_live).await?;
            print_public_ip_plan(&plan, skip_files);

            if dry_run {
                println!("Dry run: no changes applied.");
                return Ok(());
            }
            if !yes {
                anyhow::bail!(
                    "refusing to apply without --yes; rerun with --dry-run to inspect only"
                );
            }

            public_ip::apply_set(cfg, &ip, skip_files, skip_live).await?;
            println!("Public IP updated to {}.", ip);
            if !skip_live {
                println!(
                    "Gateway follow-up: rollout may refresh BattleGroup status asynchronously; cached status addresses can lag."
                );
            }
        }
        PublicIpCommand::ApplyDetected {
            providers,
            dry_run,
            yes,
            skip_files,
            skip_live,
        } => {
            let detection = public_ip::detect(&providers).await?;
            print_detection_summary(&detection);
            let plan = public_ip::plan_set(cfg, &detection.detected_ip, skip_live).await?;
            print_public_ip_plan(&plan, skip_files);

            if dry_run {
                println!("Dry run: no changes applied.");
                return Ok(());
            }
            if !yes {
                anyhow::bail!(
                    "refusing to apply detected IP without --yes; rerun with --dry-run to inspect only"
                );
            }

            public_ip::apply_set(cfg, &detection.detected_ip, skip_files, skip_live).await?;
            println!("Public IP updated to {}.", detection.detected_ip);
            if !skip_live {
                println!(
                    "Gateway follow-up: rollout may refresh BattleGroup status asynchronously; cached status addresses can lag."
                );
            }
        }
    }
    Ok(())
}

fn configured_matches_detected(summary: &public_ip::PublicIpSummary, detected_ip: &str) -> bool {
    let mut values = Vec::new();
    values.extend(summary.local_ips.iter());
    values.extend(summary.live_ips.iter());
    if let Some(gateway) = &summary.gateway_hostname {
        values.push(gateway);
    }
    !values.is_empty() && values.iter().all(|value| value.as_str() == detected_ip)
}

fn print_detection_summary(detection: &public_ip::DetectionSummary) {
    println!("Detected IP : {}", detection.detected_ip);
    println!("Providers");
    for observation in &detection.observations {
        match (&observation.ip, &observation.error) {
            (Some(ip), _) => println!("  {:<32} {}", observation.provider, ip),
            (_, Some(error)) if error.is_empty() => {
                println!("  {:<32} error", observation.provider)
            }
            (_, Some(error)) => println!("  {:<32} error: {}", observation.provider, error),
            _ => println!("  {:<32} no response", observation.provider),
        }
    }
}

fn display_list(values: &[String], empty: &str) -> String {
    if values.is_empty() {
        empty.to_string()
    } else {
        values.join(", ")
    }
}

fn print_public_ip_plan(plan: &public_ip::PublicIpPlan, skip_files: bool) {
    println!("Public IP update plan");
    println!("New IP      : {}", plan.new_ip);
    println!(
        "Old IPs     : {}",
        display_list(&plan.old_ips, "none detected")
    );
    println!(
        "Live patch  : {}",
        if plan.live { "yes" } else { "skipped" }
    );
    println!(
        "File writes : {}",
        if skip_files { "skipped" } else { "enabled" }
    );
    for file in &plan.files {
        let state = if !file.exists {
            if file.required {
                "missing required"
            } else {
                "missing optional"
            }
        } else if file.changed {
            "will update"
        } else {
            "clean"
        };
        println!("  {:<16} {}", state, file.path.display());
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

fn capsule_option(args: &mut Vec<String>, flag: &str, value: Option<String>) {
    if let Some(value) = value {
        args.push(flag.to_string());
        args.push(value);
    }
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
            match sietches::capacity(cfg).await {
                Ok(cap) => {
                    println!(
                        "Capacity: {} of {} Sietch(es) active for {} (max = enabled worldPartitions).",
                        cap.active, cap.max, cap.map
                    );
                    if cap.max <= 1 {
                        println!(
                            "Single-Sietch world. Add a Sietch with `dune-ctl --world {} sietches edit` \
                             (bg-util: raise worldPartitions, set active ≤ count, give it a unique name).",
                            cfg.title.as_deref().unwrap_or(&cfg.battlegroup)
                        );
                    }
                    println!();
                }
                Err(e) => eprintln!("warning: could not read Sietch capacity: {e}"),
            }

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
        SietchesCommand::Edit { advanced } => {
            if advanced {
                println!(
                    "Opening raw BattleGroup YAML for {} (default editor). Save and exit to apply.",
                    selected_world_label(cfg)
                );
            } else {
                println!(
                    "Opening Battlegroup Editor (bg-util) for {}. Edit dimensions/Sietches, names, and memory; save and exit to apply.",
                    selected_world_label(cfg)
                );
            }
            sietches::edit(cfg, advanced).await?;
        }
        SietchesCommand::Add {
            yes,
            dry_run,
            skip_backup,
        } => {
            if dry_run {
                let (plan, patch) = sietches::add(cfg, true).await?;
                println!(
                    "Dry run — would add a Sietch to {} (active {} -> {}):",
                    plan.map,
                    plan.new_replicas - 1,
                    plan.new_replicas
                );
                println!(
                    "  new world partition: id={} dimension={} grid bounds x[{}..{}] y[{}..{}]",
                    plan.new_partition_id,
                    plan.new_dimension,
                    plan.min_x,
                    plan.max_x,
                    plan.min_y,
                    plan.max_y
                );
                println!("CR patch:\n{patch}");
                return Ok(());
            }
            if !yes {
                anyhow::bail!(
                    "refusing to add a Sietch without --yes; rerun with --dry-run to inspect the CR patch first"
                );
            }
            if !skip_backup {
                println!("Backing up before adding a Sietch...");
                backup::run(cfg, false, None).await?;
            }
            let (plan, _) = sietches::add(cfg, false).await?;
            println!(
                "Added Sietch to {}: world partition id={} (dimension {}); active Sietches now {}.",
                plan.map, plan.new_partition_id, plan.new_dimension, plan.new_replicas
            );
            println!(
                "Note: the new Sietch inherits the world's shared display name until per-Sietch \
                 naming lands (see dune-ctl/SIETCHES-DESIGN.md). Each Sietch needs ~5 Gi RAM — \
                 verify headroom and that the new instance reaches Running."
            );
            print_target_summary(cfg);
        }
        SietchesCommand::Scale {
            count,
            yes,
            dry_run,
        } => {
            if dry_run {
                let cap = sietches::scale(cfg, count, true).await?;
                println!(
                    "Dry run — would set active Sietches to {} (max {}) for {}.",
                    cap.active, cap.max, cap.map
                );
                return Ok(());
            }
            if !yes {
                anyhow::bail!(
                    "refusing to scale Sietches without --yes; rerun with --dry-run to inspect only"
                );
            }
            let cap = sietches::scale(cfg, count, false).await?;
            println!(
                "Active Sietches set to {} (max {}) for {}.",
                cap.active, cap.max, cap.map
            );
            print_target_summary(cfg);
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

async fn cmd_capsules(action: CapsulesCommand, cfg: &Config) -> Result<()> {
    match action {
        CapsulesCommand::Inventory => {
            let text = capsules::inventory(cfg).await?;
            print!("{}", text);
        }
        CapsulesCommand::Create {
            env,
            name,
            sietch_name,
            region,
            token,
            token_file,
            package_root,
            world_id,
            host_ip,
            force,
        } => {
            if token.is_some() && token_file.is_some() {
                anyhow::bail!("use either --token or --token-file, not both");
            }
            let mut args = vec!["create".to_string(), "--env".to_string(), env];
            capsule_option(&mut args, "--name", name);
            capsule_option(&mut args, "--sietch-name", sietch_name);
            capsule_option(&mut args, "--region", region);
            capsule_option(&mut args, "--token", token);
            capsule_option(&mut args, "--token-file", token_file);
            capsule_option(&mut args, "--package-root", package_root);
            capsule_option(&mut args, "--world-id", world_id);
            capsule_option(&mut args, "--host-ip", host_ip);
            if force {
                args.push("--force".to_string());
            }
            capsules::run_stream(cfg, &args).await?;
        }
        CapsulesCommand::Refresh {
            env,
            world_id,
            package_root,
            app_id,
            allow_downgrade,
        } => {
            let mut args = vec![
                "refresh".to_string(),
                "--env".to_string(),
                env,
                "--world-id".to_string(),
                world_id,
            ];
            capsule_option(&mut args, "--package-root", package_root);
            capsule_option(&mut args, "--app-id", app_id);
            if allow_downgrade {
                args.push("--allow-downgrade".to_string());
            }
            capsules::run_stream(cfg, &args).await?;
        }
        CapsulesCommand::Package { action } => match action {
            CapsulePackageCommand::Install {
                env,
                app_id,
                package_root,
                steamcmd,
            } => {
                let mut args = vec![
                    "package".to_string(),
                    "install".to_string(),
                    "--env".to_string(),
                    env,
                ];
                capsule_option(&mut args, "--app-id", app_id);
                capsule_option(&mut args, "--package-root", package_root);
                capsule_option(&mut args, "--steamcmd", steamcmd);
                capsules::run_stream(cfg, &args).await?;
            }
            CapsulePackageCommand::Validate {
                env,
                app_id,
                package_root,
            } => {
                let mut args = vec![
                    "package".to_string(),
                    "validate".to_string(),
                    "--env".to_string(),
                    env,
                ];
                capsule_option(&mut args, "--app-id", app_id);
                capsule_option(&mut args, "--package-root", package_root);
                capsules::run_stream(cfg, &args).await?;
            }
        },
        CapsulesCommand::Images { action } => match action {
            CapsuleImagesCommand::Load {
                env,
                package_root,
                app_id,
            } => {
                let mut args = vec![
                    "images".to_string(),
                    "load".to_string(),
                    "--env".to_string(),
                    env,
                ];
                capsule_option(&mut args, "--package-root", package_root);
                capsule_option(&mut args, "--app-id", app_id);
                capsules::run_stream(cfg, &args).await?;
            }
            CapsuleImagesCommand::Verify {
                env,
                package_root,
                app_id,
            } => {
                let mut args = vec![
                    "images".to_string(),
                    "verify".to_string(),
                    "--env".to_string(),
                    env,
                ];
                capsule_option(&mut args, "--package-root", package_root);
                capsule_option(&mut args, "--app-id", app_id);
                capsules::run_stream(cfg, &args).await?;
            }
        },
        CapsulesCommand::Activate {
            env,
            world_id,
            apply,
            force,
        } => {
            let mut args = vec![
                "activate".to_string(),
                "--env".to_string(),
                env,
                "--world-id".to_string(),
                world_id,
            ];
            if apply {
                args.push("--apply".to_string());
            }
            if force {
                args.push("--force".to_string());
            }
            capsules::run_stream(cfg, &args).await?;
        }
    }
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
    let diagnostics = dune_ctl_core::diagnostics::DiagnosticsSnapshot::collect().await;
    println!("Diagnostics for {}", cfg.battlegroup);
    print_check("k3s API", &diagnostics.k3s_api);
    print_check("firewall backend", &diagnostics.firewall_backend);
    if diagnostics.k3s_api.state == CheckState::Ok {
        match gateway::status(cfg).await {
            Ok(gw) => println!(
                "{:<22} {}",
                "gateway patch",
                if gw.patched { "ok" } else { "missing" }
            ),
            Err(e) => println!("{:<22} unavailable {}", "gateway patch", e),
        }
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
    let diagnostics = dune_ctl_core::diagnostics::DiagnosticsSnapshot::collect().await;
    let mut rows = Vec::new();

    rows.push(match diagnostics.k3s_api.state {
        CheckState::Ok => PreflightRow::ok("k3s API", &diagnostics.k3s_api.message),
        CheckState::Warning => PreflightRow::warn("k3s API", &diagnostics.k3s_api.message),
        CheckState::Critical => PreflightRow::fail("k3s API", &diagnostics.k3s_api.message),
        CheckState::Unknown => PreflightRow::warn("k3s API", &diagnostics.k3s_api.message),
    });

    rows.push(match diagnostics.firewall_backend.state {
        CheckState::Ok => {
            PreflightRow::ok("firewall backend", &diagnostics.firewall_backend.message)
        }
        CheckState::Warning => {
            PreflightRow::warn("firewall backend", &diagnostics.firewall_backend.message)
        }
        CheckState::Critical => {
            PreflightRow::fail("firewall backend", &diagnostics.firewall_backend.message)
        }
        CheckState::Unknown => {
            PreflightRow::warn("firewall backend", &diagnostics.firewall_backend.message)
        }
    });

    rows.push(if cfg.has_capsule() {
        PreflightRow::ok(
            "capsule target",
            format!(
                "{} capsule selected ({})",
                cfg.backup_environment,
                cfg.capsule_dir().display()
            ),
        )
    } else if cfg.world_spec.is_some() {
        PreflightRow::warn(
            "capsule target",
            "legacy world spec selected; no capsule metadata found",
        )
    } else {
        PreflightRow::fail("capsule target", "no world spec or capsule metadata found")
    });

    if diagnostics.k3s_api.state == CheckState::Critical {
        print_preflight_rows(cfg, &rows, strict)?;
        anyhow::bail!("preflight failed");
    }

    let snap = HealthSnapshot::collect(cfg).await?;

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

    rows.push(
        match (
            snap.battlegroup_stopped,
            snap.battlegroup_phase.as_str(),
            snap.sietches.iter().find(|sietch| sietch.primary),
        ) {
            (true, phase, _) => PreflightRow::warn(
                "server start",
                format!("battlegroup is stopped; phase {}", phase),
            ),
            (false, "Healthy", Some(sietch))
                if sietch.phase == "Running"
                    && sietch.ready_replicas.unwrap_or_default() > 0
                    && sietch.consistency == battlegroup::MapConsistency::CleanOn =>
            {
                PreflightRow::ok(
                    "server start",
                    format!(
                        "healthy; {} ready {}/{}",
                        sietch.map,
                        opt_u32(sietch.ready_replicas),
                        opt_u32(sietch.target_replicas)
                    ),
                )
            }
            (false, phase, Some(sietch)) => PreflightRow::warn(
                "server start",
                format!(
                    "phase {}; {} {} ready {}/{} state {}",
                    phase,
                    sietch.map,
                    sietch.phase,
                    opt_u32(sietch.ready_replicas),
                    opt_u32(sietch.target_replicas),
                    sietch.consistency.label()
                ),
            ),
            (false, phase, None) => PreflightRow::fail(
                "server start",
                format!("phase {}; primary Sietch missing", phase),
            ),
        },
    );

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

    print_preflight_rows(cfg, &rows, strict)
}

fn print_preflight_rows(cfg: &Config, rows: &[PreflightRow], strict: bool) -> Result<()> {
    println!("Preflight for {}", selected_world_label(cfg));
    println!("Battlegroup : {}", cfg.battlegroup);
    println!("Namespace   : {}", cfg.namespace);
    println!();
    for row in rows {
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
    let diagnostics = dune_ctl_core::diagnostics::DiagnosticsSnapshot::collect().await;

    if diagnostics.k3s_api.state == CheckState::Critical {
        println!("World       : {}", selected_world_label(cfg));
        println!("Battlegroup : {}", cfg.battlegroup);
        println!("Namespace   : {}", cfg.namespace);
        println!(
            "Capsule     : {} {}",
            cfg.backup_environment,
            if cfg.has_capsule() {
                "(capsule)"
            } else {
                "(legacy)"
            }
        );
        print_check("k3s API", &diagnostics.k3s_api);
        print_check("firewall backend", &diagnostics.firewall_backend);
        anyhow::bail!("cannot read BattleGroup status because k3s is not ready");
    }

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
    println!(
        "Capsule     : {} {}",
        cfg.backup_environment,
        if cfg.has_capsule() {
            "(capsule)"
        } else {
            "(legacy)"
        }
    );
    println!("k3s API     : {}", diagnostics.k3s_api.message);
    println!("Start       : {}", start_summary(&snap));
    if let Some(gateway) = &snap.gateway {
        println!(
            "Gateway     : {} ready {}/{}",
            if gateway.patched {
                "patched"
            } else {
                "missing RMQ HTTP patch"
            },
            opt_u32(gateway.ready_replicas),
            opt_u32(gateway.updated_replicas)
        );
    } else {
        println!("Gateway     : unavailable");
    }

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
    println!("{:<32} {:<12} {:<10} Persist", "Map", "Phase", "Replicas");
    println!("{}", "-".repeat(64));
    for map in &snap.maps {
        let dot = if map.phase == "Running" { "●" } else { "○" };
        let persist = if map.is_persistent() {
            format!("MinServers={}", opt_u32(map.min_servers))
        } else {
            "—".to_string()
        };
        println!(
            "{} {:<30} {:<12} {:<10} {}",
            dot, map.name, map.phase, map.replicas, persist
        );
    }
    Ok(())
}

fn start_summary(snap: &HealthSnapshot) -> String {
    let Some(primary) = snap.sietches.iter().find(|sietch| sietch.primary) else {
        return format!("{}; primary Sietch missing", snap.battlegroup_phase);
    };

    if snap.battlegroup_stopped {
        return format!(
            "stopped; desired {} replicas={}, phase {}",
            primary.map, primary.replicas, primary.phase
        );
    }

    if snap.battlegroup_phase == "Healthy"
        && primary.phase == "Running"
        && primary.ready_replicas.unwrap_or_default() > 0
        && primary.consistency == battlegroup::MapConsistency::CleanOn
    {
        return format!(
            "ready; {} running {}/{}",
            primary.map,
            opt_u32(primary.ready_replicas),
            opt_u32(primary.target_replicas)
        );
    }

    format!(
        "starting/degraded; BattleGroup {}, {} {} ready {}/{} state {}",
        snap.battlegroup_phase,
        primary.map,
        primary.phase,
        opt_u32(primary.ready_replicas),
        opt_u32(primary.target_replicas),
        primary.consistency.label()
    )
}

async fn cmd_maps(action: MapsCommand, cfg: &Config) -> Result<()> {
    match action {
        MapsCommand::List => {
            let snap = HealthSnapshot::collect(cfg).await?;
            for map in &snap.maps {
                let dot = if map.phase == "Running" { "●" } else { "○" };
                let persist = if map.is_persistent() {
                    format!("  [persist MinServers={}]", opt_u32(map.min_servers))
                } else {
                    String::new()
                };
                println!("{} {}  ({}){}", dot, map.name, map.phase, persist);
            }
        }
        MapsCommand::Start { name, force } => {
            println!("Starting {}...", name);
            maps::start(cfg, &name, force).await?;
            println!("{}: start triggered.", name);
            print_target_summary(cfg);
        }
        MapsCommand::Stop { name } => {
            if let Ok(Some(min)) = maps::min_servers(cfg, &name).await {
                if min >= 1 {
                    println!(
                        "warning: {} is director-persistent (MinServers={}); the director will \
                         likely restart it.\n         Run 'maps persist {} --off' first for a \
                         durable stop.",
                        name, min, name
                    );
                }
            }
            println!("Stopping {}...", name);
            maps::stop(cfg, &name).await?;
            println!("{}: stop triggered.", name);
            print_target_summary(cfg);
        }
        MapsCommand::Persist { name, on, off, yes } => {
            cmd_maps_persist(cfg, name, on, off, yes).await?;
        }
    }
    Ok(())
}

async fn cmd_maps_persist(
    cfg: &Config,
    name: String,
    on: bool,
    off: bool,
    yes: bool,
) -> Result<()> {
    let on = match (on, off) {
        (true, false) => true,
        (false, true) => false,
        _ => anyhow::bail!("specify exactly one of --on / --off"),
    };
    if !yes {
        anyhow::bail!(
            "refusing to change map persistence without --yes; this edits the live \
             BattleGroup CR director.ini (and the capsule source)"
        );
    }

    let verb = if on { "persistent" } else { "not persistent" };
    println!("Setting {} {} (MinServers={})...", name, verb, on as u32);
    let outcome = maps::set_persistence(cfg, &name, on, true).await?;

    println!(
        "{}: director.ini MinServers {} -> {}{}",
        name,
        opt_u32(outcome.previous),
        outcome.applied,
        if outcome.cr_changed {
            ""
        } else {
            " (CR already matched)"
        }
    );
    match (outcome.capsule_updated, &outcome.capsule_note) {
        (Some(true), _) => println!("capsule    : battlegroup.yaml updated"),
        (_, Some(note)) => println!("capsule    : {}", note),
        _ => {}
    }
    if on {
        println!(
            "note       : persistence does not start the map now — run 'maps start {}' to \
             bring it up; the director will keep/restart it.",
            name
        );
    } else {
        println!("note       : run 'maps stop {}' if you also want it down now.", name);
    }
    Ok(())
}

async fn cmd_update(cfg: &Config) -> Result<()> {
    println!("Running update pipeline...");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let cfg = cfg.clone();
    let task = tokio::spawn(async move {
        update::run_streamed(&cfg, update::UpdateOptions { start_after: true }, tx).await
    });

    while !task.is_finished() {
        while let Ok(line) = rx.try_recv() {
            println!("{}", line);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    while let Ok(line) = rx.try_recv() {
        println!("{}", line);
    }
    task.await??;
    Ok(())
}

async fn cmd_shutdown(cfg: &Config, yes: bool, skip_backup: bool, timeout: u64) -> Result<()> {
    if !yes {
        anyhow::bail!(
            "refusing to stop the Dune world without --yes; this disconnects players but does not reboot the host"
        );
    }

    println!("Running clean Dune shutdown for planned maintenance...");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let cfg = cfg.clone();
    let task = tokio::spawn(async move {
        maintenance::shutdown_for_reboot_streamed(
            &cfg,
            maintenance::ShutdownOptions {
                skip_backup,
                timeout_secs: timeout,
            },
            tx,
        )
        .await
    });

    while !task.is_finished() {
        while let Ok(line) = rx.try_recv() {
            println!("{}", line);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    while let Ok(line) = rx.try_recv() {
        println!("{}", line);
    }
    task.await??;
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
                println!(
                    "No backups found in /srv/backups/dune/{}/{}.",
                    cfg.backup_environment, cfg.battlegroup
                );
                return Ok(());
            }
            println!(
                "{:<20} {:<6} {:<5} {:<10} Path",
                "Timestamp", "ENV", "DB", "Size"
            );
            println!("{}", "-".repeat(88));
            for e in &entries {
                println!(
                    "{:<20} {:<6} {:<5} {:<10} {}",
                    e.timestamp,
                    e.environment,
                    if e.has_db { "yes" } else { "no" },
                    backup::format_size(e.size_bytes),
                    e.path.display()
                );
            }
        }
        BackupCommand::Run {
            skip_db,
            name,
            keep,
        } => {
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
        BackupCommand::Schedule {
            show,
            remove,
            cron,
            keep,
        } => {
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
                "Restoring bundle '{}' for {} ({})...",
                bundle, cfg.battlegroup, cfg.backup_environment
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

fn cmd_schedule(show: bool, remove: bool, cron: &str, keep: usize, cfg: &Config) -> Result<()> {
    if show {
        match backup::read_schedule() {
            Some(info) => {
                println!("Schedule : {}", info.cron);
                println!("Keep     : {} bundles", info.keep);
            }
            None => println!("No dune-ctl backup schedule installed."),
        }
        return Ok(());
    }

    if remove {
        backup::remove_schedule()?;
        println!("dune-ctl backup schedule removed.");
        return Ok(());
    }

    let bin = std::env::current_exe().unwrap_or_else(|_| {
        std::path::PathBuf::from("/home/dune/dune-server/dune-ctl/target/release/dune-ctl")
    });
    backup::write_schedule(&cfg.battlegroup, &bin.to_string_lossy(), cron, keep)?;
    println!("Backup schedule installed:");
    println!("  Schedule : {}", cron);
    println!("  Binary   : {}", bin.display());
    println!("  World    : {}", cfg.battlegroup);
    println!("  Keep     : {} most recent bundles", keep);
    println!();
    println!("Verify with: dune-ctl backup schedule --show");
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
