use anyhow::Result;

use crate::{config::Config, kubectl};

pub async fn start(cfg: &Config, map_name: &str) -> Result<()> {
    toggle(cfg, map_name, 1).await
}

pub async fn stop(cfg: &Config, map_name: &str) -> Result<()> {
    toggle(cfg, map_name, 0).await
}

async fn toggle(cfg: &Config, map_name: &str, replicas: u32) -> Result<()> {
    // Find the set index in the BattleGroup CR (mirrors map-toggle.sh INDEX derivation)
    let bg = kubectl::get_json(&[
        "get", "battlegroup", &cfg.battlegroup,
        "-n", &cfg.namespace,
    ])
    .await?;

    let idx = find_map_index(&bg, map_name)
        .ok_or_else(|| anyhow::anyhow!("map '{}' not found in BattleGroup CR", map_name))?;

    // 1. Patch BattleGroup CR — propagates down to ServerGroup + ServerSet
    let patch = format!(
        r#"[{{"op":"replace","path":"/spec/serverGroup/template/spec/sets/{}/replicas","value":{}}}]"#,
        idx, replicas
    );
    kubectl::run(&[
        "patch", "battlegroup", &cfg.battlegroup,
        "-n", &cfg.namespace,
        "--type=json", &format!("-p={}", patch),
    ])
    .await?;

    // 2. Patch ServerSetScale if it exists — the final pod-creation trigger.
    // Name convention (from map-toggle.sh): ${BG}-${MAP_SLUG}
    let scale_name = format!("{}-{}", cfg.battlegroup, map_slug(map_name));
    let scale_exists = kubectl::run(&[
        "get", "serversetscale", &scale_name,
        "-n", &cfg.namespace,
    ])
    .await
    .is_ok();

    if scale_exists {
        let scale_patch = format!(
            r#"[{{"op":"replace","path":"/spec/replicas","value":{}}}]"#,
            replicas
        );
        kubectl::run(&[
            "patch", "serversetscale", &scale_name,
            "-n", &cfg.namespace,
            "--type=json", &format!("-p={}", scale_patch),
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

/// "DeepDesert_1" → "deepdesert-1"  (mirrors map-toggle.sh MAP_SLUG derivation)
fn map_slug(name: &str) -> String {
    name.to_lowercase().replace('_', "-")
}
