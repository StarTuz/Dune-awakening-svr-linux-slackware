mod cli;
#[cfg(feature = "tui")]
mod tui;
#[cfg(feature = "web")]
mod web;

use anyhow::Result;
use clap::Parser;
use dune_ctl_core::config::Config;

#[derive(Parser)]
#[command(
    name = "dune-ctl",
    about = "Dune: Awakening server management",
    version
)]
struct Cli {
    /// Target world/BattleGroup id or title. Defaults to DUNE_CTL_WORLD or first ~/.dune world.
    #[arg(long, global = true)]
    world: Option<String>,
    #[command(subcommand)]
    command: Option<cli::Command>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    let world = args.world.or_else(|| std::env::var("DUNE_CTL_WORLD").ok());
    let cfg = Config::load(world.as_deref())?;

    match args.command {
        Some(cmd) => cli::run(cmd, &cfg).await,
        #[cfg(feature = "tui")]
        None => tui::run(&cfg).await,
        #[cfg(not(feature = "tui"))]
        None => {
            eprintln!("No subcommand given and tui feature is not enabled. Use --help.");
            std::process::exit(1);
        }
    }
}
