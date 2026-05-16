use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::{config::Config, kubectl};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFile {
    Engine,
    Game,
}

impl SettingsFile {
    pub fn filename(self) -> &'static str {
        match self {
            Self::Engine => "UserEngine.ini",
            Self::Game => "UserGame.ini",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Engine => "Engine",
            Self::Game => "Game",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    BoolInt,
    BoolLower,
    BoolTitle,
    Float,
    Integer,
    QuotedString,
}

#[derive(Debug, Clone, Copy)]
pub struct SettingDef {
    pub key: &'static str,
    pub label: &'static str,
    pub file: SettingsFile,
    pub section: &'static str,
    pub ini_key: &'static str,
    pub kind: ValueKind,
    pub secret: bool,
}

#[derive(Debug, Clone)]
pub struct SettingValue {
    pub def: SettingDef,
    pub value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SettingDrift {
    pub def: SettingDef,
    pub local: Option<String>,
    pub deployed: Option<String>,
}

impl SettingDrift {
    pub fn changed(&self) -> bool {
        self.local != self.deployed
    }
}

#[derive(Debug, Clone)]
pub struct SettingsDrift {
    pub items: Vec<SettingDrift>,
    pub deployed_available: bool,
    pub error: Option<String>,
}

impl SettingsDrift {
    pub fn changed_count(&self) -> usize {
        self.items.iter().filter(|item| item.changed()).count()
    }
}

pub const CATALOG: &[SettingDef] = &[
    SettingDef {
        key: "port",
        label: "Player UDP base port",
        file: SettingsFile::Engine,
        section: "URL",
        ini_key: "Port",
        kind: ValueKind::Integer,
        secret: false,
    },
    SettingDef {
        key: "igw_port",
        label: "Server-to-server UDP base port",
        file: SettingsFile::Engine,
        section: "URL",
        ini_key: "IGWPort",
        kind: ValueKind::Integer,
        secret: false,
    },
    SettingDef {
        key: "sietch_name",
        label: "Sietch display name",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "Bgd.ServerDisplayName",
        kind: ValueKind::QuotedString,
        secret: false,
    },
    SettingDef {
        key: "sietch_password",
        label: "Sietch login password",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "Bgd.ServerLoginPassword",
        kind: ValueKind::QuotedString,
        secret: true,
    },
    SettingDef {
        key: "mining_output",
        label: "Player mining output multiplier",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "Dune.GlobalMiningOutputMultiplier",
        kind: ValueKind::Float,
        secret: false,
    },
    SettingDef {
        key: "vehicle_mining_output",
        label: "Vehicle mining output multiplier",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "Dune.GlobalVehicleMiningOutputMultiplier",
        kind: ValueKind::Float,
        secret: false,
    },
    SettingDef {
        key: "pvp_resource_multiplier",
        label: "PvP resource multiplier",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "SecurityZones.PvpResourceMultiplier",
        kind: ValueKind::Float,
        secret: false,
    },
    SettingDef {
        key: "vehicle_durability_damage",
        label: "Vehicle durability damage multiplier",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "dw.VehicleDurabilityDamageMultiplier",
        kind: ValueKind::Float,
        secret: false,
    },
    SettingDef {
        key: "sandstorm",
        label: "Sandstorm enabled",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "Sandstorm.Enabled",
        kind: ValueKind::BoolInt,
        secret: false,
    },
    SettingDef {
        key: "sandstorm_treasure",
        label: "Sandstorm treasure enabled",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "Sandstorm.Treasure.Enabled",
        kind: ValueKind::BoolInt,
        secret: false,
    },
    SettingDef {
        key: "sandworm",
        label: "Sandworm enabled",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "sandworm.dune.Enabled",
        kind: ValueKind::BoolInt,
        secret: false,
    },
    SettingDef {
        key: "vehicle_worm_collision",
        label: "Sandworm vehicle collision",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "Vehicle.SandwormCollisionInteraction",
        kind: ValueKind::BoolLower,
        secret: false,
    },
    SettingDef {
        key: "worm_danger_zones",
        label: "Sandworm danger zones",
        file: SettingsFile::Engine,
        section: "ConsoleVariables",
        ini_key: "Sandworm.SandwormDangerZonesEnabled",
        kind: ValueKind::BoolLower,
        secret: false,
    },
    SettingDef {
        key: "pvp_all",
        label: "Force PvP on all partitions",
        file: SettingsFile::Game,
        section: "/Script/DuneSandbox.PvpPveSettings",
        ini_key: "m_bShouldForceEnablePvpOnAllPartitions",
        kind: ValueKind::BoolTitle,
        secret: false,
    },
    SettingDef {
        key: "security_zones",
        label: "Security zones enabled",
        file: SettingsFile::Game,
        section: "/Script/DuneSandbox.SecurityZonesSubsystem",
        ini_key: "m_bAreSecurityZonesEnabled",
        kind: ValueKind::BoolTitle,
        secret: false,
    },
    SettingDef {
        key: "item_deterioration_rate",
        label: "Item deterioration update rate",
        file: SettingsFile::Game,
        section: "/DeteriorationSystem.ItemDeteriorationConstants",
        ini_key: "UpdateRateInSeconds",
        kind: ValueKind::Float,
        secret: false,
    },
    SettingDef {
        key: "coriolis_storm",
        label: "Coriolis storm auto spawn",
        file: SettingsFile::Game,
        section: "/Script/DuneSandbox.SandStormConfig",
        ini_key: "m_bCoriolisAutoSpawnEnabled",
        kind: ValueKind::BoolTitle,
        secret: false,
    },
    SettingDef {
        key: "landclaim_segments",
        label: "Max landclaim segments",
        file: SettingsFile::Game,
        section: "/Script/DuneSandbox.BuildingSettings",
        ini_key: "m_MaxNumLandclaimSegments",
        kind: ValueKind::Integer,
        secret: false,
    },
    SettingDef {
        key: "building_restrictions",
        label: "Building restriction limits",
        file: SettingsFile::Game,
        section: "/Script/DuneSandbox.BuildingSettings",
        ini_key: "m_bBuildingRestrictionLimitsEnabled",
        kind: ValueKind::BoolTitle,
        secret: false,
    },
];

pub fn catalog() -> &'static [SettingDef] {
    CATALOG
}

pub fn kind_label(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::BoolInt | ValueKind::BoolLower | ValueKind::BoolTitle => "bool",
        ValueKind::Float => "float",
        ValueKind::Integer => "int",
        ValueKind::QuotedString => "string",
    }
}

pub async fn list(cfg: &Config) -> Result<Vec<SettingValue>> {
    let mut out = Vec::with_capacity(CATALOG.len());
    for def in CATALOG {
        let text = tokio::fs::read_to_string(setting_path(cfg, def.file))
            .await
            .with_context(|| format!("failed to read {}", def.file.filename()))?;
        out.push(SettingValue {
            def: *def,
            value: get_value(&text, def.section, def.ini_key),
        });
    }
    Ok(out)
}

pub async fn set(cfg: &Config, key: &str, value: &str) -> Result<()> {
    let def = find_def(key)?;
    let value = normalize_value(def.kind, value)?;
    let path = setting_path(cfg, def.file);
    let text = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    let updated = set_value(&text, def.section, def.ini_key, &value)
        .with_context(|| format!("failed to update {} in {}", def.ini_key, path.display()))?;
    tokio::fs::write(&path, updated)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub async fn toggle(cfg: &Config, key: &str) -> Result<String> {
    let def = find_def(key)?;
    if !is_bool(def.kind) {
        anyhow::bail!("{} is not a boolean setting", key);
    }
    let values = list(cfg).await?;
    let current = values
        .iter()
        .find(|v| v.def.key == key)
        .and_then(|v| v.value.as_deref())
        .ok_or_else(|| anyhow::anyhow!("{} has no current value", key))?;
    let next = if parse_bool(current).unwrap_or(false) {
        "false"
    } else {
        "true"
    };
    set(cfg, key, next).await?;
    Ok(normalize_value(def.kind, next)?)
}

pub async fn apply(cfg: &Config) -> Result<()> {
    let pod = filebrowser_pod(cfg).await?;
    kubectl::run(&[
        "exec",
        "-n",
        &cfg.namespace,
        &pod,
        "--",
        "mkdir",
        "-p",
        "/srv/UserSettings",
    ])
    .await?;

    for file in [SettingsFile::Engine, SettingsFile::Game] {
        let src = setting_path(cfg, file);
        let src = src.to_string_lossy().to_string();
        let dst = format!(
            "{}/{}:/srv/UserSettings/{}",
            cfg.namespace,
            pod,
            file.filename()
        );
        kubectl::run(&["cp", &src, &dst]).await?;
    }
    Ok(())
}

pub async fn pull_deployed(cfg: &Config) -> Result<()> {
    let pod = filebrowser_pod(cfg).await?;
    tokio::fs::create_dir_all(cfg.user_settings_dir())
        .await
        .with_context(|| format!("failed to create {}", cfg.user_settings_dir().display()))?;

    for file in [SettingsFile::Engine, SettingsFile::Game] {
        let src = format!(
            "{}/{}:/srv/UserSettings/{}",
            cfg.namespace,
            pod,
            file.filename()
        );
        let dst = setting_path(cfg, file).to_string_lossy().to_string();
        kubectl::run(&["cp", &src, &dst]).await?;
    }
    Ok(())
}

pub async fn diff(cfg: &Config) -> Result<String> {
    let pod = filebrowser_pod(cfg).await?;
    let mut out = String::new();
    for file in [SettingsFile::Engine, SettingsFile::Game] {
        let local = tokio::fs::read_to_string(setting_path(cfg, file))
            .await
            .with_context(|| format!("failed to read local {}", file.filename()))?;
        let remote = kubectl::run(&[
            "exec",
            "-n",
            &cfg.namespace,
            &pod,
            "--",
            "cat",
            &format!("/srv/UserSettings/{}", file.filename()),
        ])
        .await
        .unwrap_or_else(|e| format!("<unavailable: {}>", e));

        out.push_str(&format!("=== {} ===\n", file.filename()));
        if local == remote {
            out.push_str("No differences detected.\n\n");
        } else {
            let mut changed = 0;
            for def in CATALOG.iter().filter(|def| def.file == file) {
                let local_value = get_value(&local, def.section, def.ini_key)
                    .unwrap_or_else(|| "<missing>".to_string());
                let remote_value = get_value(&remote, def.section, def.ini_key)
                    .unwrap_or_else(|| "<missing>".to_string());
                if local_value != remote_value {
                    changed += 1;
                    out.push_str(&format!(
                        "{:<28} deployed={:<12} local={}\n",
                        def.key, remote_value, local_value
                    ));
                }
            }
            if changed == 0 {
                out.push_str("File text differs, but catalogued settings match.\n");
            }
            out.push('\n');
        }
    }
    Ok(out)
}

pub async fn drift(cfg: &Config) -> Result<SettingsDrift> {
    let pod = filebrowser_pod(cfg).await?;
    let mut items = Vec::with_capacity(CATALOG.len());
    let mut deployed_available = true;
    let mut error = None;

    for file in [SettingsFile::Engine, SettingsFile::Game] {
        let local = tokio::fs::read_to_string(setting_path(cfg, file))
            .await
            .with_context(|| format!("failed to read local {}", file.filename()))?;
        let deployed = match kubectl::run(&[
            "exec",
            "-n",
            &cfg.namespace,
            &pod,
            "--",
            "cat",
            &format!("/srv/UserSettings/{}", file.filename()),
        ])
        .await
        {
            Ok(text) => Some(text),
            Err(e) => {
                deployed_available = false;
                error.get_or_insert_with(|| e.to_string());
                None
            }
        };

        for def in CATALOG.iter().copied().filter(|def| def.file == file) {
            items.push(SettingDrift {
                def,
                local: get_value(&local, def.section, def.ini_key),
                deployed: deployed
                    .as_deref()
                    .and_then(|text| get_value(text, def.section, def.ini_key)),
            });
        }
    }

    Ok(SettingsDrift {
        items,
        deployed_available,
        error,
    })
}

pub fn setting_path(cfg: &Config, file: SettingsFile) -> PathBuf {
    cfg.user_settings_dir().join(file.filename())
}

pub fn is_bool(kind: ValueKind) -> bool {
    matches!(
        kind,
        ValueKind::BoolInt | ValueKind::BoolLower | ValueKind::BoolTitle
    )
}

pub fn display_value(item: &SettingValue) -> String {
    display_def_value(&item.def, item.value.as_deref())
}

pub fn display_drift_local(item: &SettingDrift) -> String {
    display_def_value(&item.def, item.local.as_deref())
}

pub fn display_drift_deployed(item: &SettingDrift) -> String {
    display_def_value(&item.def, item.deployed.as_deref())
}

pub fn display_def_value(def: &SettingDef, value: Option<&str>) -> String {
    let Some(value) = value else {
        return "—".to_string();
    };
    if !def.secret {
        if def.kind == ValueKind::QuotedString {
            let unquoted = unquote_display(value);
            return if unquoted.is_empty() {
                "—".to_string()
            } else {
                unquoted.to_string()
            };
        }
        return value.to_string();
    }
    let unquoted = unquote_display(value);
    if unquoted.is_empty() {
        "none".to_string()
    } else {
        "********".to_string()
    }
}

fn unquote_display(value: &str) -> &str {
    value
        .trim()
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value.trim())
}

fn find_def(key: &str) -> Result<SettingDef> {
    CATALOG
        .iter()
        .copied()
        .find(|def| def.key == key)
        .ok_or_else(|| anyhow::anyhow!("unknown setting '{}'", key))
}

fn normalize_value(kind: ValueKind, value: &str) -> Result<String> {
    match kind {
        ValueKind::BoolInt => Ok(if parse_bool(value)
            .ok_or_else(|| anyhow::anyhow!("expected boolean"))?
        {
            "1"
        } else {
            "0"
        }
        .to_string()),
        ValueKind::BoolLower => Ok(if parse_bool(value)
            .ok_or_else(|| anyhow::anyhow!("expected boolean"))?
        {
            "true"
        } else {
            "false"
        }
        .to_string()),
        ValueKind::BoolTitle => Ok(if parse_bool(value)
            .ok_or_else(|| anyhow::anyhow!("expected boolean"))?
        {
            "True"
        } else {
            "False"
        }
        .to_string()),
        ValueKind::Float => {
            value.parse::<f64>().context("expected numeric value")?;
            Ok(value.to_string())
        }
        ValueKind::Integer => {
            value.parse::<i64>().context("expected integer value")?;
            Ok(value.to_string())
        }
        ValueKind::QuotedString => quote_string_value(value),
    }
}

fn quote_string_value(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(trimmed);
    if unquoted.contains('\'') || unquoted.contains('|') {
        anyhow::bail!("single quote and pipe characters are not allowed");
    }
    if unquoted.contains('"') {
        anyhow::bail!("double quote characters are not allowed inside this value");
    }
    Ok(format!("\"{}\"", unquoted))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn get_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if section_header(trimmed).is_some() {
            in_section = section_header(trimmed) == Some(section);
            continue;
        }
        if in_section {
            let Some((line_key, value)) = active_assignment(trimmed) else {
                continue;
            };
            if line_key == key {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

fn set_value(text: &str, section: &str, key: &str, value: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut in_section = false;
    let mut found_section = false;
    let mut replaced = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(name) = section_header(trimmed) {
            if in_section && !replaced {
                out.push(format!("{}={}", key, value));
                replaced = true;
            }
            in_section = name == section;
            found_section |= in_section;
            out.push(line.to_string());
            continue;
        }
        if in_section {
            if let Some((line_key, _)) = active_assignment(trimmed) {
                if line_key == key {
                    out.push(format!("{}={}", key, value));
                    replaced = true;
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }

    if !found_section {
        out.push(format!("[{}]", section));
        out.push(format!("{}={}", key, value));
    } else if in_section && !replaced {
        out.push(format!("{}={}", key, value));
    }

    Some(format!("{}\n", out.join("\n")))
}

fn section_header(line: &str) -> Option<&str> {
    line.strip_prefix('[')?.strip_suffix(']')
}

fn active_assignment(line: &str) -> Option<(&str, &str)> {
    if line.starts_with(';') || line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

async fn filebrowser_pod(cfg: &Config) -> Result<String> {
    let pods = kubectl::get_json(&[
        "get",
        "pods",
        "-n",
        &cfg.namespace,
        "-l",
        "role=igw-filebrowser",
    ])
    .await?;
    pods.pointer("/items/0/metadata/name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("filebrowser pod not found in {}", cfg.namespace))
}
