use anyhow::{Context, Result};
use serde_json::json;

use crate::{config::Config, kubectl};

const DIRECTOR_INI_POINTER: &str = "/spec/utilities/director/spec/configFiles/files/director.ini";
const PERSIST_KEY: &str = "MinServers";

pub fn is_director_managed_social_hub(map_name: &str) -> bool {
    map_name.starts_with("SH_")
}

pub async fn start(cfg: &Config, map_name: &str, force: bool) -> Result<()> {
    if is_director_managed_social_hub(map_name) && !force {
        anyhow::bail!(
            "'{}' is a director-managed social hub.\n\
             Use 'maps prewarm {} --yes' to make it available through the director. \
             Use --force only for low-level recovery/debugging.",
            map_name,
            map_name
        );
    }
    toggle(cfg, map_name, 1).await
}

pub async fn stop(cfg: &Config, map_name: &str) -> Result<()> {
    toggle(cfg, map_name, 0).await
}

pub async fn prewarm(cfg: &Config, map_name: &str) -> Result<PersistOutcome> {
    let outcome = set_persistence(cfg, map_name, true, false).await?;
    request_scale(cfg, map_name, 1).await?;
    Ok(outcome)
}

pub async fn request_scale(cfg: &Config, map_name: &str, replicas: u32) -> Result<()> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;

    let scale_name = format!("{}-{}", cfg.battlegroup, map_slug(map_name));
    kubectl::run(&["get", "serversetscale", &scale_name, "-n", &cfg.namespace])
        .await
        .with_context(|| format!("ServerSetScale '{}' not found", scale_name))?;

    let mut scale_patch = Vec::new();
    if replicas > 0 {
        let map_partitions = world_partitions(&bg, map_name).ok_or_else(|| {
            anyhow::anyhow!("no enabled world partition IDs found for '{}'", map_name)
        })?;
        scale_patch.push(json!({
            "op": "add",
            "path": "/spec/partitions",
            "value": map_partitions,
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
    Ok(())
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

/// Outcome of a persistence toggle, for reporting back to the caller.
pub struct PersistOutcome {
    /// MinServers before the change (None = no entry / not persistent).
    pub previous: Option<u32>,
    /// MinServers written (1 = persistent, 0 = not persistent).
    pub applied: u32,
    /// Whether the CR director.ini actually changed.
    pub cr_changed: bool,
    /// Some(true/false) = capsule mirror attempted and updated/unchanged;
    /// None = not attempted (no capsule).
    pub capsule_updated: Option<bool>,
    /// Human-readable note about the capsule mirror outcome, if any.
    pub capsule_note: Option<String>,
}

/// Current director.ini MinServers for a map (None = no entry / not persistent).
pub async fn min_servers(cfg: &Config, map_name: &str) -> Result<Option<u32>> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;
    Ok(crate::battlegroup::parse_director_min_servers(&bg)
        .get(map_name)
        .copied())
}

/// Toggle director-managed persistence (`MinServers`) for a map.
///
/// Writes the value into the BattleGroup CR's `director.ini` blob (where the
/// director reads it) and, when `also_capsule` and a capsule exists, mirrors it
/// into the capsule source `battlegroup.yaml` so a cold-swap re-activation does
/// not silently revert it. This is orthogonal to `start`/`stop` (replicas):
/// `--on` does not start the map now, and `--off` is required before a `stop`
/// will stick.
pub async fn set_persistence(
    cfg: &Config,
    map_name: &str,
    on: bool,
    also_capsule: bool,
) -> Result<PersistOutcome> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;

    find_map_index(&bg, map_name)
        .ok_or_else(|| anyhow::anyhow!("map '{}' not found in BattleGroup CR", map_name))?;

    let ini = bg
        .pointer(DIRECTOR_INI_POINTER)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("director.ini not found in BattleGroup CR"))?
        .to_string();

    let previous = crate::battlegroup::parse_director_min_servers(&bg)
        .get(map_name)
        .copied();
    let applied: u32 = if on { 1 } else { 0 };
    let updated_ini = set_min_servers(&ini, map_name, on);
    let cr_changed = updated_ini != ini;

    if cr_changed {
        let patch = json!([{
            "op": "replace",
            "path": DIRECTOR_INI_POINTER,
            "value": updated_ini,
        }]);
        kubectl::run(&[
            "patch",
            "battlegroup",
            &cfg.battlegroup,
            "-n",
            &cfg.namespace,
            "--type=json",
            &format!("-p={}", serde_json::to_string(&patch)?),
        ])
        .await?;
    }

    let (capsule_updated, capsule_note) = if also_capsule && cfg.has_capsule() {
        match update_capsule_director_ini(cfg, &updated_ini) {
            Ok(true) => (Some(true), None),
            Ok(false) => (
                Some(false),
                Some("capsule battlegroup.yaml already matched".to_string()),
            ),
            Err(e) => (Some(false), Some(format!("capsule mirror skipped: {}", e))),
        }
    } else if also_capsule {
        (
            None,
            Some("no capsule for this world; live CR only".to_string()),
        )
    } else {
        (None, None)
    };

    Ok(PersistOutcome {
        previous,
        applied,
        cr_changed,
        capsule_updated,
        capsule_note,
    })
}

/// Re-emit the director.ini block in the capsule `battlegroup.yaml` to match
/// `ini`. Returns whether the file changed.
fn update_capsule_director_ini(cfg: &Config, ini: &str) -> Result<bool> {
    let path = cfg.capsule_dir().join("battlegroup.yaml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let new_text = replace_yaml_block(&text, "director.ini", ini)?;
    if new_text == text {
        return Ok(false);
    }
    std::fs::write(&path, new_text).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Set `MinServers` for `map` within an INI-shaped director config string.
///
/// `on` writes `MinServers = 1`; `off` sets an existing entry to `0` and is a
/// no-op when the map has no entry. A missing `[ map ]` section is appended
/// only when turning persistence on. All other content/formatting is preserved.
pub fn set_min_servers(ini: &str, map: &str, on: bool) -> String {
    let target: u32 = if on { 1 } else { 0 };
    let mut lines: Vec<String> = ini.lines().map(str::to_string).collect();

    let section_start = lines
        .iter()
        .position(|l| section_name(l).as_deref() == Some(map));

    match section_start {
        Some(start) => {
            // Greedily scan the section body (blank or more-indented lines).
            let mut greedy_end = start + 1;
            while greedy_end < lines.len() {
                let l = &lines[greedy_end];
                if l.trim().is_empty() || section_name(l).is_none() {
                    greedy_end += 1;
                } else {
                    break;
                }
            }
            let existing = (start + 1..greedy_end).find(|&i| is_key(&lines[i], PERSIST_KEY));
            match (existing, on) {
                (Some(i), _) => {
                    let indent = leading_ws(&lines[i]);
                    lines[i] = format!("{indent}{PERSIST_KEY} = {target}");
                }
                (None, true) => {
                    // Insert after the last non-blank body line, matching its indent.
                    let last_content = (start + 1..greedy_end)
                        .rev()
                        .find(|&i| !lines[i].trim().is_empty());
                    let (insert_at, indent) = match last_content {
                        Some(i) => (i + 1, leading_ws(&lines[i])),
                        None => (start + 1, String::new()),
                    };
                    lines.insert(insert_at, format!("{indent}{PERSIST_KEY} = {target}"));
                }
                (None, false) => {}
            }
        }
        None if on => {
            if lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
            lines.push(format!("[ {map} ]"));
            lines.push(format!("{PERSIST_KEY} = {target}"));
        }
        None => {}
    }

    let mut result = lines.join("\n");
    if ini.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Replace a YAML literal block scalar (`key: |-` / `key: |`) body with
/// `content`, re-indented to the block's existing indentation. Preserves the
/// header line and any trailing separator blank lines.
fn replace_yaml_block(text: &str, key: &str, content: &str) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let header_prefix = format!("{key}:");
    let header_idx = lines
        .iter()
        .position(|l| {
            let t = l.trim_start();
            t.strip_prefix(&header_prefix)
                .map(|after| after.trim_start().starts_with('|'))
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow::anyhow!("'{}:' block scalar not found in capsule YAML", key))?;

    let header_indent = leading_ws(lines[header_idx]).chars().count();

    // Body = subsequent blank or more-indented lines.
    let mut greedy_end = header_idx + 1;
    while greedy_end < lines.len() {
        let l = lines[greedy_end];
        if l.trim().is_empty() || leading_ws(l).chars().count() > header_indent {
            greedy_end += 1;
        } else {
            break;
        }
    }
    // Trailing blank lines belong to the separator, not the block.
    let last_content = (header_idx + 1..greedy_end)
        .rev()
        .find(|&i| !lines[i].trim().is_empty());
    let body_end = last_content.map(|i| i + 1).unwrap_or(header_idx + 1);

    let body_indent = (header_idx + 1..body_end)
        .find(|&i| !lines[i].trim().is_empty())
        .map(|i| leading_ws(lines[i]).chars().count())
        .unwrap_or(header_indent + 2);
    let pad = " ".repeat(body_indent);

    let mut out: Vec<String> = lines[..=header_idx].iter().map(|s| s.to_string()).collect();
    for cl in content.lines() {
        if cl.is_empty() {
            out.push(String::new());
        } else {
            out.push(format!("{pad}{cl}"));
        }
    }
    out.extend(lines[body_end..].iter().map(|s| s.to_string()));

    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

fn section_name(line: &str) -> Option<String> {
    let t = line.trim();
    t.strip_prefix('[')?
        .strip_suffix(']')
        .map(|s| s.trim().to_string())
}

fn is_key(line: &str, key: &str) -> bool {
    line.split_once('=')
        .map(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        .unwrap_or(false)
}

fn leading_ws(line: &str) -> String {
    line.chars().take_while(|c| c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[ Battlegroup ]\nAuthorizationPreset = BattlegroupInternal\n\n[ InstancingModes ]\nDeepDesert_1=ClassicalInstancing\n\n[ DeepDesert_1 ]\nNumExtraServers = 0\nMinServers = 0\n\n[ SH_Arrakeen ]\nNumExtraServers = 0\nMinServers = 0";

    #[test]
    fn turns_existing_entry_on_and_off() {
        let on = set_min_servers(SAMPLE, "DeepDesert_1", true);
        assert!(on.contains("[ DeepDesert_1 ]\nNumExtraServers = 0\nMinServers = 1"));
        // Other sections untouched.
        assert!(on.contains("[ SH_Arrakeen ]\nNumExtraServers = 0\nMinServers = 0"));
        // The InstancingModes key referencing the map is not mistaken for a section.
        assert!(on.contains("DeepDesert_1=ClassicalInstancing"));

        let off = set_min_servers(&on, "DeepDesert_1", false);
        assert_eq!(off, SAMPLE);
    }

    #[test]
    fn inserts_minservers_when_section_has_no_entry() {
        let ini = "[ Story_ArtOfKanly ]\nEnableAutomaticInstanceScaling = true\nNumExtraServers = 0\n\n[ Other ]\nNumExtraServers = 0";
        let on = set_min_servers(ini, "Story_ArtOfKanly", true);
        assert!(on.contains(
            "[ Story_ArtOfKanly ]\nEnableAutomaticInstanceScaling = true\nNumExtraServers = 0\nMinServers = 1"
        ));
        assert!(on.contains("[ Other ]\nNumExtraServers = 0"));
    }

    #[test]
    fn appends_section_when_missing_only_for_on() {
        let ini = "[ Battlegroup ]\nAuthorizationPreset = BattlegroupInternal";
        let on = set_min_servers(ini, "DeepDesert_1", true);
        assert!(on.ends_with("[ DeepDesert_1 ]\nMinServers = 1"));
        // Off on a map with no section is a no-op.
        assert_eq!(set_min_servers(ini, "DeepDesert_1", false), ini);
    }

    #[test]
    fn toggling_one_section_leaves_others_at_their_value() {
        let on = set_min_servers(SAMPLE, "DeepDesert_1", true);
        let both = set_min_servers(&on, "SH_Arrakeen", true);
        assert!(both.contains("[ DeepDesert_1 ]\nNumExtraServers = 0\nMinServers = 1"));
        assert!(both.contains("[ SH_Arrakeen ]\nNumExtraServers = 0\nMinServers = 1"));
    }

    #[test]
    fn replaces_yaml_block_preserving_indent_and_tail() {
        let yaml = "        configFiles:\n          files:\n            director.ini: |-\n              [ Battlegroup ]\n              MinServers = 0\n          other: value\n";
        let new =
            replace_yaml_block(yaml, "director.ini", "[ Battlegroup ]\nMinServers = 1").unwrap();
        assert!(new.contains("            director.ini: |-\n              [ Battlegroup ]\n              MinServers = 1\n          other: value"));
    }
}
