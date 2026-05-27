# Dune: Awakening Slackware Self-Host

Operations repository for running a Dune: Awakening self-hosted battlegroup
natively on Slackware Linux, co-hosted with the existing Conan Exiles Enhanced
server on `arrakis.algieba.org`.

`STATUS.md` is the current source of truth. `ARCHITECTURE.md` explains the
system shape and control loops. `FILE-LOCATIONS.md` indexes important paths.
`INSTALLER-DESIGN.md` captures the future cross-distro installer direction.
`PUBLIC-IP.md` documents public Internet IP changes and router checks.
`CLAUDE.md` contains detailed operator notes for future agent sessions.

## Current State

- Host: `arrakis.algieba.org`
- LAN IP: `192.168.254.200`
- Public IP: `47.145.31.211`
- Active battlegroup (Live capsule): `sh-db3533a2d5a25fb-silakw` / `Ixware`
- Namespace: `funcom-seabass-sh-db3533a2d5a25fb-silakw`
- Inactive PTC capsule: `sh-db3533a2d5a25fb-xyyxbx` / `Slackware-Arrakis`
- Current maps: `Survival_1`, `Overmap`, and `DeepDesert_1` can all be run together when validating travel/load behavior
- Server browser: visible in the PTC/Experimental browser
- Hagga Basin travel: confirmed working after firewall cleanup
- Platform: Slackware current, kernel `6.18.27`, k3s `v1.36.0+k3s1`
- Host sizing: 16 GiB RAM with heavy SSD-backed swap; Conan Exiles Enhanced is co-resident on the same machine

## Important Operational Notes

- Re-run `~/dune-server/scripts/gateway-patch.sh` after every battlegroup
  restart or update. The operator can regenerate the gateway deployment and
  lose `--RMQGameHttpPort=30196`.
- Start and stop maps only with `~/dune-server/scripts/map-toggle.sh` or
  `dune-ctl maps start|stop`. Do not patch `ServerSet` or `ServerGroup`
  replicas directly.
- Dedicated-scaled maps, including social hubs and most story/CB maps, need
  clean coordination between `BattleGroup`, `ServerSet`, and `ServerSetScale`.
  The bad split states are `BattleGroup replicas=1` with `ServerSetScale=0`,
  or `ServerSetScale.replicas=1` without the matching
  `ServerSetScale.partitions`. Both can leave maps absent, stuck in startup, or
  using the wrong pod/partition index.
- firewalld must use `FirewallBackend=iptables` — the nftables backend
  conflicts with k3s/flannel CNI. Verify with
  `grep FirewallBackend /etc/firewalld/firewalld.conf`.

- The current host is not memory-starved in the old sense, but it is swap-heavy
  by design. Use resource snapshots when you want a real picture of DD load
  rather than relying on old K3s-era assumptions.

## Common Commands

```sh
# Overall status
~/dune-server/server/scripts/battlegroup.sh status
sudo kubectl get battlegroup -A -o wide
sudo kubectl get serverset,serversetscale,serverstats -n funcom-seabass-sh-db3533a2d5a25fb-silakw

# dune-ctl world targeting
~/dune-server/dune-ctl/target/release/dune-ctl worlds list
~/dune-server/dune-ctl/target/release/dune-ctl --world sh-db3533a2d5a25fb-silakw status
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware preflight
~/dune-server/dune-ctl/target/release/dune-ctl --world sh-db3533a2d5a25fb-silakw worlds init-settings
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware sietches list
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware settings status

# Planned shutdown / restart / update
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware shutdown --yes
~/dune-server/server/scripts/battlegroup.sh restart
~/dune-server/scripts/gateway-patch.sh
~/dune-server/scripts/update.sh
~/dune-server/scripts/update.sh --skip-backup --skip-stop --start-after  # resume after backup+stop already completed
~/dune-server/scripts/update.sh --post-update-only --start-after          # resume after Funcom update completed
~/dune-server/scripts/db-credentials.sh check

# Maps
~/dune-server/scripts/map-toggle.sh list
~/dune-server/scripts/map-toggle.sh start DeepDesert_1
~/dune-server/scripts/map-toggle.sh stop DeepDesert_1
~/dune-server/dune-ctl/target/release/dune-ctl maps start SH_Arrakeen
~/dune-server/dune-ctl/target/release/dune-ctl maps stop SH_Arrakeen
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware maps list
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware maps start DeepDesert_1
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware maps stop DeepDesert_1

# Firewall sanity
grep -n '^FirewallBackend' /etc/firewalld/firewalld.conf
firewall-cmd --info-service=dune-game
~/dune-server/scripts/security-audit.sh

# Memory
free -h
swapon --show
~/dune-server/scripts/vpa/watch-gameservers.sh --once
sudo ~/dune-server/scripts/resource-snapshot.sh known-good-YYYYMMDD-resources

# Backups
dune-ctl backup list                          # list bundles with age/size
dune-ctl backup run                           # full backup (DB + metadata)
dune-ctl backup run --skip-db                 # metadata only (fast)
dune-ctl backup schedule                      # install/view nightly cron (3am, keep 14)
dune-ctl backup schedule --show               # view installed schedule
dune-ctl backup restore --yes <timestamp>     # restore a bundle (stop BG first)
sudo ~/dune-server/scripts/system-snapshot.sh known-good-YYYYMMDD  # full btrfs snapshot
```

## Networking

The Dune game UDP range is `7782-7790`. Conan Exiles owns `7777-7778` and other
ports documented in `CLAUDE.md`, so Dune is kept above that range.

## Multi-World Note

`dune-ctl` is world-aware. It discovers local `~/.dune/<battlegroup>.yaml`
world specs, ignores backup/dump companion YAMLs, and can target a specific
world with `--world <battlegroup-or-title>` or `DUNE_CTL_WORLD`.

By default settings use Funcom's shared local defaults in
`server/scripts/setup/config`. Before managing a second world, initialize a
per-world settings profile:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world <bg> worlds init-settings
```

After that, `settings list/set/apply` for that world uses
`~/.dune/worlds/<bg>/UserSettings/`. The PTC-to-official transition has been
completed: the Live capsule `Ixware` is now the active battlegroup, and the
PTC capsule `Slackware-Arrakis` is cold. The same per-world settings flow
applies when standing up additional worlds.

`dune-ctl sietches list` shows the selected world's primary Sietch. The
current self-host package exposes one Sietch per BattleGroup, so
`dune-ctl sietches start|stop|restart` intentionally maps to the selected
BattleGroup lifecycle. Keep using `maps start|stop <map>` for individual travel
maps such as `DeepDesert_1` or story instances.

For Sietch name/password changes, edit locally with `settings set` or the TUI,
then deploy with `dune-ctl settings apply`. Use
`dune-ctl settings apply-restart` when you want to deploy both `User*.ini`
files and immediately restart the selected world's primary Sietch. Check
pending local-vs-deployed managed setting changes with
`dune-ctl settings status`; the TUI Settings tab also shows a drift column. If
the deployed copy is the source of truth, sync it back into the local profile
with `dune-ctl settings pull` before making more edits. `settings apply` and
`settings apply-restart` refuse to overwrite deployed managed settings while
drift exists unless you pass `--force`.

Active world profile hygiene:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware settings status
```

The selected world currently uses:

```text
~/.dune/worlds/sh-db3533a2d5a25fb-silakw/UserSettings/
```

Managed drift should normally be `0 changed managed setting(s)`. If drift is
intentional, `settings apply --force` or `settings apply-restart --force` can
overwrite the deployed copy; otherwise use `settings pull` to make local match
the live deployed settings before further edits.

## dune-ctl Command Reference

Use `--world Ixware` or `--world sh-db3533a2d5a25fb-silakw` to target the
active Live capsule; `--world Ixware` targets the inactive PTC
capsule. The TUI is the default when no subcommand is provided.
Examples below use `dune-ctl`; replace that with
`~/dune-server/dune-ctl/target/release/dune-ctl` if it is not in `PATH`.

```sh
# World and health
dune-ctl worlds list
dune-ctl --world Ixware status
dune-ctl --world Ixware preflight
dune-ctl --world Ixware preflight --strict
dune-ctl --world Ixware diagnostics
dune-ctl --world Ixware token-check

# Primary Sietch lifecycle
dune-ctl --world Ixware sietches list
dune-ctl --world Ixware sietches start
dune-ctl --world Ixware sietches stop
dune-ctl --world Ixware sietches restart

# Maps / travel surfaces
dune-ctl --world Ixware maps list
dune-ctl --world Ixware maps start DeepDesert_1
dune-ctl --world Ixware maps stop DeepDesert_1

# Settings
dune-ctl --world Ixware settings list
dune-ctl --world Ixware settings status
dune-ctl --world Ixware settings pull
dune-ctl --world Ixware settings set sietch_name "Arrakis-SlackwareLinux"
dune-ctl --world Ixware settings set admin_password "secret"
dune-ctl --world Ixware settings apply
dune-ctl --world Ixware settings apply-restart

# Logs
dune-ctl logs Survival_1              # last 100 lines from game server pod
dune-ctl logs gateway -f              # stream gateway logs until Ctrl-C
dune-ctl logs postgres --tail 50      # last 50 postgres lines

# Backups
dune-ctl backup list
dune-ctl backup run
dune-ctl backup run --skip-db         # fast metadata-only bundle
dune-ctl backup run --keep 14         # run + prune to 14 most recent
dune-ctl backup schedule              # install nightly cron at 3am, keep 14
dune-ctl backup schedule --show       # view installed schedule
dune-ctl backup restore --yes <timestamp>   # restore (stop battlegroup first)

# Players
dune-ctl players                      # table of online players

# Update/security helpers
dune-ctl --world Ixware gateway-patch
~/dune-server/scripts/update.sh --start-after
~/dune-server/scripts/security-audit.sh
sudo ~/dune-server/scripts/resource-snapshot.sh known-good-YYYYMMDD-resources
```

Full CLI reference: `dune-ctl/OPERATIONS.md`

LAN clients behind the TP-Link A7 can connect through the public FLS/browser
path; NAT hairpin was confirmed working after removing the old Steam
`-ConnectToIP=192.168.254.200:7784` override. The old Frontier router required
a local OUTPUT DNAT workaround; keep this only as a fallback if hairpin breaks:

```sh
sudo firewall-cmd --permanent --direct --add-rule ipv4 nat OUTPUT 0 \
  -d 47.145.31.211 -j DNAT --to-destination 192.168.254.200
sudo firewall-cmd --reload
```

## Repository Layout

```text
server/                  Funcom server package and scripts
scripts/                 Local Slackware/operations scripts
scripts/funcom-patches/  Local patches re-applied after SteamCMD updates
dune-ctl/                Rust control/status tooling
BACKUP-RESTORE.md        Backup and restore runbook
STATUS.md                Current operational state
ARCHITECTURE.md          System architecture and control loops
FILE-LOCATIONS.md        Important paths and logs
CLAUDE.md                Detailed operator/agent guidance
dune-ctl/TUI-MASCOT.md   TUI mascot design note
```

## Historical Context

This started as an unsupported native Slackware deployment of Funcom's
self-hosted server, bypassing the supported Windows Pro + Hyper-V wrapper. The
Windows package remains useful as a reference, but the live deployment runs k3s
directly on Slackware.
