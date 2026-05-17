use anyhow::Result;
use serde_json::json;

use crate::{config::Config, kubectl};

pub async fn start(cfg: &Config, map_name: &str, force: bool) -> Result<()> {
    if map_name.starts_with("SH_") && !force {
        anyhow::bail!(
            "'{}' is a director-managed social hub.\n\
             Social hubs are allocated on demand when a player travels to them — \
             forcing one up via 'maps start' bypasses the director session handshake \
             and the transition will fail or be immediately shut down (MinServers=0).\n\
             Travel to the hub in-game instead; the director will start it automatically.\n\
             Use --force to override this guard.",
            map_name
        );
    }
    toggle(cfg, map_name, 1).await
}

pub async fn stop(cfg: &Config, map_name: &str) -> Result<()> {
    toggle(cfg, map_name, 0).await
}

async fn toggle(cfg: &Config, map_name: &str, replicas: u32) -> Result<()> {
    // Find the set index in the BattleGroup CR (mirrors map-toggle.sh INDEX derivation)
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;

    let idx = find_map_index(&bg, map_name)
        .ok_or_else(|| anyhow::anyhow!("map '{}' not found in BattleGroup CR", map_name))?;

    // 1. Patch BattleGroup CR — propagates down to ServerGroup + ServerSet.
    // Starting a map must also carry the stable world partition IDs; otherwise
    // dedicated maps can launch without -PartitionIndex and crash or stay
    // permanently unready.
    let mut partitions = None;
    let mut patch = Vec::new();
    if replicas == 1 {
        let map_partitions = world_partitions(&bg, map_name).ok_or_else(|| {
            anyhow::anyhow!("no enabled world partition IDs found for '{}'", map_name)
        })?;
        patch.push(json!({
            "op": "replace",
            "path": format!("/spec/serverGroup/template/spec/sets/{}/partitions", idx),
            "value": map_partitions,
        }));

        if map_partitions.len() == 1 && !has_partition_arg(&bg, idx) {
            patch.push(json!({
                "op": "add",
                "path": format!("/spec/serverGroup/template/spec/sets/{}/arguments/-", idx),
                "value": format!("-PartitionIndex={}", map_partitions[0]),
            }));
        }
        partitions = Some(map_partitions);
    }
    patch.push(json!({
        "op": "replace",
        "path": format!("/spec/serverGroup/template/spec/sets/{}/replicas", idx),
        "value": replicas,
    }));
    let patch = serde_json::to_string(&patch)?;
    kubectl::run(&[
        "patch",
        "battlegroup",
        &cfg.battlegroup,
        "-n",
        &cfg.namespace,
        "--type=json",
        &format!("-p={}", patch),
    ])
    .await?;

    // 2. Patch ServerSetScale if it exists — the final pod-creation trigger.
    // Name convention (from map-toggle.sh): ${BG}-${MAP_SLUG}
    let scale_name = format!("{}-{}", cfg.battlegroup, map_slug(map_name));
    let scale_exists = kubectl::run(&["get", "serversetscale", &scale_name, "-n", &cfg.namespace])
        .await
        .is_ok();

    if scale_exists {
        let mut scale_patch = Vec::new();
        if let Some(partitions) = partitions {
            scale_patch.push(json!({
                "op": "add",
                "path": "/spec/partitions",
                "value": partitions,
            }));
        }
        scale_patch.push(json!({
            "op": "replace",
            "path": "/spec/replicas",
            "value": replicas,
        }));
        let scale_patch = serde_json::to_string(&scale_patch)?;
        kubectl::run(&[
            "patch",
            "serversetscale",
            &scale_name,
            "-n",
            &cfg.namespace,
            "--type=json",
            &format!("-p={}", scale_patch),
        ])
        .await?;
    }
    Ok(())
}

fn find_map_index(bg: &serde_json::Value, map_name: &str) -> Option<usize> {
    bg.pointer("/spec/serverGroup/template/spec/sets")?
        .as_array()?
        .iter()
        .position(|s| s.get("map").and_then(|v| v.as_str()) == Some(map_name))
}

fn world_partitions(bg: &serde_json::Value, map_name: &str) -> Option<Vec<u32>> {
    let partitions: Vec<u32> = bg
        .pointer("/spec/database/template/spec/deployment/spec/worldPartitions")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("map").and_then(|v| v.as_str()) == Some(map_name))?
        .get("partitions")?
        .as_array()?
        .iter()
        .filter(|partition| {
            !partition
                .get("disable")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        })
        .filter_map(|partition| partition.get("id").and_then(as_u32))
        .collect();
    (!partitions.is_empty()).then_some(partitions)
}

fn has_partition_arg(bg: &serde_json::Value, idx: usize) -> bool {
    bg.pointer(&format!(
        "/spec/serverGroup/template/spec/sets/{}/arguments",
        idx
    ))
    .and_then(|value| value.as_array())
    .into_iter()
    .flatten()
    .filter_map(|value| value.as_str())
    .any(|arg| arg.starts_with("-PartitionIndex="))
}

fn as_u32(value: &serde_json::Value) -> Option<u32> {
    value.as_u64().and_then(|n| n.try_into().ok())
}

/// "DeepDesert_1" → "deepdesert-1"  (mirrors map-toggle.sh MAP_SLUG derivation)
fn map_slug(name: &str) -> String {
    name.to_lowercase().replace('_', "-")
}
