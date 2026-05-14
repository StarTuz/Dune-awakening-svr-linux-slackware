# Dune Server Setup — Status

Last updated: 2026-05-13 — Survival_1, Overmap, and DeepDesert_1 all running

## Current state: fully running ✅

| Namespace | Component | Status |
|---|---|---|
| kube-system | coredns, local-path-provisioner, metrics-server, traefik | Running |
| kube-system | vpa-recommender (Off mode, memory only) | Running |
| cert-manager | cert-manager, cainjector, webhook | Running |
| funcom-operators | battlegroupoperator, databaseoperator, serveroperator, utilitiesoperator | Running |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | postgres, rabbitmq, gateway, director, text-router, filebrowser | Running |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | Survival_1 | Running (~3.3 Gi RSS, 5 Gi req / 12 Gi limit) |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | Overmap | Running (~165 Mi RSS, 200 Mi req / 1 Gi limit, swap-backed) |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | DeepDesert_1 | Running (~954 Mi RSS, 3 Gi req / 10 Gi limit) |

Battlegroup: `sh-db3533a2d5a25fb-xyyxbx` ("Slackware-Arrakis"), Phase: Healthy

## RAM picture

- Physical RAM: 16 GB
- Conan Exiles Enhanced (co-tenant): ~9.5 GB RSS
- Available: ~6.5 GB
- Game servers in use: ~4.4 Gi RSS (Survival_1 + Overmap + DeepDesert_1)
- **Result: all three game servers fit in available RAM. Swap is not under pressure.**

Total swap: **62 GB** (zram 15.5 + dune-vg SSD 32 + sdc1 15.4) — available as headroom only.

Overmap's *request* is 200 Mi (swap mode) but its actual RSS is ~165 Mi, so it barely touches swap. Deep Desert's request is 3 Gi but actual RSS is ~954 Mi. Survival_1's request is 5 Gi against ~3.3 Gi actual RSS. The gap between request and reality is wide — there is room to start additional maps if needed.

**After motherboard swap (64 GB):** Overmap can run at full allocation (remove the 200 Mi request patch). All game servers will comfortably fit with no swap dependency.

## Map management

```sh
# See all 28 maps and which are currently on/off
~/dune-server/scripts/map-toggle.sh list

# Start or stop a map
~/dune-server/scripts/map-toggle.sh start DeepDesert_1
~/dune-server/scripts/map-toggle.sh stop  DeepDesert_1
```

**Important:** Do not patch `ServerSet` or `ServerGroup` replicas directly. Starting a map requires patching both the `BattleGroup CR` and the `ServerSetScale` — `map-toggle.sh` handles both. Patching only the BattleGroup CR propagates through ServerGroup and ServerSet but the `ServerSetScale` (final pod-creation trigger) does not auto-update, leaving the map stuck in `Stopped` phase.

After a k3s restart, maps do not come back automatically — use `map-toggle.sh start` or `battlegroup.sh restart`.

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

## What still needs doing

- [ ] Confirm motherboard swap outcome (64 GB recognised?) — reboot and verify with `free -h`
- [ ] After board swap: raise Overmap request back to its natural limit (remove 200 Mi swap patch)
- [ ] Set up backup jobs writing to `/srv/backups/dune/` and `/srv/backups/conan/`
- [ ] Off-server backup strategy (rsync to NAS / rclone to cloud — TBD)
- [ ] Create `settings.conf` (`printf '\n\n\n192.168.254.200\n' > ~/.dune/settings.conf`) — missing, no known failures yet

## Bootstrapping fixes applied (fresh cluster workarounds)

The Funcom scripts assume a cloud-provisioned base — these were done manually:

- **cert-manager v1.8.0** installed via official manifest (not in download package)
- **ServiceMonitor CRD** installed (prometheus-operator CRD needed by database operator)
- **Funcom operator deployments** created from scratch (namespace, SA, CRB, Deployments)
- **Webhook TLS** — self-signed cert mounted into all 4 operator pods
- **operator.sh** — fixed `kubectl replace` → `kubectl apply --server-side` for fresh installs
- **world.sh** — added Europe Test / North America Test regions to the region menu
- **memory-focused-scheduler** — host daemon deployed; auto-starts via `/etc/rc.d/rc.local`
- **root-setup.sh** ✅ ran 2026-05-13 — k3s shims, rc.k3s, sudoers, LVM swap + backup volume
- **experimental_swap.sh** ✅ ran 2026-05-13 — swap enabled, all map memory requests patched down

## Storage (as of 2026-05-13)

| Device | Use |
|---|---|
| `/dev/sdc2` 916 GB HDD | btrfs root |
| `/dev/sdc1` 15.4 GB | swap pri -2 |
| `/dev/zram0` 15.5 GB | swap pri 100 |
| `dune-vg/swap` 32 GB SSD | swap pri -1 |
| `dune-vg/backups` ~150 GB SSD | `/srv/backups`, btrfs+zstd |

## Boot sequence (on reboot)

rc.local starts (in order):
1. QEMU guest agent (existing)
2. `memory-focused-scheduler` daemon
3. k3s must be started manually: `sudo rc-service k3s start`

After k3s is up, maps do not restart automatically. Start them:
```sh
~/dune-server/server/scripts/battlegroup.sh restart
# or individually:
~/dune-server/scripts/map-toggle.sh start Survival_1
~/dune-server/scripts/map-toggle.sh start Overmap
~/dune-server/scripts/map-toggle.sh start DeepDesert_1
```

## Key paths

| Thing | Path |
|---|---|
| Server files | `~/dune-server/server/` |
| Funcom scripts | `~/dune-server/server/scripts/` |
| Our scripts | `~/dune-server/scripts/` |
| Battlegroup mgmt | `~/dune-server/server/scripts/battlegroup.sh` |
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
