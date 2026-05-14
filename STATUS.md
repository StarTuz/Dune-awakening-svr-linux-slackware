# Dune Server Setup — Status

Last updated: 2026-05-14 — Security hardening complete; FLS browser visible

## Current state ✅

| Namespace | Component | Status |
|---|---|---|
| kube-system | coredns, local-path-provisioner, metrics-server, traefik | Running |
| kube-system | vpa-recommender (Off mode, memory only) | Running |
| cert-manager | cert-manager, cainjector, webhook | Running |
| funcom-operators | battlegroupoperator, databaseoperator, serveroperator, utilitiesoperator | Running |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | postgres, rabbitmq, gateway, director, text-router, filebrowser | Running |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | Survival_1 | Running (~3.3 Gi RSS, 5 Gi req / 12 Gi limit) |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | Overmap | Running (~165 Mi RSS, 200 Mi req / 1 Gi limit, swap-backed) |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | DeepDesert_1 | **Stopped** (ServerSetScale=0 after last restart — start with `map-toggle.sh start DeepDesert_1`) |

Battlegroup: `sh-db3533a2d5a25fb-xyyxbx` ("Slackware-Arrakis"), Phase: Healthy

## FLS server browser ✅ visible (as of 2026-05-14)

"Slackware-Arrakis" appears in the EXPERIMENTAL browser tab with "Arrakis-SlackwareLinux" (password-protected) below it. The "0 ms" ping shown is a Funcom-side display anomaly affecting every server in the list, not specific to ours.

### Likely root cause: stale build version

The server only became visible after updating from build `23147813` to `23216207` (battlegroup image `1957345-0-shipping`). The gateway `--RMQGameHttpPort=30196` fix was applied earlier the same day and did not produce visibility on its own — the version bump was the last thing changed before the browser showed the server.

Probable mechanism: FLS rejects outdated builds from the browser to prevent players from joining incompatible servers. Funcom does not document a minimum build requirement, but the behaviour fits.

**Future debugging**: if the server vanishes from the browser, *first* run `~/dune-server/scripts/update.sh` and wait at least 5 minutes for `DeclareBattlegroupUpdates` to re-fire. Only re-investigate FLS declarations if visibility doesn't return after an update.

### What was found and fixed

**`GameRmqHttpAddress: "47.145.51.160:None"` (fixed 2026-05-14)**

The gateway's Python service discovers RabbitMQ NodePorts via the Kubernetes API. It successfully found the `amqp` port (NodePort 31982) but not the `http` port (NodePort 30196) because the port name in the service doesn't match what the code expects. This caused every `GatewayDeclareFarmStatus` FLS call to send `GameRmqHttpAddress: "47.145.51.160:None"`.

Fix: added `--RMQGameHttpPort=30196` to the gateway Deployment args via JSON patch. The gateway now sends `GameRmqHttpAddress: "47.145.51.160:30196"` correctly.

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
| `GameRmqAddress: "47.145.51.160:31982"` | ✅ |
| `GameRmqHttpAddress: "47.145.51.160:30196"` | ✅ (fixed 2026-05-14) |
| Survival_1 declared to FLS (`DeclareBattlegroupUpdates` with UpDeclarations, partition 1) | ✅ |
| 8-hour heartbeat firing (`HeartbeatUpdatesByPartitionId`) | ✅ (confirmed 13:46 UTC 2026-05-14) |

### FLS declaration chain (for reference)

1. **`GatewayDeclareFarmStatus`** — gateway, once at startup. Registers the farm: `DatacenterId`, `BattlegroupId`, `DisplayName`, `GameRmqAddress`, `GameRmqHttpAddress`. This is the call that had the `None` bug.

2. **`DeclareBattlegroupUpdates`** — director, ~4 minutes after game server pods start. Triggered when the BGD subsystem (BattlegroupDirectorSubsystem) initializes inside the game server and sends its first `ready=true` ServerState via Admin RMQ to the director. Contains `UpDeclarationsByPartitionId` for Survival_1 (partition 1). **Wait at least 5 minutes** after a battlegroup restart before checking the browser.

3. **`HeartbeatUpdatesByPartitionId`** — director, every 8 hours (`FlsServerHeartbeatUpdateFrequencySeconds=28800`). Refreshes the declaration to prevent expiry.

**Note on Overmap**: The director pre-loads Overmap's server ID from the `world_partition` DB at startup, so it never sees a DOWN→UP transition and never sends an `UpDeclaration` for it. This is expected — `IsStartingMap: false` for Overmap, so FLS only needs Survival_1 for browser visibility.

## How to update

```sh
# Preferred — full pipeline:
~/dune-server/scripts/update.sh

# What it does internally:
#   1. steamcmd +app_update 3104830 validate  (pre-fetch; the `validate` flag
#      works around Funcom revoking old PTC depot manifests)
#   2. funcom-patches.sh  (re-applies our Slackware patches to
#      server/scripts/setup/experimental_swap.sh, overwritten by step 1)
#   3. battlegroup.sh update  (Funcom flow: steamcmd no-op now, operator
#      image+CRD update, BattleGroup CR patched to new image revision —
#      triggers rolling restart)
#   4. gateway-patch.sh  (restores --RMQGameHttpPort=30196 on the gateway
#      Deployment if it was wiped)
```

The Funcom Windows deployment runs `battlegroup.bat` → `battlegroup.ps1` → SSH into VM → `battlegroup.sh update`. Our setup adds the `validate` pre-fetch and Slackware patch re-application around that.

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

**Important:** Do not patch `ServerSet` or `ServerGroup` replicas directly. Starting a map requires patching both the `BattleGroup CR` and the `ServerSetScale` — `map-toggle.sh` handles both. Patching only the BattleGroup CR leaves the map stuck in `Stopped` phase because `ServerSetScale` (the final pod-creation trigger) does not auto-update.

After a k3s restart, maps do not come back automatically — use `map-toggle.sh start` or `battlegroup.sh restart` (followed by `gateway-patch.sh`).

## VPA memory recommendations

VPA 1.6.0 recommender deployed 2026-05-13. Off mode — recommendations only, no auto-apply.

**Standard workloads** (9 VPA objects): postgres, rabbitmq, gateway, director, text-router, filebrowser, db-util-mon, db-util-pghero, bgd-deploy. Recommendations populate after ~24h.

```sh
sudo kubectl get vpa -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx
sudo kubectl describe vpa <name> -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx
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
| FLS token expiry tracking | ✅ in dune-ctl (`token-check`); token expires 2027-05-08, rotate by 2027-04-08 |

## What still needs doing

- [x] ~~Server browser visibility~~ — resolved 2026-05-14, "Slackware-Arrakis" visible in EXPERIMENTAL list
- [x] ~~Security hardening~~ — resolved 2026-05-14; see above
- [ ] Re-apply gateway patch after every restart: `~/dune-server/scripts/gateway-patch.sh`
- [ ] Confirm motherboard swap outcome (64 GB recognised?) — reboot and verify with `free -h`
- [ ] After board swap: raise Overmap request back to its natural limit (remove 200 Mi swap patch via `experimental_swap.sh`)
- [ ] Set up backup jobs writing to `/srv/backups/dune/` and `/srv/backups/conan/`
- [ ] Off-server backup strategy (rsync to NAS / rclone to cloud — TBD)
- [ ] Create `settings.conf` (`printf '\n\n\n47.145.51.160\n' > ~/.dune/settings.conf`) — cosmetic, no known runtime failures
- [ ] **Rotate FLS token before 2027-04-08** (expires 2027-05-08) — update BattleGroup CR args (28 occurrences) + re-apply gateway patch
- [ ] Build dune-ctl (Rust TUI + web) — FLS token expiry warning is planned feature

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

rc.local starts automatically:
1. firewalld
2. QEMU guest agent
3. `memory-focused-scheduler` daemon

Then manually:
```sh
sudo rc-service k3s start
```

After k3s is up, maps do not restart automatically. Start them, then re-apply the gateway patch:
```sh
~/dune-server/server/scripts/battlegroup.sh restart
~/dune-server/scripts/gateway-patch.sh
# or start maps individually:
~/dune-server/scripts/map-toggle.sh start Survival_1
~/dune-server/scripts/map-toggle.sh start Overmap
~/dune-server/scripts/map-toggle.sh start DeepDesert_1
~/dune-server/scripts/gateway-patch.sh
```

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
| World config | `~/.dune/sh-db3533a2d5a25fb-xyyxbx.yaml` |
| DOWNLOAD_PATH | `~/.dune/download` → `~/dune-server/server/` |
| Dune backups | `/srv/backups/dune/` |
| Conan backups | `/srv/backups/conan/` |
| VPA scripts | `~/dune-server/scripts/vpa/` |
| Windows reference | `~/steamcmd/dune_server/` |
