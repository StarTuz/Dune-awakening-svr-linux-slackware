use std::collections::HashMap;

use anyhow::Result;
use serde_json::Value;

use crate::{config::Config, kubectl};

#[derive(Debug, Clone)]
pub struct BattlegroupStatus {
    pub phase: String,
    pub title: Option<String>,
    pub stop: bool,
    pub size: Option<u32>,
    pub start_timestamp: Option<String>,
    pub utilities: Vec<UtilityStatus>,
    pub runtime_servers: Vec<RuntimeServer>,
    pub maps: Vec<MapEntry>,
}

#[derive(Debug, Clone)]
pub struct MapEntry {
    pub name: String,
    pub category: MapCategory,
    /// Live phase from the ServerSet status (Running / Stopped / Starting / …)
    pub phase: String,
    /// 0 = stopped, 1 = desired running (from BattleGroup CR sets[n].replicas)
    pub replicas: u32,
    pub scale_replicas: Option<u32>,
    pub ready_replicas: Option<u32>,
    pub target_replicas: Option<u32>,
    pub partitions: Vec<u32>,
    pub players: Option<u32>,
    pub ready: Option<bool>,
    pub game_port: Option<u16>,
    pub sfps: Option<String>,
    pub memory_request: Option<String>,
    pub memory_limit: Option<String>,
    pub consistency: MapConsistency,
}

/// Query the BattleGroup CR and return phase + map list.
pub async fn status(cfg: &Config) -> Result<BattlegroupStatus> {
    let bg = raw(cfg).await?;

    let phase = bg
        .pointer("/status/phase")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let title = bg
        .pointer("/spec/title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let stop = bg
        .pointer("/spec/stop")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let size = bg.pointer("/status/size").and_then(as_u32);
    let start_timestamp = bg
        .pointer("/status/startTimestamp")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let maps = parse_maps(&bg);
    let utilities = parse_utilities(&bg);
    let runtime_servers = parse_runtime_servers(&bg);
    Ok(BattlegroupStatus {
        phase,
        title,
        stop,
        size,
        start_timestamp,
        utilities,
        runtime_servers,
        maps,
    })
}

pub async fn raw(cfg: &Config) -> Result<Value> {
    kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await
}

pub async fn start(cfg: &Config) -> Result<()> {
    patch_stop(cfg, false).await
}

pub async fn stop(cfg: &Config) -> Result<()> {
    patch_stop(cfg, true).await
}

pub async fn restart(cfg: &Config) -> Result<()> {
    stop(cfg).await?;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    start(cfg).await
}

async fn patch_stop(cfg: &Config, stop: bool) -> Result<()> {
    let patch = format!(r#"{{"spec":{{"stop":{}}}}}"#, stop);
    kubectl::run(&[
        "patch",
        "battlegroup",
        &cfg.battlegroup,
        "-n",
        &cfg.namespace,
        "--type=merge",
        &format!("-p={}", patch),
    ])
    .await?;
    Ok(())
}

/// Enrich MapEntry.phase from live ServerSet statuses.
pub async fn enrich_maps(cfg: &Config, maps: &mut [MapEntry]) -> Result<()> {
    enrich_from_serversets(cfg, maps).await?;
    enrich_from_serversetscales(cfg, maps).await.ok();
    enrich_from_serverstats(cfg, maps).await.ok();

    for map in maps {
        map.consistency = map_consistency(map);
    }
    Ok(())
}

async fn enrich_from_serversets(cfg: &Config, maps: &mut [MapEntry]) -> Result<()> {
    let items = kubectl::get_json(&["get", "serverset", "-n", &cfg.namespace])
        .await?
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
                map.ready_replicas = item.pointer("/status/readyReplicas").and_then(as_u32);
                map.target_replicas = item.pointer("/status/targetReplicas").and_then(as_u32);
                break;
            }
        }
        if map.phase.is_empty() {
            map.phase = "Unknown".to_string();
        }
    }
    Ok(())
}

async fn enrich_from_serversetscales(cfg: &Config, maps: &mut [MapEntry]) -> Result<()> {
    let items = kubectl::get_json(&["get", "serversetscale", "-n", &cfg.namespace])
        .await?
        .pointer("/items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut by_map = HashMap::new();
    for item in items {
        if let Some(name) = item
            .pointer("/metadata/annotations/igw.funcom.com~1map-name")
            .and_then(|v| v.as_str())
        {
            by_map.insert(
                name.to_string(),
                item.pointer("/spec/replicas").and_then(as_u32),
            );
        }
    }

    for map in maps {
        map.scale_replicas = by_map.get(&map.name).copied().flatten();
    }
    Ok(())
}

async fn enrich_from_serverstats(cfg: &Config, maps: &mut [MapEntry]) -> Result<()> {
    let items = kubectl::get_json(&["get", "serverstats", "-n", &cfg.namespace])
        .await?
        .pointer("/items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for item in items {
        let Some(name) = item.pointer("/spec/area/map").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(map) = maps.iter_mut().find(|m| m.name == name) {
            map.players = item.pointer("/status/runtime/players").and_then(as_u32);
            map.ready = item
                .pointer("/status/runtime/ready")
                .and_then(|v| v.as_bool());
            map.sfps = item
                .pointer("/status/runtime/sfps")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
    }
    Ok(())
}

fn parse_maps(bg: &Value) -> Vec<MapEntry> {
    let ports_by_map: HashMap<String, u16> = bg
        .pointer("/status/servers")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|server| {
            let map = server.get("partitionMap")?.as_str()?.to_string();
            let port = as_u16(server.get("gamePort")?)?;
            Some((map, port))
        })
        .collect();

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
            let partitions = s
                .get("partitions")
                .and_then(|v| v.as_array())
                .map(|items| items.iter().filter_map(as_u32).collect())
                .unwrap_or_default();
            Some(MapEntry {
                category: MapCategory::from_map_name(&name),
                game_port: ports_by_map.get(&name).copied(),
                memory_request: s.pointer("/resources/requests/memory").and_then(str_value),
                memory_limit: s.pointer("/resources/limits/memory").and_then(str_value),
                name,
                replicas,
                phase: String::new(),
                scale_replicas: None,
                ready_replicas: None,
                target_replicas: None,
                partitions,
                players: None,
                ready: None,
                sfps: None,
                consistency: MapConsistency::Unknown,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapCategory {
    Core,
    Social,
    DeepDesert,
    Story,
    Dungeon,
    Dlc,
    Other,
}

impl MapCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Core => "Core",
            Self::Social => "Social",
            Self::DeepDesert => "Deep Desert",
            Self::Story => "Story",
            Self::Dungeon => "Dungeon",
            Self::Dlc => "DLC",
            Self::Other => "Other",
        }
    }

    fn from_map_name(name: &str) -> Self {
        if matches!(name, "Survival_1" | "Overmap") {
            Self::Core
        } else if name == "DeepDesert_1" {
            Self::DeepDesert
        } else if name.starts_with("SH_") {
            Self::Social
        } else if name.starts_with("DLC_") {
            Self::Dlc
        } else if name.starts_with("CB_Dungeon_") || name.starts_with("CB_Ecolab_") {
            Self::Dungeon
        } else if name.starts_with("Story_") || name.starts_with("CB_Story_") {
            Self::Story
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapConsistency {
    CleanOn,
    CleanOff,
    Starting,
    Stopping,
    Split,
    Unknown,
}

impl MapConsistency {
    pub fn label(self) -> &'static str {
        match self {
            Self::CleanOn => "clean on",
            Self::CleanOff => "clean off",
            Self::Starting => "starting",
            Self::Stopping => "stopping",
            Self::Split => "split",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeServer {
    pub map: String,
    pub partition: Option<u32>,
    pub port: Option<u16>,
    pub ready: bool,
    pub phase: String,
    pub restarts: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct UtilityStatus {
    pub name: String,
    pub phase: String,
    pub address: Option<String>,
}

fn parse_runtime_servers(bg: &Value) -> Vec<RuntimeServer> {
    bg.pointer("/status/servers")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|server| {
            Some(RuntimeServer {
                map: server.get("partitionMap")?.as_str()?.to_string(),
                partition: server.get("partitionIndex").and_then(as_u32),
                port: server.get("gamePort").and_then(as_u16),
                ready: server
                    .get("ready")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                phase: server
                    .get("phase")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string(),
                restarts: server.get("restarts").and_then(as_u32),
            })
        })
        .collect()
}

fn parse_utilities(bg: &Value) -> Vec<UtilityStatus> {
    let mut out = Vec::new();
    push_utility(&mut out, "Database", bg.pointer("/status/database"));
    push_utility(
        &mut out,
        "Director",
        bg.pointer("/status/utilities/director"),
    );
    push_utility(
        &mut out,
        "Gateway",
        bg.pointer("/status/utilities/serverGateway"),
    );
    push_utility(
        &mut out,
        "Text Router",
        bg.pointer("/status/utilities/textRouter"),
    );
    push_utility(
        &mut out,
        "Filebrowser",
        bg.pointer("/status/utilities/fileBrowser"),
    );

    if let Some(statuses) = bg
        .pointer("/status/utilities/messageQueues/statuses")
        .and_then(|v| v.as_object())
    {
        for (name, value) in statuses {
            push_utility(&mut out, &format!("RMQ {}", name), Some(value));
        }
    }
    out
}

fn push_utility(out: &mut Vec<UtilityStatus>, name: &str, value: Option<&Value>) {
    let Some(value) = value else {
        return;
    };
    let address = value
        .get("address")
        .or_else(|| value.get("amqpAddress"))
        .or_else(|| value.get("managementAddress"))
        .and_then(str_value);
    out.push(UtilityStatus {
        name: name.to_string(),
        phase: value
            .get("phase")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string(),
        address,
    });
}

fn map_consistency(map: &MapEntry) -> MapConsistency {
    match (map.replicas, map.scale_replicas) {
        (0, Some(0)) => MapConsistency::CleanOff,
        (1, Some(1)) if map.ready_replicas.unwrap_or(0) >= 1 => MapConsistency::CleanOn,
        (1, Some(1)) => MapConsistency::Starting,
        (0, Some(1)) => MapConsistency::Stopping,
        (bg, Some(scale)) if bg != scale => MapConsistency::Split,
        (0, None) if map.phase == "Stopped" => MapConsistency::CleanOff,
        (1, None) if map.phase == "Running" => MapConsistency::CleanOn,
        _ => MapConsistency::Unknown,
    }
}

fn as_u32(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| n.try_into().ok())
}

fn as_u16(v: &Value) -> Option<u16> {
    v.as_u64().and_then(|n| n.try_into().ok())
}

fn str_value(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}
