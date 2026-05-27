use std::time::{Duration, Instant};

use anyhow::Result;

use crate::{backup, battlegroup, config::Config};

#[derive(Debug, Clone, Copy)]
pub struct ShutdownOptions {
    pub skip_backup: bool,
    pub timeout_secs: u64,
}

impl Default for ShutdownOptions {
    fn default() -> Self {
        Self {
            skip_backup: false,
            timeout_secs: 300,
        }
    }
}

pub async fn shutdown_for_reboot_streamed(
    cfg: &Config,
    options: ShutdownOptions,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    send(&tx, format!("World       : {}", world_label(cfg)));
    send(&tx, format!("Battlegroup : {}", cfg.battlegroup));
    send(&tx, format!("Namespace   : {}", cfg.namespace));

    if options.skip_backup {
        send(&tx, "Backup      : skipped by request");
    } else {
        send(&tx, "Backup      : starting full backup");
        backup::run_streamed(cfg, false, None, tx.clone()).await?;
        send(&tx, "Backup      : complete");
    }

    send(&tx, "Shutdown    : requesting BattleGroup stop");
    battlegroup::stop(cfg).await?;
    send(&tx, "Shutdown    : waiting for game servers to stop");
    wait_stopped(cfg, options.timeout_secs, &tx).await?;
    send(&tx, "Shutdown    : Dune world is stopped");
    send(&tx, "Host reboot : safe to run the host reboot command now");
    Ok(())
}

async fn wait_stopped(
    cfg: &Config,
    timeout_secs: u64,
    tx: &tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut last_summary = String::new();

    loop {
        let mut status = battlegroup::status(cfg).await?;
        battlegroup::enrich_maps(cfg, &mut status.maps).await.ok();

        let active_maps = status
            .maps
            .iter()
            .filter(|map| {
                map.replicas > 0
                    || map.ready_replicas.unwrap_or_default() > 0
                    || matches!(map.phase.as_str(), "Running" | "Starting")
            })
            .count();
        let runtime_servers = status.runtime_servers.len();
        let summary = format!(
            "phase={} stop={} active_maps={} runtime_servers={}",
            status.phase, status.stop, active_maps, runtime_servers
        );

        if summary != last_summary {
            send(tx, format!("Shutdown    : {}", summary));
            last_summary = summary;
        }

        if status.stop && active_maps == 0 && runtime_servers == 0 {
            return Ok(());
        }

        if Instant::now() >= deadline {
            anyhow::bail!(
                "timed out waiting for battlegroup {} to stop; last state: {}",
                cfg.battlegroup,
                last_summary
            );
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn world_label(cfg: &Config) -> &str {
    cfg.title.as_deref().unwrap_or(&cfg.battlegroup)
}

fn send(tx: &tokio::sync::mpsc::UnboundedSender<String>, line: impl Into<String>) {
    let _ = tx.send(line.into());
}
