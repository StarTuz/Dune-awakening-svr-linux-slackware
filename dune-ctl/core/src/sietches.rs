use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use crate::{battlegroup, config::Config, kubectl};

pub const PRIMARY_SIETCH_MAP: &str = "Survival_1";

const WORLD_PARTITIONS_PTR: &str = "/spec/database/template/spec/deployment/spec/worldPartitions";
const SETS_PTR: &str = "/spec/serverGroup/template/spec/sets";

/// A computed plan to add one Sietch to a map, derived purely from the live
/// BattleGroup JSON. Mirrors exactly what `bg-util` writes (verified against a
/// captured `bg-util` diff): a new `worldPartitions` partition (next dimension,
/// next global id, same grid), the id appended to the set's `partitions`, and
/// `replicas` raised by one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddSietchPlan {
    pub map: String,
    pub set_index: usize,
    pub worldpartitions_index: usize,
    pub new_partition_id: u32,
    pub new_dimension: u32,
    pub new_replicas: u32,
    pub min_x: i64,
    pub max_x: i64,
    pub min_y: i64,
    pub max_y: i64,
    /// Whether the set already has a `podSpecs` array (decides the JSON-patch op
    /// used when attaching a per-Sietch name).
    pub set_has_podspecs: bool,
}

/// The `-execcmds=...` argument bg-util writes to give a Sietch a unique display
/// name (verified against a captured diff). Names may not contain `'` or `"`.
fn display_name_arg(name: &str) -> Result<String> {
    Ok(format!(
        "-execcmds=\"Bgd.ServerDisplayName '{}'\"",
        validate_cvar_value(name, "name")?
    ))
}

/// The `-execcmds=...` argument that sets a Sietch's join password. Same shape as
/// the display-name arg (the only captured per-Sietch form), with the
/// `Bgd.ServerLoginPassword` cvar.
fn login_password_arg(password: &str) -> Result<String> {
    Ok(format!(
        "-execcmds=\"Bgd.ServerLoginPassword '{}'\"",
        validate_cvar_value(password, "password")?
    ))
}

/// Per-Sietch cvar values are single-quoted inside a double-quoted `-execcmds`,
/// so they cannot contain `'` or `"` (matches bg-util's own restriction).
fn validate_cvar_value<'a>(value: &'a str, kind: &str) -> Result<&'a str> {
    if value.is_empty() {
        anyhow::bail!("Sietch {kind} must not be empty");
    }
    if value.contains('\'') || value.contains('"') {
        anyhow::bail!("Sietch {kind} must not contain ' or \" characters");
    }
    Ok(value)
}

/// True if an argument string sets `Bgd.ServerDisplayName` via `-execcmds`.
fn is_display_name_arg(arg: &str) -> bool {
    arg.starts_with("-execcmds=") && arg.contains("Bgd.ServerDisplayName")
}

/// True if an argument string sets `Bgd.ServerLoginPassword` via `-execcmds`.
fn is_password_arg(arg: &str) -> bool {
    arg.starts_with("-execcmds=") && arg.contains("Bgd.ServerLoginPassword")
}

/// Compute the add-Sietch plan for `map` from the BattleGroup JSON.
fn plan_add_sietch(bg: &Value, map: &str) -> Result<AddSietchPlan> {
    let sets = bg
        .pointer(SETS_PTR)
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("serverGroup sets not found in BattleGroup CR"))?;
    let set_index = sets
        .iter()
        .position(|s| s.get("map").and_then(|m| m.as_str()) == Some(map))
        .ok_or_else(|| anyhow::anyhow!("map '{}' not found in serverGroup sets", map))?;
    let current_replicas = sets[set_index]
        .get("replicas")
        .and_then(|r| r.as_u64())
        .unwrap_or(0) as u32;

    let wps = bg
        .pointer(WORLD_PARTITIONS_PTR)
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("worldPartitions not found in BattleGroup CR"))?;
    let worldpartitions_index = wps
        .iter()
        .position(|e| e.get("map").and_then(|m| m.as_str()) == Some(map))
        .ok_or_else(|| anyhow::anyhow!("worldPartitions entry for '{}' not found", map))?;
    let map_parts = wps[worldpartitions_index]
        .get("partitions")
        .and_then(|p| p.as_array())
        .ok_or_else(|| anyhow::anyhow!("partitions missing for map '{}'", map))?;

    // id = (global max id across ALL maps) + 1  (bg-util assigns globally-unique ids)
    let global_max_id = wps
        .iter()
        .filter_map(|e| e.get("partitions").and_then(|p| p.as_array()))
        .flatten()
        .filter_map(|p| p.get("id").and_then(|i| i.as_u64()))
        .max()
        .unwrap_or(0);
    let new_partition_id = (global_max_id + 1) as u32;

    // dimension = (max existing dimension for this map) + 1
    let new_dimension = map_parts
        .iter()
        .filter_map(|p| p.get("dimension").and_then(|d| d.as_u64()))
        .max()
        .map(|m| m + 1)
        .unwrap_or(0) as u32;

    // Grid copied from the map's existing partition (default to 1x1).
    let grid = |key: &str, default: i64| -> i64 {
        map_parts
            .first()
            .and_then(|p| p.get(key))
            .and_then(|v| v.as_i64())
            .unwrap_or(default)
    };

    Ok(AddSietchPlan {
        map: map.to_string(),
        set_index,
        worldpartitions_index,
        new_partition_id,
        new_dimension,
        new_replicas: current_replicas + 1,
        min_x: grid("minX", 0),
        max_x: grid("maxX", 1),
        min_y: grid("minY", 0),
        max_y: grid("maxY", 1),
        set_has_podspecs: sets[set_index]
            .get("podSpecs")
            .and_then(|v| v.as_array())
            .is_some(),
    })
}

/// Build the RFC-6902 JSON patch that realises an [`AddSietchPlan`]. When `name`
/// and/or `password` are given, also attach a `podSpecs` entry binding them to
/// the new Sietch's partition id (mirrors bg-util).
fn build_add_patch(
    plan: &AddSietchPlan,
    name: Option<&str>,
    password: Option<&str>,
) -> Result<Vec<Value>> {
    let new_partition = json!({
        "dimension": plan.new_dimension,
        "disable": false,
        "id": plan.new_partition_id,
        "maxX": plan.max_x,
        "maxY": plan.max_y,
        "minX": plan.min_x,
        "minY": plan.min_y,
    });
    let mut ops = vec![
        json!({
            "op": "add",
            "path": format!("{}/{}/partitions/-", WORLD_PARTITIONS_PTR, plan.worldpartitions_index),
            "value": new_partition,
        }),
        json!({
            "op": "add",
            "path": format!("{}/{}/partitions/-", SETS_PTR, plan.set_index),
            "value": plan.new_partition_id,
        }),
        json!({
            "op": "replace",
            "path": format!("{}/{}/replicas", SETS_PTR, plan.set_index),
            "value": plan.new_replicas,
        }),
    ];
    let mut args: Vec<Value> = Vec::new();
    if let Some(name) = name {
        args.push(Value::String(display_name_arg(name)?));
    }
    if let Some(password) = password {
        args.push(Value::String(login_password_arg(password)?));
    }
    if !args.is_empty() {
        let entry = json!({ "index": plan.new_partition_id, "arguments": args });
        if plan.set_has_podspecs {
            ops.push(json!({
                "op": "add",
                "path": format!("{}/{}/podSpecs/-", SETS_PTR, plan.set_index),
                "value": entry,
            }));
        } else {
            ops.push(json!({
                "op": "add",
                "path": format!("{}/{}/podSpecs", SETS_PTR, plan.set_index),
                "value": [entry],
            }));
        }
    }
    Ok(ops)
}

/// Add a Sietch to the primary Sietch map. Returns the plan (also usable for a
/// dry-run preview). Does NOT take a backup itself — callers should back up first
/// (the CLI does). When `name` is given, the new Sietch gets a unique display
/// name (`podSpecs`); otherwise it inherits the world's shared name.
pub async fn add(
    cfg: &Config,
    name: Option<&str>,
    password: Option<&str>,
    dry_run: bool,
) -> Result<(AddSietchPlan, String)> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;
    let plan = plan_add_sietch(&bg, PRIMARY_SIETCH_MAP)?;
    let patch = build_add_patch(&plan, name, password)?;
    let patch_json = serde_json::to_string_pretty(&patch)?;
    if dry_run {
        return Ok((plan, patch_json));
    }
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
    Ok((plan, patch_json))
}

/// Set (or change) the display name of an existing Sietch (by world-partition id).
pub async fn rename(cfg: &Config, partition_id: u32, new_name: &str) -> Result<()> {
    upsert_podspec_arg(cfg, partition_id, display_name_arg(new_name)?, is_display_name_arg).await
}

/// Set (or change) the join password of an existing Sietch (by world-partition id).
pub async fn set_password(cfg: &Config, partition_id: u32, password: &str) -> Result<()> {
    upsert_podspec_arg(cfg, partition_id, login_password_arg(password)?, is_password_arg).await
}

/// Add or update one `-execcmds` per-Sietch argument (display name or password)
/// on the `podSpecs` entry for `partition_id`, preserving the entry's other
/// arguments. `matches` identifies the argument flavour to replace.
async fn upsert_podspec_arg(
    cfg: &Config,
    partition_id: u32,
    new_arg: String,
    matches: fn(&str) -> bool,
) -> Result<()> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;

    let sets = bg
        .pointer(SETS_PTR)
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("serverGroup sets not found"))?;
    let set_index = sets
        .iter()
        .position(|s| s.get("map").and_then(|m| m.as_str()) == Some(PRIMARY_SIETCH_MAP))
        .ok_or_else(|| anyhow::anyhow!("{} not found in sets", PRIMARY_SIETCH_MAP))?;
    let set = &sets[set_index];

    let in_set = set
        .get("partitions")
        .and_then(|p| p.as_array())
        .map(|ps| {
            ps.iter()
                .filter_map(|v| v.as_u64())
                .any(|v| v as u32 == partition_id)
        })
        .unwrap_or(false);
    if !in_set {
        anyhow::bail!(
            "partition id {} is not a Sietch of {} (see `sietches list`)",
            partition_id,
            PRIMARY_SIETCH_MAP
        );
    }

    let podspecs = set.get("podSpecs").and_then(|v| v.as_array());
    let existing_idx = podspecs.and_then(|ps| {
        ps.iter().position(|e| {
            e.get("index").and_then(|i| i.as_u64()).map(|i| i as u32) == Some(partition_id)
        })
    });

    let patch = match (podspecs, existing_idx) {
        (Some(ps), Some(i)) => {
            let mut args: Vec<Value> = ps[i]
                .get("arguments")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter(|v| !v.as_str().map(matches).unwrap_or(false))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            args.push(Value::String(new_arg));
            json!([{
                "op": "replace",
                "path": format!("{}/{}/podSpecs/{}/arguments", SETS_PTR, set_index, i),
                "value": args,
            }])
        }
        (Some(_), None) => json!([{
            "op": "add",
            "path": format!("{}/{}/podSpecs/-", SETS_PTR, set_index),
            "value": { "index": partition_id, "arguments": [new_arg] },
        }]),
        (None, _) => json!([{
            "op": "add",
            "path": format!("{}/{}/podSpecs", SETS_PTR, set_index),
            "value": [{ "index": partition_id, "arguments": [new_arg] }],
        }]),
    };

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
    Ok(())
}

/// Set the number of active Sietches (`sets[i].replicas`) for the primary map.
/// Enforces the bg-util invariant `active <= max` (enabled `worldPartitions`
/// count); raising beyond the current max requires `add` first.
pub async fn scale(cfg: &Config, active: u32, dry_run: bool) -> Result<SietchCapacity> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;
    let max = enabled_partition_count(&bg, PRIMARY_SIETCH_MAP);
    if active as usize > max {
        anyhow::bail!(
            "cannot set active Sietches to {} — only {} world partition(s) exist for {}.\n\
             Add a Sietch first (`dune-ctl sietches add`), which provisions the partition a \
             new instance needs; a bare replicas bump beyond the partition count crash-loops.",
            active,
            max,
            PRIMARY_SIETCH_MAP
        );
    }
    let set_index = bg
        .pointer(SETS_PTR)
        .and_then(|v| v.as_array())
        .and_then(|sets| {
            sets.iter()
                .position(|s| s.get("map").and_then(|m| m.as_str()) == Some(PRIMARY_SIETCH_MAP))
        })
        .ok_or_else(|| anyhow::anyhow!("{} not found in serverGroup sets", PRIMARY_SIETCH_MAP))?;

    if !dry_run {
        let patch = json!([{
            "op": "replace",
            "path": format!("{}/{}/replicas", SETS_PTR, set_index),
            "value": active,
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
    Ok(SietchCapacity {
        map: PRIMARY_SIETCH_MAP.to_string(),
        max,
        active,
    })
}

/// Plan to remove one Sietch (by world-partition id) from a map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveSietchPlan {
    pub map: String,
    pub partition_id: u32,
    pub dimension: u32,
    set_index: usize,
    worldpartitions_index: usize,
    partition_arr_index: usize,
    set_partition_index: usize,
    podspec_index: Option<usize>,
    pub remaining_replicas: u32,
}

/// Compute (and safety-check) the plan to remove `partition_id` from `map`.
/// Refuses to remove the primary partition (dimension 0) or the last one.
fn plan_remove_sietch(bg: &Value, map: &str, partition_id: u32) -> Result<RemoveSietchPlan> {
    let sets = bg
        .pointer(SETS_PTR)
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("serverGroup sets not found"))?;
    let set_index = sets
        .iter()
        .position(|s| s.get("map").and_then(|m| m.as_str()) == Some(map))
        .ok_or_else(|| anyhow::anyhow!("map '{}' not found in sets", map))?;
    let set_partitions = sets[set_index]
        .get("partitions")
        .and_then(|p| p.as_array())
        .ok_or_else(|| anyhow::anyhow!("set partitions missing for '{}'", map))?;
    let set_partition_index = set_partitions
        .iter()
        .position(|v| v.as_u64().map(|n| n as u32) == Some(partition_id))
        .ok_or_else(|| {
            anyhow::anyhow!("partition id {} is not a Sietch of '{}'", partition_id, map)
        })?;
    if set_partitions.len() <= 1 {
        anyhow::bail!("refusing to remove the only Sietch of '{}'", map);
    }

    let wps = bg
        .pointer(WORLD_PARTITIONS_PTR)
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("worldPartitions not found"))?;
    let worldpartitions_index = wps
        .iter()
        .position(|e| e.get("map").and_then(|m| m.as_str()) == Some(map))
        .ok_or_else(|| anyhow::anyhow!("worldPartitions entry for '{}' not found", map))?;
    let map_parts = wps[worldpartitions_index]
        .get("partitions")
        .and_then(|p| p.as_array())
        .ok_or_else(|| anyhow::anyhow!("partitions missing for '{}'", map))?;
    let partition_arr_index = map_parts
        .iter()
        .position(|p| p.get("id").and_then(|i| i.as_u64()).map(|n| n as u32) == Some(partition_id))
        .ok_or_else(|| anyhow::anyhow!("worldPartitions has no partition id {}", partition_id))?;
    let dimension = map_parts[partition_arr_index]
        .get("dimension")
        .and_then(|d| d.as_u64())
        .unwrap_or(0) as u32;
    if dimension == 0 {
        anyhow::bail!(
            "refusing to remove the primary Sietch (dimension 0, partition id {}) of '{}'",
            partition_id,
            map
        );
    }

    let podspec_index = sets[set_index]
        .get("podSpecs")
        .and_then(|v| v.as_array())
        .and_then(|ps| {
            ps.iter().position(|e| {
                e.get("index").and_then(|i| i.as_u64()).map(|n| n as u32) == Some(partition_id)
            })
        });

    Ok(RemoveSietchPlan {
        map: map.to_string(),
        partition_id,
        dimension,
        set_index,
        worldpartitions_index,
        partition_arr_index,
        set_partition_index,
        podspec_index,
        remaining_replicas: (set_partitions.len() - 1) as u32,
    })
}

/// Build the JSON patch for a [`RemoveSietchPlan`]. Each `remove` targets a
/// distinct array (one element each), so the precomputed indices stay valid.
fn build_remove_patch(plan: &RemoveSietchPlan) -> Vec<Value> {
    let mut ops = vec![
        json!({
            "op": "remove",
            "path": format!("{}/{}/partitions/{}", WORLD_PARTITIONS_PTR, plan.worldpartitions_index, plan.partition_arr_index),
        }),
        json!({
            "op": "remove",
            "path": format!("{}/{}/partitions/{}", SETS_PTR, plan.set_index, plan.set_partition_index),
        }),
        json!({
            "op": "replace",
            "path": format!("{}/{}/replicas", SETS_PTR, plan.set_index),
            "value": plan.remaining_replicas,
        }),
    ];
    if let Some(ps_idx) = plan.podspec_index {
        ops.push(json!({
            "op": "remove",
            "path": format!("{}/{}/podSpecs/{}", SETS_PTR, plan.set_index, ps_idx),
        }));
    }
    ops
}

/// Remove a Sietch (by world-partition id) from the primary Sietch map: drops the
/// `worldPartitions` entry, the id from the set's `partitions`, any matching
/// `podSpecs` entry, and lowers `replicas`. Refuses the primary/last Sietch. Does
/// NOT back up itself (the CLI does).
pub async fn remove(
    cfg: &Config,
    partition_id: u32,
    dry_run: bool,
) -> Result<(RemoveSietchPlan, String)> {
    let bg =
        kubectl::get_json(&["get", "battlegroup", &cfg.battlegroup, "-n", &cfg.namespace]).await?;
    let plan = plan_remove_sietch(&bg, PRIMARY_SIETCH_MAP, partition_id)?;
    let patch = build_remove_patch(&plan);
    let patch_json = serde_json::to_string_pretty(&patch)?;
    if dry_run {
        return Ok((plan, patch_json));
    }
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
    Ok((plan, patch_json))
}

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

    /// Fixture mirroring the live CR: Survival_1 has one partition (id 1, dim 0),
    /// and other maps fill ids 2..=max_id. Survival_1 set has `replicas`.
    fn bg_full(max_id: u64, replicas: u64) -> Value {
        let mut world_partitions = vec![json!({
            "map": "Survival_1",
            "partitions": [{ "dimension": 0, "disable": false, "id": 1,
                             "maxX": 1, "maxY": 1, "minX": 0, "minY": 0 }]
        })];
        for id in 2..=max_id {
            world_partitions.push(json!({
                "map": format!("Map{id}"),
                "partitions": [{ "dimension": 0, "disable": false, "id": id,
                                 "maxX": 1, "maxY": 1, "minX": 0, "minY": 0 }]
            }));
        }
        json!({
            "spec": {
                "database": { "template": { "spec": { "deployment": { "spec": {
                    "worldPartitions": world_partitions
                }}}}},
                "serverGroup": { "template": { "spec": { "sets": [
                    { "map": "Survival_1", "replicas": replicas, "partitions": [1] }
                ]}}}
            }
        })
    }

    #[test]
    fn add_plan_matches_bg_util_output() {
        // ids 1..=30 exist, Survival_1 dim 0; adding a Sietch must mirror bg-util:
        // new id 31, dimension 1, replicas 2, 1x1 grid (captured from a real diff).
        let v = bg_full(30, 1);
        let plan = plan_add_sietch(&v, "Survival_1").unwrap();
        assert_eq!(plan.new_partition_id, 31);
        assert_eq!(plan.new_dimension, 1);
        assert_eq!(plan.new_replicas, 2);
        assert_eq!(
            (plan.min_x, plan.max_x, plan.min_y, plan.max_y),
            (0, 1, 0, 1)
        );
        assert_eq!(plan.set_index, 0);
        assert_eq!(plan.worldpartitions_index, 0);
    }

    #[test]
    fn add_patch_matches_bg_util_diff() {
        let v = bg_full(30, 1);
        let plan = plan_add_sietch(&v, "Survival_1").unwrap();
        assert!(!plan.set_has_podspecs);
        let patch = build_add_patch(&plan, None, None).unwrap();
        assert_eq!(patch.len(), 3);
        // 1) append the new partition object to worldPartitions[0].partitions
        assert_eq!(patch[0]["op"], "add");
        assert_eq!(
            patch[0]["path"],
            "/spec/database/template/spec/deployment/spec/worldPartitions/0/partitions/-"
        );
        assert_eq!(patch[0]["value"]["id"], 31);
        assert_eq!(patch[0]["value"]["dimension"], 1);
        assert_eq!(patch[0]["value"]["disable"], false);
        // 2) append id 31 to the set's partitions
        assert_eq!(patch[1]["op"], "add");
        assert_eq!(
            patch[1]["path"],
            "/spec/serverGroup/template/spec/sets/0/partitions/-"
        );
        assert_eq!(patch[1]["value"], 31);
        // 3) replicas -> 2
        assert_eq!(patch[2]["op"], "replace");
        assert_eq!(patch[2]["path"], "/spec/serverGroup/template/spec/sets/0/replicas");
        assert_eq!(patch[2]["value"], 2);
    }

    #[test]
    fn add_patch_with_name_appends_podspecs() {
        let v = bg_full(30, 1);
        let plan = plan_add_sietch(&v, "Survival_1").unwrap();
        let patch = build_add_patch(&plan, Some("Sietch Testbed"), None).unwrap();
        assert_eq!(patch.len(), 4);
        // set has no podSpecs yet → create the array at /podSpecs
        assert_eq!(patch[3]["op"], "add");
        assert_eq!(patch[3]["path"], "/spec/serverGroup/template/spec/sets/0/podSpecs");
        let entry = &patch[3]["value"][0];
        assert_eq!(entry["index"], 31); // index = new partition id
        assert_eq!(
            entry["arguments"][0],
            "-execcmds=\"Bgd.ServerDisplayName 'Sietch Testbed'\""
        );
    }

    #[test]
    fn add_patch_with_name_and_password_has_both_args() {
        let v = bg_full(30, 1);
        let plan = plan_add_sietch(&v, "Survival_1").unwrap();
        let patch = build_add_patch(&plan, Some("Sietch Testbed"), Some("hunter2")).unwrap();
        let args = &patch[3]["value"][0]["arguments"];
        assert_eq!(args[0], "-execcmds=\"Bgd.ServerDisplayName 'Sietch Testbed'\"");
        assert_eq!(args[1], "-execcmds=\"Bgd.ServerLoginPassword 'hunter2'\"");
    }

    #[test]
    fn name_and_password_with_quote_are_rejected() {
        assert!(display_name_arg("Bad'Name").is_err());
        assert!(display_name_arg("Bad\"Name").is_err());
        assert!(display_name_arg("").is_err());
        assert!(display_name_arg("Sietch Tarball").is_ok());
        assert!(login_password_arg("pw'x").is_err());
        assert!(login_password_arg("good-pw").is_ok());
    }

    /// Fixture with two Survival_1 Sietches (dim 0 id 1, dim 1 id 31), set
    /// partitions [1,31], replicas 2, and a podSpecs entry for id 31.
    fn bg_two_survival() -> Value {
        json!({
            "spec": {
                "database": { "template": { "spec": { "deployment": { "spec": {
                    "worldPartitions": [{
                        "map": "Survival_1",
                        "partitions": [
                            { "dimension": 0, "disable": false, "id": 1, "maxX":1,"maxY":1,"minX":0,"minY":0 },
                            { "dimension": 1, "disable": false, "id": 31, "maxX":1,"maxY":1,"minX":0,"minY":0 }
                        ]
                    }]
                }}}}},
                "serverGroup": { "template": { "spec": { "sets": [{
                    "map": "Survival_1",
                    "replicas": 2,
                    "partitions": [1, 31],
                    "podSpecs": [{ "index": 31, "arguments": ["-execcmds=\"Bgd.ServerDisplayName 'X'\""] }]
                }]}}}
            }
        })
    }

    #[test]
    fn remove_plan_rejects_primary_and_last() {
        let v = bg_two_survival();
        // dimension-0 / primary partition id 1 is refused
        assert!(plan_remove_sietch(&v, "Survival_1", 1).is_err());
        // unknown id refused
        assert!(plan_remove_sietch(&v, "Survival_1", 999).is_err());
        // removing the only partition is refused
        let single = bg_full(30, 1);
        assert!(plan_remove_sietch(&single, "Survival_1", 1).is_err());
    }

    #[test]
    fn remove_patch_drops_partition_set_id_podspec_and_lowers_replicas() {
        let v = bg_two_survival();
        let plan = plan_remove_sietch(&v, "Survival_1", 31).unwrap();
        assert_eq!(plan.dimension, 1);
        assert_eq!(plan.remaining_replicas, 1);
        assert_eq!(plan.podspec_index, Some(0));
        let patch = build_remove_patch(&plan);
        assert_eq!(patch.len(), 4);
        // worldPartitions partition removed (id 31 is at array index 1)
        assert_eq!(patch[0]["op"], "remove");
        assert_eq!(
            patch[0]["path"],
            "/spec/database/template/spec/deployment/spec/worldPartitions/0/partitions/1"
        );
        // set partitions: id 31 at index 1 removed
        assert_eq!(patch[1]["op"], "remove");
        assert_eq!(patch[1]["path"], "/spec/serverGroup/template/spec/sets/0/partitions/1");
        // replicas -> 1
        assert_eq!(patch[2]["op"], "replace");
        assert_eq!(patch[2]["value"], 1);
        // podSpecs entry removed
        assert_eq!(patch[3]["op"], "remove");
        assert_eq!(patch[3]["path"], "/spec/serverGroup/template/spec/sets/0/podSpecs/0");
    }

    #[test]
    fn add_plan_increments_global_id_and_dimension_on_second_add() {
        // After one add (ids up to 31, Survival_1 has dims 0 and 1), a further add
        // must go to id 32 / dimension 2.
        let mut v = bg_full(30, 1);
        // simulate the first add already applied
        let parts = v
            .pointer_mut("/spec/database/template/spec/deployment/spec/worldPartitions/0/partitions")
            .unwrap()
            .as_array_mut()
            .unwrap();
        parts.push(json!({ "dimension": 1, "disable": false, "id": 31,
                           "maxX": 1, "maxY": 1, "minX": 0, "minY": 0 }));
        let plan = plan_add_sietch(&v, "Survival_1").unwrap();
        assert_eq!(plan.new_partition_id, 32);
        assert_eq!(plan.new_dimension, 2);
    }
}
