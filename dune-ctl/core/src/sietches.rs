use anyhow::Result;

use crate::{battlegroup, config::Config};

pub const PRIMARY_SIETCH_MAP: &str = "Survival_1";

/// Start the selected world's primary Sietch.
///
/// Funcom's current self-hosting model exposes one Sietch per BattleGroup. Until
/// a first-class per-Sietch lifecycle exists, primary Sietch lifecycle is the
/// selected BattleGroup lifecycle.
pub async fn start_primary(cfg: &Config) -> Result<()> {
    battlegroup::start(cfg).await
}

pub async fn stop_primary(cfg: &Config) -> Result<()> {
    battlegroup::stop(cfg).await
}

pub async fn restart_primary(cfg: &Config) -> Result<()> {
    battlegroup::restart(cfg).await
}
