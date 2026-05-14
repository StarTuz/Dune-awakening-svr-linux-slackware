use anyhow::Result;

use crate::{config::Config, kubectl};

#[derive(Debug, Clone)]
pub struct BattlegroupStatus {
    pub phase: String,
    pub maps: Vec<MapEntry>,
}

#[derive(Debug, Clone)]
pub struct MapEntry {
    pub name: String,
    /// Live phase from the ServerSet status (Running / Stopped / Starting / …)
    pub phase: String,
    /// 0 = stopped, 1 = desired running (from BattleGroup CR sets[n].replicas)
    pub replicas: u32,
}

/// Query the BattleGroup CR and return phase + map list.
pub async fn status(cfg: &Config) -> Result<BattlegroupStatus> {
    let bg = kubectl::get_json(&[
        "get", "battlegroup", &cfg.battlegroup,
        "-n", &cfg.namespace,
    ])
    .await?;

    let phase = bg
        .pointer("/status/phase")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();

    let maps = parse_maps(&bg);
    Ok(BattlegroupStatus { phase, maps })
}

/// Enrich MapEntry.phase from live ServerSet statuses.
pub async fn enrich_phases(cfg: &Config, maps: &mut Vec<MapEntry>) -> Result<()> {
    let list = kubectl::get_json(&["get", "serverset", "-n", &cfg.namespace]).await?;
    let items = list
        .pointer("/items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for map in maps.iter_mut() {
        for item in &items {
            if item.pointer("/spec/map").and_then(|v| v.as_str()) == Some(&map.name) {
                map.phase = item
                    .pointer("/status/phase")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string();
                break;
            }
        }
        if map.phase.is_empty() {
            map.phase = "Unknown".to_string();
        }
    }
    Ok(())
}

fn parse_maps(bg: &serde_json::Value) -> Vec<MapEntry> {
    let Some(sets) = bg
        .pointer("/spec/serverGroup/template/spec/sets")
        .and_then(|v| v.as_array())
    else {
        return vec![];
    };
    sets.iter()
        .filter_map(|s| {
            let name = s.get("map")?.as_str()?.to_string();
            let replicas = s.get("replicas").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            Some(MapEntry { name, replicas, phase: String::new() })
        })
        .collect()
}
