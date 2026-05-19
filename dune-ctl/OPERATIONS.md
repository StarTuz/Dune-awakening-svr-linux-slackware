# dune-ctl Operations Reference

`dune-ctl` is the Rust CLI/TUI for managing the Dune: Awakening self-hosted
battlegroup running on Arrakis (Slackware 15.0, k3s, Funcom operators). It wraps
kubectl, the Funcom scripts, and local config files behind a single binary.

Binary: `~/dune-server/dune-ctl/target/release/dune-ctl`

Build:
```sh
cd ~/dune-server/dune-ctl
cargo build --release -p dune-ctl
```

---

## World targeting

Every command resolves a world (battlegroup) in this order:

1. `--world <id-or-title>` flag
2. `DUNE_CTL_WORLD=<id>` environment variable
3. Auto-selects the only world found in `~/.dune/`

The active world is shown in every command's output footer and in the TUI header.
Use `dune-ctl worlds list` to see all known worlds, including capsule-backed
worlds under `~/.dune/capsules/<env>/<bg>/capsule.env`.

---

## TUI

Launch with no subcommand:

```sh
dune-ctl
```

### Views

| Key | Tab | What it shows |
|-----|-----|---------------|
| `1` | Worlds | Known worlds, settings profile status |
| `2` | Dashboard | Battlegroup phase, FLS token, RAM, map grid, gateway/RMQ status |
| `3` | Maps | All 28 maps with live phase; start/stop from here |
| `4` | Settings | Managed settings with local values; inline set/toggle/apply |
| `5` | Logs | Two-pane: pod selector (left) + last N log lines (right) |
| `6` | Backups | Bundle list with age/size; trigger backup run with streaming output |

`Tab` cycles through all views in order. `q` quits from any view.

### Global keys

| Key | Action |
|-----|--------|
| `Tab` | Next view |
| `1`–`6` | Jump to view |
| `r` | Refresh (re-fetches health snapshot and current view data) |
| `q` | Quit |

### Maps view keys

| Key | Action |
|-----|--------|
| `↑` / `↓` | Select map |
| `Enter` | Start / stop selected map (shows confirmation) |
| `r` | Refresh map list |

### Settings view keys

| Key | Action |
|-----|--------|
| `↑` / `↓` | Select setting |
| `e` | Edit selected setting (opens inline input) |
| `t` | Toggle boolean setting |
| `a` | Apply local settings to deployed UserSettings |
| `A` | Apply + restart primary Sietch |
| `p` | Pull deployed settings to local |
| `Enter` (in input) | Confirm edit |
| `Esc` | Cancel |

### Logs view keys

| Key | Action |
|-----|--------|
| `↑` / `↓` | Select pod target |
| `r` | Force-refresh log lines for selected target |

### Backups view keys

| Key | Action |
|-----|--------|
| `r` | Run full backup (shows confirmation modal) |
| `r` (while running) | Refreshes bundle list only |

---

## CLI Reference

### `dune-ctl status`

Quick snapshot: battlegroup phase, running maps, FLS token expiry, RAM.

```sh
dune-ctl status
```

### `dune-ctl preflight [--strict]`

Go/no-go check before opening the server. Tests firewall backend, gateway patch,
FLS token, primary Sietch health, and RAM. Exits non-zero on failure.
`--strict` also fails on warnings.

```sh
dune-ctl preflight
dune-ctl preflight --strict
```

### `dune-ctl worlds`

```sh
dune-ctl worlds list          # show all ~/.dune world specs
dune-ctl worlds init-settings # create per-world UserSettings profile
```

`init-settings` copies the shared `server/scripts/setup/config/User*.ini`
templates into `~/.dune/worlds/<battlegroup>/UserSettings/`. After this, all
`settings` commands read/write that profile instead of the shared files.

### `dune-ctl capsules`

Capsule inventory and activation front-end. This wraps the `scripts/world-capsules.sh`
workflow and is the preferred operator entry point for capsule creation and
deployment.

```sh
dune-ctl capsules inventory
dune-ctl capsules create --env live
dune-ctl capsules package validate --env live
dune-ctl capsules package install --env live
dune-ctl capsules images load --env live
dune-ctl capsules activate --env live --world-id sh-db3533a2d5a25fb-silakw
```

### `dune-ctl maps`

```sh
dune-ctl maps list            # all 28 maps with current phase
dune-ctl maps start <name>    # start a stopped map
dune-ctl maps stop  <name>    # stop a running map
dune-ctl maps start SH_Arrakeen --force   # bypass social-hub guard
```

Map names are case-sensitive and match the Kubernetes ServerSet names
(`Survival_1`, `DeepDesert_1`, `Overmap`, `SH_Arrakeen`, etc.).

Social hub maps (`SH_*`) are director-managed. Starting one manually puts the
game in an inconsistent state unless the director has already allocated it.
Use `--force` only when you know what you are doing. Prefer joining the map
in-game to trigger director allocation.

### `dune-ctl sietches`

```sh
dune-ctl sietches list        # Sietch table with phase/ready/players/port
dune-ctl sietches start       # start the primary Sietch
dune-ctl sietches stop        # stop the primary Sietch
dune-ctl sietches restart     # rolling restart of the primary Sietch
```

### `dune-ctl battlegroup`

```sh
dune-ctl battlegroup start
dune-ctl battlegroup stop
dune-ctl battlegroup restart
```

Operates the entire battlegroup, not individual maps. After a restart, run
`dune-ctl gateway-patch` to reapply the RMQ port argument.

### `dune-ctl settings`

Reads/writes `UserEngine.ini` and `UserGame.ini` in the active settings profile.
See [Settings Catalog](#settings-catalog) for the full key list.

```sh
dune-ctl settings list                          # all keys + current local values
dune-ctl settings set sietch_name "Arrakis"     # update a key
dune-ctl settings set admin_password "hunter2"  # secret — displays as ********
dune-ctl settings toggle sandstorm              # flip a boolean
dune-ctl settings status                        # show local-vs-deployed drift
dune-ctl settings diff                          # raw diff against deployed files
dune-ctl settings pull                          # overwrite local with deployed
dune-ctl settings apply                         # push local to filebrowser pod
dune-ctl settings apply-restart                 # apply + restart primary Sietch
```

Settings changes are local-only until `apply` is run. A Sietch restart is needed
for most settings to take effect in-game.

### `dune-ctl logs`

```sh
dune-ctl logs Survival_1              # last 100 lines from the game server pod
dune-ctl logs Survival_1 --tail 50   # last 50 lines
dune-ctl logs gateway -f             # stream gateway logs until Ctrl-C
dune-ctl logs postgres                # last 100 lines from the postgres pod
```

**Infra aliases:**

| Alias | Pod fragment matched |
|-------|---------------------|
| `gateway`, `sgw` | `sgw-deploy` |
| `director` | `bgd-deploy` |
| `postgres`, `db` | `db-dbdepl-sts` |
| `rabbitmq`, `rmq`, `mq` | `mq-game-sts` |
| `rabbitmq-admin`, `mq-admin` | `mq-admin-sts` |
| `filebrowser`, `fb` | `fb-deploy` |
| `text-router`, `textrouter`, `tr` | `tr-deploy` |

Map names (e.g. `Survival_1`, `Overmap`) are lowercased and `_` → `-` to
produce the pod substring (`survival-1`, `overmap`).

### `dune-ctl backup`

See [Backup Reference](#backup-reference).

### `dune-ctl players`

```sh
dune-ctl players    # table of online players or "No players currently online"
```

Queries `dune.encrypted_player_state` via `kubectl exec` into the postgres pod.
Character names are decrypted by the database's own `decrypt_user_data()`
function. Online is `online_status IN ('Online', 'LoggingOut')`.

The TUI Dashboard header also shows a live player count.

### `dune-ctl diagnostics`

Checks firewall backend (must be `iptables`, not `nftables`), gateway patch
presence, and other deployment-specific invariants.

### `dune-ctl update`

Full update pipeline: SteamCMD prefetch → funcom-patches → Funcom update →
gateway patch. Streams output live.

### `dune-ctl gateway-patch`

Reapplies `--RMQGameHttpPort=30196` to the gateway Deployment. Idempotent.
Required after any battlegroup restart because the gateway Deployment is
recreated.

### `dune-ctl token-check`

Prints FLS token expiry. Exits `2` if ≤ 14 days remain or token is expired.
Safe to call from cron for early warning.

---

## Settings Catalog

All settings read/write `UserEngine.ini` or `UserGame.ini` in the active profile.
Changes are local-only until `dune-ctl settings apply` is run.

| Key | File | Type | Notes |
|-----|------|------|-------|
| `port` | Engine | int | Player UDP base port |
| `igw_port` | Engine | int | Server-to-server UDP base port |
| `sietch_name` | Engine | string | Sietch display name shown in server browser |
| `sietch_password` | Engine | string *(secret)* | Login password; empty = open |
| `mining_output` | Engine | float | Player mining output multiplier |
| `vehicle_mining_output` | Engine | float | Vehicle mining output multiplier |
| `pvp_resource_multiplier` | Engine | float | PvP zone resource multiplier |
| `vehicle_durability_damage` | Engine | float | Vehicle durability damage multiplier |
| `sandstorm` | Engine | bool | Sandstorm on/off (0/1) |
| `sandstorm_treasure` | Engine | bool | Sandstorm treasure spawns |
| `sandworm` | Engine | bool | Sandworm on/off |
| `vehicle_worm_collision` | Engine | bool | Sandworm kills vehicles |
| `worm_danger_zones` | Engine | bool | Sandworm danger zone indicators |
| `pvp_all` | Game | bool | Force PvP on all partitions |
| `security_zones` | Game | bool | Security zone enforcement |
| `item_deterioration_rate` | Game | float | Item decay update interval (seconds) |
| `coriolis_storm` | Game | bool | Coriolis auto-spawn |
| `landclaim_segments` | Game | int | Max landclaim segments per player |
| `building_restrictions` | Game | bool | Building restriction limits |
| `blueprint_max_extensions` | Game | int | Foundation levels (solido replicator) |
| `base_backup_max_extensions` | Game | int | Base backup restore foundation levels |
| `admin_password` | Game | string *(secret)* | In-game `AdminLogin` password |
| `allowed_gm_commands` | Game | list | Whitelisted GM commands, one per line |

**`admin_password`** — stored as `Password_Admin=<value>` (no quotes) in
`[AdminSetting.Global]` in `UserGame.ini`. Use `AdminLogin <password>` in the
in-game console to gain admin access.

**`allowed_gm_commands`** — stored as `+Allowed_GM_Commands=<cmd>` per line
(UE5 array-append format). Set with a newline-separated string:
```sh
dune-ctl settings set allowed_gm_commands "AdminLogin
AdminLogout
GiveItem
TeleportToPlayer"
```

**Drift** — `settings status` compares local files to the deployed copies in the
filebrowser pod. Drift is expected for `sietch_name` and `sietch_password` when
the deployed files contain Funcom's defaults from world creation. This is not an
error; it just means local overrides haven't been applied yet.

---

## Backup Reference

### How backups work

`dune-ctl backup run` shells to `~/dune-server/scripts/dune-backup.sh`, which:

1. Creates a `DatabaseOperation` CR (`spec.action: dump`) and waits for the
   Funcom database operator to run the dump pod and write a `.backup` file to
   `/funcom/artifacts/database-dumps/<battlegroup>/`.
2. Copies the dump into the bundle.
3. Captures Kubernetes metadata (BattleGroup CR, DatabaseDeployment, PVCs, etc.)
4. Captures local and deployed `User*.ini` files.
5. Runs `dune-ctl diagnostics` and saves the output.
6. Creates a compressed `.tar.gz` of the metadata.

All stdout/stderr is streamed live — you see each step as it runs. The DB dump
step takes the longest (typically 1–3 minutes while the operator creates the dump
pod and waits for it to complete).

Bundles land in `/srv/backups/dune/<environment>/<battlegroup>/<timestamp>/`
on the 151 GB backup volume (`dune-vg/backups`, ~149 GB free). `ptc` and
`live` are restore trust boundaries; `dune-ctl backup restore` refuses bundles
whose manifest environment does not match the current world.

### Commands

```sh
dune-ctl backup list                          # list bundles, newest first
dune-ctl backup run                           # full backup (DB + metadata)
dune-ctl backup run --skip-db                 # metadata/settings only (fast, ~5s)
dune-ctl backup run --keep 14                 # run + prune to 14 most recent
dune-ctl backup restore --yes 20260517-021045 # restore by timestamp
dune-ctl backup restore --yes /srv/backups/dune/<env>/<bg>/20260517-021045  # by path
dune-ctl backup schedule                      # install nightly cron at 3am, keep 14
dune-ctl backup schedule --cron "0 2 * * *" --keep 7   # custom schedule
dune-ctl backup schedule --show               # print installed schedule
dune-ctl backup schedule --remove             # remove scheduled job
```

### Restore procedure

Restore **overwrites the live database**. Always stop the battlegroup first.

```sh
# 1. Check what's available
dune-ctl backup list

# 2. Stop the battlegroup
dune-ctl battlegroup stop

# 3. Restore (--yes required)
dune-ctl backup restore --yes <timestamp>

# 4. Start and re-patch
dune-ctl battlegroup start
dune-ctl gateway-patch
dune-ctl status
```

### Retention

By default `backup run` does not prune. Pass `--keep N` to keep only the N most
recent bundles after a successful run. `N=0` disables pruning. The `schedule`
command defaults to `--keep 14` (two weeks of daily backups).

### Scheduled backups

`dune-ctl backup schedule` installs an entry in the `dune` user's crontab:

```
0 3 * * *   DUNE_CTL_WORLD=<bg> /path/to/dune-ctl backup run --keep 14  # dune-ctl-backup
```

The `# dune-ctl-backup` marker lets subsequent `schedule` calls find and
replace that line without touching anything else in the crontab.

---

## Implementation History

### dune-admin Assessment Phases (2026-05)

These four features were identified by assessing `/home/dune/Code/dune-awakening-truenas`
(an independent dune-admin Go TUI) and porting the operationally useful parts.

#### Phase 1 — Log streaming (`core/src/logs.rs`)

CLI: `dune-ctl logs <target> [--tail N] [-f]`
TUI: 5th tab "5 Logs"

`resolve_pod()` maps human-friendly aliases (`gateway`, `director`, `postgres`,
etc.) to pod name substrings and does a kubectl pod list substring search. Map
names are slugified (`Survival_1` → `survival-1`). `-f` uses
`tokio::process::Command::spawn()` + async `BufReader` for live streaming;
without `-f`, `kubectl::run()` captures and returns lines.

TUI two-pane layout: left side shows available targets (infra pods + running
maps); right side shows the last N lines, refreshed on selection change and on
the 5s poll interval. Separate `logs_task` JoinHandle keeps log fetches
independent of the health snapshot refresh.

#### Phase 2 — Backup/restore wiring (`core/src/backup.rs`)

CLI: `dune-ctl backup list|run|restore`

Thin wrappers around the existing `dune-backup.sh` (which already exceeds what
dune-admin does — it uses Funcom's `DatabaseOperation` CR rather than raw
pg_dump, and bundles k8s metadata + settings alongside the dump). `run()` and
`restore()` use `stream_command()` for live output. `restore()` stages the
`.backup` file into `/funcom/artifacts/database-dumps/<bg>/` then calls
`battlegroup.sh import`.

`--yes` is required for restore to prevent accidental invocation.

#### Phase 3 — Admin settings catalog (`core/src/settings.rs`)

Added two `ValueKind` variants:

- `StringRaw` — plain unquoted string (`Password_Admin=value`)
- `StringList` — UE5 array-append format (`+Key=Value` per line)

Added `get_value_list()` / `set_value_list()` for the array format. Added
`read_value()` dispatcher used by `list()`, `drift()`, and `diff()`.

New catalog entries:

- `admin_password` — `[AdminSetting.Global]` → `Password_Admin`, secret
- `allowed_gm_commands` — `[AdminSetting.Global]` → `+Allowed_GM_Commands`, list

Both require `[AdminSetting.Global]` to exist in `UserGame.ini`. The section
with default GM commands is pre-populated in both the shared template
(`server/scripts/setup/config/UserGame.ini`) and the active world profile.

#### Phase 4 — Players online status (`core/src/players.rs`)

CLI: `dune-ctl players`
TUI: player count in Dashboard header

`list_online()` and `count_online()` execute a `psql` query via
`kubectl exec` into the postgres pod. The actual table is
`dune.encrypted_player_state` (not `player_state` as in dune-admin — Funcom
encrypts character names at rest). Character names are decrypted by the
database's own `dune.decrypt_user_data()` function. Online status uses the
`playerconnectionstatus` enum `{Offline, LoggingOut, Online}`.

`count_online()` is called during every `HealthSnapshot::collect()` and its
result appears in the TUI header as `Players:N`. Failures are silently ignored
(`Result::ok()`) so a postgres hiccup doesn't break the rest of the dashboard.

### Backup Enhancement Phases (2026-05)

#### Backup Phase 3 — Bundle size tracking

Added `size_bytes: u64` to `BackupEntry`. `list()` calls `du -sb` per bundle
(one process call per entry, handles all file types correctly). `format_size()`
formats as `1.3 MB` / `245 MB` / `1.1 GB`. `backup list` output now includes a
Size column.

#### Backup Phase 1 — Schedule + retention

`backup run --keep N` prunes the oldest bundles after a successful run (default
14, 0 = disabled). `prune()` reuses `list()` and calls `remove_dir_all` on the
tail of the sorted list.

`backup schedule` writes to the `dune` user crontab via `crontab -`. A
`# dune-ctl-backup` marker on the installed line makes subsequent calls
idempotent (finds and replaces, never duplicates). `--show` and `--remove`
manage the installed entry without reinstalling.

#### Backup Phase 2 — TUI Backups tab

6th tab "6 Backups". Shows the bundle list (timestamp, age, DB flag, size).
Pressing `r` shows a confirmation modal then triggers `backup run` via
`run_streamed()`, which sends each output line to an
`UnboundedSender<String>`. The TUI drains the channel each event loop tick
(`try_recv` loop, ~200ms latency). Output accumulates in `backup_lines` and
is displayed in a split pane below the list. After completion the list
auto-refreshes.
