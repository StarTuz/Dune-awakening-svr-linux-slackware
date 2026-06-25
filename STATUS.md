# Dune Server Setup — Status

Last updated: 2026-05-26 — active capsule is now the Live world `Ixware` (`sh-db3533a2d5a25fb-silakw`). PTC capsule `Slackware-Arrakis` (`sh-db3533a2d5a25fb-xyyxbx`) is configured but cold. Earlier sections of this document still describe PTC-era validation and remain useful as history.

## Current state ✅

| Namespace | Component | Status |
|---|---|---|
| kube-system | coredns, local-path-provisioner, metrics-server, traefik | Running |
| kube-system | vpa-recommender (Off mode, memory only) | Running |
| cert-manager | cert-manager, cainjector, webhook | Running |
| funcom-operators | battlegroupoperator, databaseoperator, serveroperator, utilitiesoperator | Running |
| funcom-seabass-sh-db3533a2d5a25fb-silakw | postgres, rabbitmq, gateway, director, text-router, filebrowser | Running |
| funcom-seabass-sh-db3533a2d5a25fb-silakw | Survival_1 + Overmap | Running (Live capsule; DeepDesert_1 started on demand) |

Active battlegroup: `sh-db3533a2d5a25fb-silakw` ("Ixware", Live, region North America), Phase: Healthy (verified `kubectl get battlegroups -A` 2026-05-26, ~7d age, 8h uptime).

Security audit state:

- `~/dune-server/scripts/security-audit.sh` reports the expected public Dune/RMQ ports only.
- Director, Filebrowser, Postgres, k3s API, and RabbitMQ admin ports stay private behind the host firewall.
- The audit is still useful after every gateway or update change, because Funcom patches can regenerate the gateway deployment and shift what is exposed.

## FLS server browser (PTC-era history)

> **Historical** — this section describes validation of the **PTC** world
> `Slackware-Arrakis` in May 2026, before the Live `Ixware` cutover. Live
> browser visibility under the official self-hosted browser tab should be
> re-validated independently rather than inherited from these notes.

"Slackware-Arrakis" appears in the EXPERIMENTAL browser tab with "Arrakis-SlackwareLinux" (password-protected) below it. The "0 ms" ping shown is a Funcom-side display anomaly affecting every server in the list, not specific to ours.

### Likely root cause: stale build version

The server only became visible after updating from build `23147813` to `23216207` (battlegroup image `1957345-0-shipping`). The gateway `--RMQGameHttpPort=30196` fix was applied earlier the same day and did not produce visibility on its own — the version bump was the last thing changed before the browser showed the server.

Probable mechanism: FLS rejects outdated builds from the browser to prevent players from joining incompatible servers. Funcom does not document a minimum build requirement, but the behaviour fits.

**Future debugging**: if the server vanishes from the browser, *first* run `~/dune-server/scripts/update.sh` and wait at least 5 minutes for `DeclareBattlegroupUpdates` to re-fire. After a Funcom update, allow longer than a plain restart; fresh image pulls and first-run work can push that window past 10 minutes.

### What was found and fixed

**`GameRmqHttpAddress: "47.145.31.211:None"` (fixed 2026-05-14)**

The gateway's Python service discovers RabbitMQ NodePorts via the Kubernetes API. It successfully found the `amqp` port (NodePort 31982) but not the `http` port (NodePort 30196) because the port name in the service doesn't match what the code expects. This caused every `GatewayDeclareFarmStatus` FLS call to send `GameRmqHttpAddress: "47.145.31.211:None"`.

Fix: added `--RMQGameHttpPort=30196` to the gateway Deployment args via JSON patch. The gateway now sends `GameRmqHttpAddress: "47.145.31.211:30196"` correctly.

**Caveat**: this patch is applied to the Deployment directly. The server-operator regenerates the gateway Deployment from the BattleGroup CR on every restart or update, wiping the patch. Scripts exist to re-apply it:

```sh
# After any battlegroup restart:
~/dune-server/scripts/gateway-patch.sh

# For updates (does update + patch in one step):
~/dune-server/scripts/update.sh
```

### Confirmed correct (do not re-investigate)

| Item | Status |
|---|---|
| Router port forwarding UDP 7782-7790 | ✅ confirmed in place |
| Router port forwarding TCP 31982 (RMQ AMQP NodePort) | ✅ confirmed in place |
| `DatacenterId` / `-FarmRegion=` / director env var all set to `"North America Test"` | ✅ |
| `GameRmqAddress: "47.145.31.211:31982"` | ✅ |
| `GameRmqHttpAddress: "47.145.31.211:30196"` | ✅ (fixed 2026-05-14) |
| Survival_1 declared to FLS (`DeclareBattlegroupUpdates` with UpDeclarations, partition 1) | ✅ |
| 8-hour heartbeat firing (`HeartbeatUpdatesByPartitionId`) | ✅ (confirmed 13:46 UTC 2026-05-14) |

### FLS declaration chain (for reference)

1. **`GatewayDeclareFarmStatus`** — gateway, once at startup. Registers the farm: `DatacenterId`, `BattlegroupId`, `DisplayName`, `GameRmqAddress`, `GameRmqHttpAddress`. This is the call that had the `None` bug.

2. **`DeclareBattlegroupUpdates`** — director, ~4 minutes after game server pods start. Triggered when the BGD subsystem (BattlegroupDirectorSubsystem) initializes inside the game server and sends its first `ready=true` ServerState via Admin RMQ to the director. Contains `UpDeclarationsByPartitionId` for Survival_1 (partition 1). **Wait at least 5 minutes** after a battlegroup restart before checking the browser. **After a Funcom update, allow longer** (10+ minutes observed) — fresh image pulls, asset reverification, and longer first-run GC sweeps in the game pod push the BGD-ready timestamp further out than a plain restart. Don't start firewall/FLS diagnosis until the pods have had time to settle.

3. **`HeartbeatUpdatesByPartitionId`** — director, every 8 hours (`FlsServerHeartbeatUpdateFrequencySeconds=28800`). Refreshes the declaration to prevent expiry.

**Note on Overmap**: The director pre-loads Overmap's server ID from the `world_partition` DB at startup, so it never sees a DOWN→UP transition and never sends an `UpDeclaration` for it. This is expected — `IsStartingMap: false` for Overmap, so FLS only needs Survival_1 for browser visibility.

## Planned reboot / host maintenance

Use the clean Dune shutdown command before rebooting the host:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware shutdown --yes
```

This takes a full backup, patches the BattleGroup to stopped, then waits for
game servers to stop. It does not reboot the host. After boot, start the world
and reapply the gateway patch:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware battlegroup start
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware gateway-patch
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware preflight
```

The TUI Dashboard exposes the same workflow on `Q` with confirmation.

## How to update

```sh
# Preferred — full pipeline:
~/dune-server/scripts/update.sh

# Resume after a failure where backup+stop already completed:
~/dune-server/scripts/update.sh --skip-backup --skip-stop --start-after

# Resume after Funcom update completed but DB/gateway/start did not:
~/dune-server/scripts/update.sh --post-update-only --start-after

# What it does internally:
#   1. dune-backup.sh  (host bundle + DB dump unless --skip-backup)
#   2. Stop BattleGroup  (unless --skip-stop; avoids updating live maps)
#   3. steamcmd +app_update 3104830 validate  (runs as the sudo caller with
#      HOME=/home/dune; avoids root's Steam state; the `validate` flag
#      works around Funcom revoking old PTC depot manifests)
#   4. funcom-patches.sh  (re-applies our Slackware patches to
#      server/scripts/setup/experimental_swap.sh, overwritten by SteamCMD)
#   5. battlegroup.sh update  (Funcom flow: steamcmd, operator
#      image+CRD update, BattleGroup CR patched to new image revision —
#      triggers rollout; wrapper clears stale ~/.dune/bin symlinks first
#      because Funcom setup/system.sh uses non-idempotent `ln -s`)
#   6. funcom-patches.sh again  (guards against battlegroup.sh update overwrites)
#   7. db-credentials.sh check/fix  (guards against Postgres password drift)
#   8. If --start-after, start the BattleGroup, then gateway-patch.sh waits
#      for the gateway Deployment and restores --RMQGameHttpPort=30196 if it
#      was wiped)
```

By default the wrapper leaves the battlegroup stopped after a successful update.
Use `~/dune-server/scripts/update.sh --start-after` if you want it restarted
automatically.

The Funcom Windows deployment runs `battlegroup.bat` → `battlegroup.ps1` → SSH into VM → `battlegroup.sh update`. Our setup adds backup, stop, `validate` pre-fetch, Slackware patch re-application, DB credential verification/repair, and the gateway patch around that.

2026-05-15 update note: Funcom added three maps to `experimental_swap.sh`
(`CB_Overland_S_07`, `CB_Overland_S_08`, `CB_Dungeon_ThePit`). The local
`scripts/funcom-patches/experimental_swap.sh` and `.upstream` baseline have
been refreshed with those entries.

2026-05-15 update wrapper note: after a BattleGroup image patch, the DB pod may
exist before Postgres is listening. `db-credentials.sh` now waits for
`pg_isready` before deciding whether credentials are actually broken. The
updated operator may also expose Postgres on `5432`; the guard discovers the
live port from the DatabaseDeployment/status/service instead of assuming the
older `15432` value.

## RAM picture

- Physical RAM: 16 GB
- Conan Exiles Enhanced (co-tenant): ~9.5 GB RSS
- Available: ~6.5 GB
- Game servers in use (Survival_1 + Overmap): ~3.5 Gi RSS (DeepDesert_1 currently stopped)
- **Result: servers fit in available RAM. Swap is not under pressure.**

Total swap: **62 GB** (zram 15.5 + dune-vg SSD 32 + sdc1 15.4) — headroom only.

Overmap's *request* is 200 Mi (swap mode) but actual RSS is ~165 Mi. Survival_1 request is 5 Gi against ~3.3 Gi actual. Wide gap between request and reality — room to start more maps.

**After motherboard swap (64 GB):** remove the 200 Mi swap-mode request from Overmap; all servers comfortably in RAM.

## Map management

```sh
# See all 28 maps and which are on/off
~/dune-server/scripts/map-toggle.sh list

# Start or stop a map
~/dune-server/scripts/map-toggle.sh start DeepDesert_1
~/dune-server/scripts/map-toggle.sh stop  DeepDesert_1
```

**Important:** Do not patch `ServerSet` or `ServerGroup` replicas directly. Starting a map requires patching both the `BattleGroup CR` and the `ServerSetScale` — `map-toggle.sh` and `dune-ctl maps start|stop` handle both. Patching only the BattleGroup CR leaves the map stuck in `Stopped` phase because `ServerSetScale` (the final pod-creation trigger) does not auto-update.

For dedicated-scaled maps, `ServerSetScale.spec.partitions` must also be patched on start. If `ServerSet` has `partitions: [3]` but `ServerSetScale` has `replicas: 1` and no `partitions`, the operator can create the right map with the wrong dynamic pod/partition path. This caused the 2026-05-17 social hub regression: Arrakeen/Harko started as `pod-0` or stayed in startup until the start tooling was fixed to patch both `replicas` and `partitions`.

After a k3s restart, maps do not come back automatically — use `map-toggle.sh start` or `battlegroup.sh restart` (followed by `gateway-patch.sh`).

**Map persistence (auto-restart across reboots).** Start/stop (replicas) is a
separate layer from the director's per-map `MinServers` in `director.ini`. With
`MinServers = 0` (default for every map) the director powers a map back down
when nothing needs it and never brings it back after a restart — the root of the
"maps don't auto-restart after reboot" behaviour and the 2026-05-17 social-hub
power-down. To make a map come back on its own, toggle it persistent with
`dune-ctl --world Ixware maps persist <Map> --on --yes` (writes the live CR and
mirrors the capsule `battlegroup.yaml`; `--off` reverts). `--on` does not start
the map now and `--off` is required before a `maps stop` will stick. `maps list`
/ `status` / the TUI show persistence state. Social hubs (`SH_*`) are left on the
director's on-demand handshake by default and should not be forced persistent.

## VPA memory recommendations

VPA 1.6.0 recommender deployed 2026-05-13. Off mode — recommendations only, no auto-apply.

**Standard workloads** (9 VPA objects): postgres, rabbitmq, gateway, director, text-router, filebrowser, db-util-mon, db-util-pghero, bgd-deploy. Recommendations populate after ~24h.

VPA objects were created against the PTC namespace. Re-run
`scripts/vpa/vpa-objects.sh` to create the equivalent Off-mode objects under
the active Live namespace `funcom-seabass-sh-db3533a2d5a25fb-silakw` when
recommendations on the live workloads are wanted.

```sh
sudo kubectl get vpa -n funcom-seabass-sh-db3533a2d5a25fb-silakw
sudo kubectl describe vpa <name> -n funcom-seabass-sh-db3533a2d5a25fb-silakw
```

**Game servers** use Funcom's ServerSet CRD — VPA can't target them. Use `watch-gameservers.sh`:

```sh
~/dune-server/scripts/vpa/watch-gameservers.sh --once
```

Baseline readings (2026-05-13, single user): Survival_1 ~3.3 Gi, Overmap ~165 Mi, DeepDesert_1 ~954 Mi.

## Security hardening (2026-05-14) ✅

All items applied. Details in CLAUDE.md § Security.

| Item | Status |
|---|---|
| firewalld with iptables backend (k3s-safe) | ✅ configured, public + trusted zones, boot entry in rc.local |
| SSH key-only auth (RSA-4096 from defiant) | ✅ PasswordAuthentication + KbdInteractiveAuthentication both `no` |
| `~/.dune/*.yaml` permissions 600 | ✅ |
| PostgreSQL passwords rotated | ✅ ALTER USER applied; CR + on-disk YAML updated |
| k3s API `bind-address: 127.0.0.1` | ❌ REVERTED — breaks pod→API DNAT; firewall is sufficient |
| SNMP disabled | ✅ off |
| FLS token expiry tracking | ✅ in dune-ctl (`token-check`); token expires 2027-05-19, rotate by 2027-04-19 |
| dune-ctl world targeting | ✅ `worlds list`, `--world`, and per-world settings profiles; used to cut over PTC→Live |
| dune-ctl primary Sietch lifecycle | ✅ `sietches list/start/stop/restart`; start/stop/restart currently map to selected BattleGroup lifecycle |
| dune-ctl Sietch settings workflow | ✅ TUI shows name/password state and setting drift; `settings status` summarizes local-vs-deployed changes; `settings pull` syncs deployed User*.ini to local; `settings apply`/`apply-restart` require `--force` while drift exists |
| dune-ctl preflight | ✅ `preflight` checks firewall backend, gateway patch, FLS token, primary Sietch, settings drift, and RAM; `--strict` fails on warnings |
| Per-world settings profiles | ✅ Live: `~/.dune/worlds/sh-db3533a2d5a25fb-silakw/UserSettings`; PTC (cold): `~/.dune/worlds/sh-db3533a2d5a25fb-xyyxbx/UserSettings` |

## Live cutover (completed)

The PTC/Experimental world `Slackware-Arrakis` was the original battlegroup
under self-host PTC. With Funcom's official self-hosting launch the Live
capsule `Ixware` (`sh-db3533a2d5a25fb-silakw`, app `4754530`) was stood up as
a separate capsule and is now the active battlegroup. The PTC capsule is cold
on disk under `~/.dune/capsules/ptc/sh-db3533a2d5a25fb-xyyxbx/`.

Sequence used:

```sh
# Inspect available capsules
~/dune-server/dune-ctl/target/release/dune-ctl worlds list

# Initialize per-world UserSettings for the Live capsule
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware worlds init-settings

# Verify the Live world target
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware status

# Stop the PTC world (already done)
~/dune-server/dune-ctl/target/release/dune-ctl --world Slackware-Arrakis sietches stop
```

PTC and Live are cold-swap capsules and **must not run simultaneously** — they
use different operator image tags and would collide on cluster-wide NodePorts,
CRDs, and host firewall rules. See `WORLD-CAPSULES.md`.

## LAN client hairpin status (defiant / 192.168.254.17)

The old Frontier NVG468MQ router did not support NAT hairpin. After replacing it
with a TP-Link A7 and moving to external IP `47.145.31.211`, LAN login through
the normal FLS/browser path works. The old Steam launcher
`-ConnectToIP=192.168.254.200:7784` override was removed.

Historical fallback if hairpin breaks again:

```sh
sudo firewall-cmd --permanent --direct --add-rule ipv4 nat OUTPUT 0 \
    -d 47.145.31.211 -j DNAT --to-destination 192.168.254.200
sudo firewall-cmd --reload
```

Any LAN client can use the same rule (or equivalent iptables/nftables OUTPUT
DNAT) if a future router or firmware change breaks hairpin behavior.

## Hagga Basin travel timeout — root cause and status

**Symptom**: Player gets P34 timeout entering Hagga Basin (Survival_1) after the tutorial.

**Root cause (found 2026-05-15)**: arrakis had a stale nftables `table inet firewalld` active while firewalld was configured for `FirewallBackend=iptables`. The correct iptables firewalld rules allowed Dune UDP `7782-7790`, but the stale nft firewalld input hook still rejected incoming client packets with `ICMP admin prohibited`.

**Confirmed evidence**:

- `tcpdump -ni any 'host 192.168.254.17 and (udp port 7783 or icmp)'` showed client UDP reaching `192.168.254.200:7783`.
- The same capture showed arrakis replying with `ICMP host 192.168.254.200 unreachable - admin prohibited filter`.
- `iptables-save` showed the iptables firewalld path accepted LAN traffic via `IN_trusted`.
- `nft list tables` still showed `table inet firewalld`, and `nft list ruleset -a` showed stale nft firewalld input chains ending in `reject with icmpx admin-prohibited`.
- After `nft delete table inet firewalld`, tcpdump immediately showed two-way UDP traffic and the player loaded into Hagga Basin.

**Fix applied**:

```sh
nft delete table inet firewalld
firewall-cmd --reload
```

`/etc/firewalld/firewalld.conf` has `FirewallBackend=iptables`. After `firewall-cmd --reload`, `nft list tables` no longer shows `table inet firewalld`, so the stale table did not return.

**Do not use the old S2S-window workaround**: `scripts/s2s-watchdog.sh` was removed. The previous theory that players had to connect during a short Farm-session window was wrong for this incident.

**Related cleanup**: DeepDesert_1 was also corrected to a clean stopped state (`BattleGroup replicas=0`, `ServerSetScale=0`). The previous split state (`replicas=1`, `ServerSetScale=0`) caused confusing farm-size/partition-8 noise in logs and should not be treated as normal.

## 2026-05-17 reboot/memory-upgrade recovery

After the RAM upgrade/reboot, the host reported about 58.9 GB usable memory and the Dune stack fit comfortably with Survival_1 + Overmap around 27 GB total host RAM in use. k3s and the game pods came back, but several control-plane and map lifecycle issues were exposed.

**Scheduler fix**: `scripts/memory-focused-scheduler.sh` now resolves the Kubernetes node inside its loop instead of once at startup. If the scheduler starts before k3s publishes a node, it now logs `waiting for Kubernetes node` instead of trying to bind pods with an empty node name.

**Social hub regression**: while fixing hub crash loops, the start path initially patched `BattleGroup`/`ServerSet` partitions but not `ServerSetScale.partitions`. For dedicated-scaled maps, `ServerSetScale` overwrites matching ServerSet settings. The corrected behavior is:

```text
SH_Arrakeen:     ServerSetScale replicas=1, partitions=[3]
SH_HarkoVillage: ServerSetScale replicas=1, partitions=[4]
```

Both `scripts/map-toggle.sh` and `dune-ctl/core/src/maps.rs` were updated so `start` patches `ServerSetScale.spec.partitions` before `replicas`. `cargo fmt`, `cargo check`, and `bash -n scripts/map-toggle.sh` passed.

**Recovery actions performed**:

- Rebuilt `dune-ctl`.
- Restarted battlegroup director, server gateway, and text router.
- Started Arrakeen/Harko with corrected scaler partitions; pods came up as `sg-sh-arrakeen-pod-3` and `sg-sh-harkovillage-pod-4`.
- Restarted Survival_1 and Overmap game pods to refresh farm leadership.
- Director then powered down social hubs because `MinServers = 0` and no travel queue required them, so hub requested replicas were cleaned back to 0.

**Final live state after cleanup**:

- BattleGroup `Healthy`, size 2.
- `Survival_1` Running, ready, partition 1.
- `Overmap` Running, ready, partition 2.
- Social hubs stopped cleanly at 0.
- Director population declaration recovered from `BattlegroupMaxPlayerCapacity: 0` to `60`.
- Note: director still publishes `PasswordProtected {"Survival_1_0": true}` while `Bgd.ServerLoginPassword` remains set; clear the password only if testing browser visibility/filter behavior.

## What still needs doing

- [x] ~~Server browser visibility~~ — resolved 2026-05-14, "Slackware-Arrakis" visible in EXPERIMENTAL list
- [x] ~~Security hardening~~ — resolved 2026-05-14; see above
- [ ] Re-apply gateway patch after every restart: `~/dune-server/scripts/gateway-patch.sh`
- [ ] Confirm motherboard swap outcome (64 GB recognised?) — reboot and verify with `free -h`
- [ ] After board swap: raise Overmap request back to its natural limit (remove 200 Mi swap patch via `experimental_swap.sh`)
- [x] Add and verify Dune backup/restore runbook and host backup wrapper — full DB backup succeeded 2026-05-15; see `BACKUP-RESTORE.md` and `scripts/dune-backup.sh`
- [x] Schedule Dune backup jobs writing to `/srv/backups/dune/` — `dune-ctl backup schedule` installs nightly cron at 03:00, keeps 14
- [ ] Set up Conan backup jobs writing to `/srv/backups/conan/`
- [x] Off-server backup strategy — **live + verified 2026-06-24**
  (`OFFSITE-BACKUP.md`, `scripts/offsite-sync.sh`): two restic repos via rclone —
  Backblaze B2 (bucket `dune-backups-offsite`, 30-day Governance Object Lock,
  immutable) + Google Drive (`drive.file` scope) — both encrypted with one
  escrowed master passphrase (`~/.dune/offsite-restic-pass`, in password manager
  + printed). `restic check` clean on both; restore drill byte-identical from
  each. Nightly cron 03:20 + weekly check Sun 04:30. `restic` 0.19.0 / `rclone`
  1.74.3 in `~/.local/bin`. Switched the Drive leg from `rclone copy`+crypt to a
  second restic repo after rclone 1.74.3 `copy` hit an intermittent `fs/cache`
  panic unsafe for unattended cron.
- [ ] Create `settings.conf` (`printf '\n\n\n47.145.31.211\n' > ~/.dune/settings.conf`) — cosmetic, no known runtime failures
- [ ] **Rotate FLS token before 2027-04-19** (expires 2027-05-19) — update BattleGroup CR args (28 occurrences) + re-apply gateway patch
- [ ] **Set sietch password before official launch** — no password is set (fine for PTC; FLS browser not widely used yet). At official release the server is publicly visible to all players. Set with `dune-ctl settings set sietch_password <password> && dune-ctl settings apply` before going live on the official world.
- [x] dune-ctl operational polish — world targeting, primary Sietch lifecycle,
  settings drift guard, per-world settings profile, and TUI settings polish are
  in place
- [x] dune-ctl combined preflight command
- [x] dune-ctl log streaming — `dune-ctl logs <target> [-f]`; TUI tab 5
- [x] dune-ctl backup/restore — `dune-ctl backup list|run|restore`; TUI tab 6
- [x] dune-ctl admin settings — `admin_password`, `allowed_gm_commands` in catalog
- [x] dune-ctl players online — `dune-ctl players`; count in Dashboard header
- [ ] Future dune-ctl work — web UI and multi-Sietch research remain
  optional/future work

## Bootstrapping fixes applied (fresh cluster workarounds)

The Funcom scripts assume a cloud-provisioned base — these were done manually:

- **cert-manager v1.8.0** installed via official manifest (not in download package)
- **ServiceMonitor CRD** installed (prometheus-operator CRD needed by database operator)
- **Funcom operator deployments** created from scratch (namespace, SA, CRB, Deployments)
- **Webhook TLS** — self-signed cert mounted into all 4 operator pods
- **operator.sh** — fresh-install workaround: `kubectl replace` fails when CRDs don't yet exist. Before running `setup.sh` on a new cluster, manually apply CRDs with `sudo kubectl apply --server-side -f ~/dune-server/server/images/operators/crds/`. Not auto-patched — only matters during bootstrap
- **experimental_swap.sh** — Slackware patches now managed via `scripts/funcom-patches/`; re-applied automatically by `funcom-patches.sh` after every `update.sh` run
- **steamcmd wrapper** — `/usr/local/bin/steamcmd` execs `~dune/steamcmd/steamcmd.sh` (a bare symlink fails because steamcmd uses `dirname $0` to find its own files). Created by `root-setup.sh` Step 9
- **world.sh** — added Europe Test / North America Test regions to the region menu (one-time, only used during world creation; if lost, re-add manually before re-running)
- **memory-focused-scheduler** — host daemon deployed; auto-starts via `/etc/rc.d/rc.local`
- **root-setup.sh** ✅ ran 2026-05-13 — k3s shims, rc.k3s, sudoers, LVM swap + backup volume
- **experimental_swap.sh** ✅ ran 2026-05-13 — swap enabled, all map memory requests patched down
- **gateway `--RMQGameHttpPort=30196`** ✅ fixed 2026-05-14 — see `gateway-patch.sh`

## Storage (as of 2026-05-13)

| Device | Use |
|---|---|
| `/dev/sdc2` 916 GB HDD | btrfs root |
| `/dev/sdc1` 15.4 GB | swap pri -2 |
| `/dev/zram0` 15.5 GB | swap pri 100 |
| `dune-vg/swap` 32 GB SSD | swap pri -1 |
| `dune-vg/backups` ~150 GB SSD | `/srv/backups`, btrfs+zstd |

## Boot sequence (on reboot)

Before rebooting the host, run:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware shutdown --yes
```

This backs up the world, stops the BattleGroup, and waits for game servers to
stop. It does not reboot the host.

rc.local starts automatically:
1. firewalld
2. QEMU guest agent
3. `memory-focused-scheduler` daemon

Then manually:
```sh
sudo rc-service k3s start
```

After k3s is up, start the world and re-apply the gateway patch:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware battlegroup start
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware gateway-patch
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware preflight
```

Then check maps and explicitly start any on-demand travel maps that are needed.

## Key paths

| Thing | Path |
|---|---|
| Server files | `~/dune-server/server/` |
| Funcom scripts | `~/dune-server/server/scripts/` |
| Our scripts | `~/dune-server/scripts/` |
| Battlegroup mgmt | `~/dune-server/server/scripts/battlegroup.sh` |
| Update (with gateway patch) | `~/dune-server/scripts/update.sh` |
| Gateway patch (post-restart) | `~/dune-server/scripts/gateway-patch.sh` |
| Map toggle | `~/dune-server/scripts/map-toggle.sh` |
| Scheduler daemon | `~/dune-server/scripts/memory-focused-scheduler.sh` |
| Scheduler log | `~/dune-server/logs/memory-focused-scheduler.log` |
| k3s log | `~/dune-server/logs/k3s.log` |
| Active world config (Live capsule) | `~/.dune/capsules/live/sh-db3533a2d5a25fb-silakw/{capsule.env,battlegroup.yaml}` |
| Cold PTC world config | `~/.dune/sh-db3533a2d5a25fb-xyyxbx.yaml` |
| DOWNLOAD_PATH | `~/.dune/download` → `~/dune-server/server/` |
| Dune backups | `/srv/backups/dune/` |
| Conan backups | `/srv/backups/conan/` |
| VPA scripts | `~/dune-server/scripts/vpa/` |
| Windows reference | `~/steamcmd/dune_server/` |
