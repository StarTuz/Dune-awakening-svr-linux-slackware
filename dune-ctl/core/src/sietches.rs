use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use crate::{battlegroup, config::Config, kubectl};

pub const PRIMARY_SIETCH_MAP: &str = "Survival_1";

/// Sietch capacity for the primary Sietch map, per the Battlegroup Editor model:
/// the maximum number of Sietches a map can run equals its enabled
/// `worldPartitions` count, and the number actually started (`active`) must be
/// `<= max`. A bare `replicas` bump beyond `max` crash-loops on
/// `load_world_partition ... got 0 rows`, so this is the figure that gates adding
/// Sietches. See `SIETCHES-DESIGN.md`.
#[derive(Debug, Clone)]
pub struct SietchCapacity {
    pub map: String,
    /// Enabled `worldPartitions` count = max Sietches this map can run.
    pub max: usize,
    /// `sets[i].replicas` = Sietches currently started.
    pub active: u32,
}

/// Read the primary Sietch capacity (active vs. max) from the live BattleGroup CR.
pub async fn capacity(cfg: &Config) -> Result<SietchCapacity> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;
    Ok(SietchCapacity {
        map: PRIMARY_SIETCH_MAP.to_string(),
        max: enabled_partition_count(&bg, PRIMARY_SIETCH_MAP),
        active: set_replicas(&bg, PRIMARY_SIETCH_MAP),
    })
}

/// Count of enabled (`disable != true`) `worldPartitions` entries for `map`.
fn enabled_partition_count(bg: &Value, map: &str) -> usize {
    bg.pointer("/spec/database/template/spec/deployment/spec/worldPartitions")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .find(|e| e.get("map").and_then(|m| m.as_str()) == Some(map))
        .and_then(|e| e.get("partitions"))
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter(|p| {
                    !p.get("disable")
                        .and_then(|d| d.as_bool())
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

/// `sets[i].replicas` for `map` (active Sietches), 0 if absent.
fn set_replicas(bg: &Value, map: &str) -> u32 {
    bg.pointer("/spec/serverGroup/template/spec/sets")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
        .find(|s| s.get("map").and_then(|m| m.as_str()) == Some(map))
        .and_then(|s| s.get("replicas"))
        .and_then(|r| r.as_u64())
        .unwrap_or(0) as u32
}

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

/// Launch the Battlegroup Editor on the selected world's BattleGroup CR.
///
/// This is Funcom's "Battlegroup Editor": `KUBE_EDITOR=<bg-util> kubectl edit
/// battlegroup ...` (mirrors `server/scripts/battlegroup.sh::edit_battlegroup`).
/// `bg-util` is a Funcom TUI for editing **dimensions** (Sietches / world
/// partitions), per-Sietch display names and passwords, and per-map memory
/// limits. The key invariants it enforces: a map's max Sietches = its
/// `worldPartitions` count, and active `replicas` must be ≤ that count.
///
/// `advanced` skips `bg-util` and opens the raw CR YAML in the default editor
/// (mirrors `edit_battlegroup_advanced`).
///
/// This is the safe wrapper (Phase 0); native `add`/`remove`/`scale`/`rename`
/// land later (see `SIETCHES-DESIGN.md`). Inherits the terminal so the TUI works.
pub async fn edit(cfg: &Config, advanced: bool) -> Result<()> {
    let mut cmd = Command::new("sudo");
    if !advanced {
        let editor = bg_util_path(cfg)?;
        cmd.arg(format!("KUBE_EDITOR={}", editor.display()));
    }
    cmd.args([
        "kubectl",
        "edit",
        "battlegroup",
        &cfg.battlegroup,
        "-n",
        &cfg.namespace,
    ]);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd
        .status()
        .await
        .context("failed to launch the Battlegroup Editor (kubectl edit)")?;
    if !status.success() {
        anyhow::bail!("Battlegroup Editor exited with status {}", status);
    }
    Ok(())
}

/// Resolve the `bg-util` editor binary: installed symlink first, then in-repo copy.
fn bg_util_path(cfg: &Config) -> Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/dune".into());
    let candidates = [
        PathBuf::from(&home).join(".dune/bin/bg-util"),
        cfg.repo_root().join("server/scripts/bg-util"),
    ];
    candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "bg-util (Battlegroup Editor) not found at ~/.dune/bin/bg-util or \
                 <repo>/server/scripts/bg-util; use 'sietches edit --advanced' for the raw editor"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bg(partitions: serde_json::Value, replicas: u64) -> Value {
        json!({
            "spec": {
                "database": { "template": { "spec": { "deployment": { "spec": {
                    "worldPartitions": [
                        { "map": "Survival_1", "partitions": partitions },
                        { "map": "Overmap", "partitions": [ { "id": 2 } ] }
                    ]
                }}}}},
                "serverGroup": { "template": { "spec": { "sets": [
                    { "map": "Survival_1", "replicas": replicas },
                    { "map": "Overmap", "replicas": 1 }
                ]}}}
            }
        })
    }

    #[test]
    fn single_partition_single_replica_is_one_sietch() {
        let v = bg(json!([{ "id": 1, "disable": false }]), 1);
        assert_eq!(enabled_partition_count(&v, "Survival_1"), 1);
        assert_eq!(set_replicas(&v, "Survival_1"), 1);
    }

    #[test]
    fn disabled_partitions_do_not_count_toward_max() {
        let v = bg(
            json!([{ "id": 1 }, { "id": 2, "disable": true }, { "id": 3, "disable": false }]),
            2,
        );
        // ids 1 and 3 enabled; id 2 disabled → max 2.
        assert_eq!(enabled_partition_count(&v, "Survival_1"), 2);
        assert_eq!(set_replicas(&v, "Survival_1"), 2);
    }

    #[test]
    fn missing_map_yields_zero() {
        let v = bg(json!([{ "id": 1 }]), 1);
        assert_eq!(enabled_partition_count(&v, "DeepDesert_1"), 0);
        assert_eq!(set_replicas(&v, "DeepDesert_1"), 0);
    }
}
