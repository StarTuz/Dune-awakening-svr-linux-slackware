# dune-ctl Sietch Management — Design & Implementation Plan

Status: **Phase 0 ✅ + Phase 1 ✅ done; Phase 2 next** (2026-06-02). Motivating
case: `../PLANETOLOGIST-TRAINER-BUG.md` (a single-Sietch world blocks a quest
whose recovery requires switching Sietches).

- Phase 0: `sietches edit [--advanced]` wraps `bg-util` (`core/src/sietches.rs::edit`).
- Phase 1: `sietches list` shows capacity (`active`/`max` = enabled
  `worldPartitions`) with a single-Sietch hint (`core/src/sietches.rs::capacity`,
  unit-tested).

## Goal

Make `dune-ctl` a full, safe replacement for Funcom's **Battlegroup Editor**
(`bg-util`) for **Sietch (instance / "dimension") management**, matching its model
exactly and adding dune-ctl's conveniences (preflight, capsule mirroring,
dry-run, TUI). Today `sietches` only proxies whole-BattleGroup start/stop
(`core/src/sietches.rs`); this elevates it to real per-Sietch lifecycle.

## What a "Sietch" is

- **Battlegroup = World; Sietch = a world instance/shard (~60-player cap).**
- Official worlds run many Sietches (e.g. `Acheron` = 25). Self-hosted worlds can
  too — the limit is **hardware, not an FLS entitlement** (confirmed via Funcom's
  self-hosted docs). Our `Ixware` runs a single Sietch (`Silakwir`).
- The in-game client lets a player switch Sietches under a world.

## The Battlegroup Editor IS `bg-util`

Not a separate GUI. `bg-util` is a Funcom Go TUI (`github.com/funcom/bg-util`
v1.0.16) shipped at `server/scripts/bg-util` (symlink `~/.dune/bin/bg-util`),
launched as `KUBE_EDITOR` for `kubectl edit battlegroup` (see
`server/scripts/battlegroup.sh::edit_battlegroup`). `--help`: *"Edit dimensions
and memory limits in a BattleGroup world template."*

### Invariants decoded from bg-util (non-negotiable correctness rules)

- **Max Sietches for a map = its `worldPartitions` ("dimensions") count.**
- **Active servers (`sets[i].replicas`) must be ≤ the partition count.** Violating
  this is what crash-looped our manual experiment: `replicas=2` with one partition
  → second instance used a nonexistent partition id (`load_world_partition … got 0
  rows, expected exactly 1`).
- **Each partition/Sietch has its own `Bgd.ServerDisplayName` + `Bgd.ServerLoginPassword`**
  (per-partition or shared). Names must be unique.
- Also editable by bg-util: per-map memory limits, per-partition arguments
  (kept out of initial native scope; covered by the `edit` passthrough).
- `Overmap can only be max 1`; some maps use `dedicatedScaling` (runtime-managed) —
  exclude from manual scaling.

## Existing dune-ctl scaffolding to reuse

- `core/src/maps.rs` — template for everything: kubectl JSON-patch, `world_partitions()`
  reader, INI editing + `replace_yaml_block` capsule mirroring, the ServerSetScale
  chain, guards, and unit tests.
- `core/src/battlegroup.rs` — `SietchEntry`, `derive_sietches()`,
  `parse_director_min_servers()`, status phases.
- `core/src/settings.rs` — already maps `sietch_name`→`Bgd.ServerDisplayName`,
  `sietch_password`→`Bgd.ServerLoginPassword`.
- `ctl/src/cli/mod.rs` — `Command::Sietches { action: SietchesCommand }`
  (List/Start/Stop/Restart today), dispatched by `cmd_sietches`.

## Phased approach (de-risked)

### Phase 0 — `sietches edit` (wrap bg-util). **Ship first.**
Shell out to the official editor; zero risk of getting partition math wrong.
```
dune-ctl --world <w> sietches edit            # KUBE_EDITOR=<bg-util> kubectl edit battlegroup …
dune-ctl --world <w> sietches edit --advanced # raw kubectl edit (vi/nano)
```
Requires interactive (inherited) stdio + setting `KUBE_EDITOR`; locate `bg-util`
at `~/.dune/bin/bg-util`, falling back to `<download>/scripts/bg-util`.

### Phase 1 — read-only parity (`list`/`status`).
Enhance `sietches list` to show, per Sietch-hosting map (primarily `Survival_1`):
partition count (max), active replicas, per-partition `ServerDisplayName`, live
player count + phase (join `derive_sietches()` + `serverstats`). No mutation.

### Phase 2 — native mutations, validated against bg-util.
`add`/`remove`/`scale`/`rename`/`set-password`.
**Gate:** diff the CR `bg-util` produces for "add a Sietch" vs. our output on a
scratch/PTC capsule; they must match (esp. the partition-id ↔ replica-index
scheme — see Risks). Until clean, `add`/`remove` delegate to a scripted bg-util
invocation.

## Core API — expand `core/src/sietches.rs`

```rust
pub struct SietchInfo {            // list/status
    pub partition_id: u32,
    pub display_name: Option<String>,
    pub active: bool,              // within replicas
    pub phase: ServerPhase,        // reuse battlegroup.rs
    pub players: u32,
}
pub async fn list(cfg) -> Result<Vec<SietchInfo>>;

pub async fn add(cfg, name, password: Option<&str>, also_capsule, dry_run) -> Result<SietchOutcome>;
pub async fn remove(cfg, id_or_name, also_capsule, dry_run) -> Result<SietchOutcome>;
pub async fn scale(cfg, active: u32, dry_run) -> Result<SietchOutcome>;   // enforce active ≤ partitions
pub async fn rename(cfg, id_or_name, new_name, also_capsule) -> Result<SietchOutcome>;
pub async fn edit(cfg, advanced: bool) -> Result<()>;                     // Phase 0
```
`add` = append a `worldPartitions` entry (`{dimension:0,disable:false,id:<next>,
minX:0,maxX:1,minY:0,maxY:1}`) **and** raise active replicas **and** write the
per-partition `Bgd.ServerDisplayName`/password — one CR patch, mirrored to the
capsule.

## Safety rails (value-add over bare bg-util)

- Refuse `active > partition_count`; refuse duplicate `ServerDisplayName`.
- RAM preflight (~5 Gi/Sietch) via `health.rs`/`diagnostics.rs`; `--force` to override.
- Auto-backup (`backup::run`) before any mutation.
- `--dry-run` prints CR patch + capsule diff without applying.
- Capsule mirroring (`replace_yaml_block`) so cold-swap keeps Sietch topology.
- `--yes` gating for mutations.

## CLI surface — extend `SietchesCommand`

```
sietches list
sietches add <name> [--password <p>] [--yes] [--dry-run]
sietches remove <name|id> [--yes] [--dry-run]
sietches scale <N> [--yes]
sietches rename <name|id> <new-name>
sietches edit [--advanced]
sietches start|stop|restart            # existing whole-BG lifecycle
```

## TUI

Extend the Sietches view (`tui/ui.rs`,`tui/app.rs`) to list Sietches
(name/phase/players) with key-bound add / rename / scale / edit-in-bg-util,
mirroring the Maps tab.

## Tests

Unit-test pure logic à la `maps.rs`: `worldPartitions` add/remove, the
`active ≤ count` invariant, per-partition `ServerDisplayName` insertion +
uniqueness, capsule `replace_yaml_block` round-trips, and the Phase-2 bg-util
CR-diff fixture.

## Docs

- `dune-ctl/OPERATIONS.md`: Sietches section (commands + worldPartitions=max /
  replicas≤count / per-partition-name model).
- `CLAUDE.md`: correct the Sietches line (currently "maps to BattleGroup spec.stop").
- Cross-link from `PLANETOLOGIST-TRAINER-BUG.md`.

## Risks / open questions

- **Partition-id ↔ replica-index scheme (biggest).** The experiment showed the
  operator used replica index `1000000` as a partition id, so the mapping is not
  naive `1,2,3`. Decode `bg-util`'s `internal/partition/partition.go`
  (`SetMaxDimensions`/`SetActiveDimensions`) or empirically match its CR output
  before trusting native writes — hence the Phase-2 diff gate. Phase 0 (`edit`) and
  delegation cover us until then.
- Per-map max constraints (`Overmap` max 1); `dedicatedScaling` maps excluded.
