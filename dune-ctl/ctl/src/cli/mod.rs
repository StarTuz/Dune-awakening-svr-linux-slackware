use anyhow::Result;
use clap::Subcommand;
use dune_ctl_core::{config::Config, fls::FlsTokenState, gateway, health::HealthSnapshot, maps, update};

#[derive(Subcommand)]
pub enum Command {
    /// Show battlegroup status, map phases, FLS token, and RAM
    Status,
    /// Map management
    Maps {
        #[command(subcommand)]
        action: MapsCommand,
    },
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
pub enum MapsCommand {
    /// List all 28 maps with current phase
    List,
    /// Start a stopped map
    Start { name: String },
    /// Stop a running map
    Stop { name: String },
}

pub async fn run(cmd: Command, cfg: &Config) -> Result<()> {
    match cmd {
        Command::Status => cmd_status(cfg).await,
        Command::Maps { action } => cmd_maps(action, cfg).await,
        Command::Update => cmd_update(cfg).await,
        Command::GatewayPatch => cmd_gateway_patch(cfg).await,
        Command::TokenCheck => cmd_token_check(cfg).await,
        Command::Web { port } => cmd_web(port, cfg).await,
    }
}

async fn cmd_status(cfg: &Config) -> Result<()> {
    let snap = HealthSnapshot::collect(cfg).await?;

    println!("Battlegroup : {}  Phase: {}", cfg.battlegroup, snap.battlegroup_phase);

    if let Some(fls) = &snap.fls {
        println!(
            "FLS token   : {} (expires {})",
            fls.label(),
            fls.expires_at.format("%Y-%m-%d")
        );
    }
    if let (Some(used), Some(total)) = (snap.ram_used_bytes, snap.ram_total_bytes) {
        println!("RAM         : {:.1} / {:.1} GB", used as f64 / 1e9, total as f64 / 1e9);
    }

    println!();
    println!("{:<32} {:<12} {}", "Map", "Phase", "Replicas");
    println!("{}", "-".repeat(50));
    for map in &snap.maps {
        let dot = if map.phase == "Running" { "●" } else { "○" };
        println!("{} {:<30} {:<12} {}", dot, map.name, map.phase, map.replicas);
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
        }
        MapsCommand::Stop { name } => {
            println!("Stopping {}...", name);
            maps::stop(cfg, &name).await?;
            println!("{}: stop triggered.", name);
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
            eprintln!("WARNING: {} days until expiry — rotate token by 2026-08-20.", status.days_remaining);
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
