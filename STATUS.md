# Dune Server Setup — Status

Last updated: 2026-05-13, fully running — Survival_1 and Overmap up, VPA recommender live

## Current state: fully running ✅

| Namespace | Component | Status |
|---|---|---|
| kube-system | coredns, local-path-provisioner, metrics-server, traefik | Running |
| kube-system | vpa-recommender (Off mode, memory only) | Running |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | DeepDesert_1 game server | Running (954Mi actual, 3Gi req / 10Gi limit) |
| cert-manager | cert-manager, cainjector, webhook | Running |
| funcom-operators | battlegroupoperator, databaseoperator, serveroperator, utilitiesoperator | Running |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | postgres, rabbitmq, gateway, director, text-router, filebrowser | Running |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | Survival_1 game server | Running |
| funcom-seabass-sh-db3533a2d5a25fb-xyyxbx | Overmap | Running (swap-backed, 200Mi request / 1Gi limit) |

Battlegroup: `sh-db3533a2d5a25fb-xyyxbx` ("Slackware-Arrakis"), Phase: Healthy

## RAM picture

Conan Exiles Enhanced is co-hosted and uses ~9.5 GB RSS, leaving ~5.8 GB available.
Total swap is now **62 GB** (zram 15.5 + dune-vg SSD 32 + sdc1 15.4).

Two paths to get Overmap running:

1. **Experimental swap (now)** — run `~/dune-server/server/scripts/setup/experimental_swap.sh` as the `dune` user. Lowers Overmap's memory request to 200 Mi so Kubernetes will schedule it against swap. Requires k3s restart (~2 min downtime). Overmap's actual usage is small (1 Gi limit) so swap penalty should be acceptable.

2. **New motherboard (preferred)** — 64 GB fully recognised, Overmap comes up automatically with no config changes needed. No swap required. ETA was 2026-05-09; confirm board is seated and RAM recognised with `free -h` after reboot.

## VPA memory recommendations

VPA 1.6.0 recommender deployed 2026-05-13. Off mode — recommendations only, no auto-apply.

**Standard workloads** (9 VPA objects in battlegroup namespace): postgres, rabbitmq, gateway, director, text-router, filebrowser, db-util-mon, db-util-pghero, and the BGD deploy. Recommendations populate after ~24h.

```sh
sudo kubectl get vpa -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx
sudo kubectl describe vpa <name> -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx
```

**Game servers** (Survival_1, Overmap) use Funcom's ServerSet CRD — VPA can't target them.
`watch-gameservers.sh` polls metrics-server and logs a RECOMMEND line when usage exceeds request + 20%.

```sh
~/dune-server/scripts/vpa/watch-gameservers.sh --once
```

Readings (2026-05-13): Survival_1 3313Mi/12Gi (28%), Overmap 165Mi/1Gi (17%), DeepDesert_1 954Mi/10Gi (10%).

Total game server RSS ~4.4Gi — fits comfortably in available RAM alongside Conan (~9.5GB). No meaningful swap pressure observed with all three running.

To tune game server memory: edit `experimental_swap.sh` map_to_requests and re-run, or patch the BattleGroup CR directly (see CLAUDE.md VPA section for the patch command).

## What still needs doing

- [ ] Confirm motherboard swap outcome (64 GB recognised?) — once in, reboot and verify Overmap comes up without swap
- [ ] After board swap: keep `experimental_swap.sh` request/limit split or tune via VPA data rather than reverting
- [ ] Set up backup jobs (scripts TBD) writing to `/srv/backups/dune/` and `/srv/backups/conan/`
- [ ] Off-server backup strategy (rsync to NAS / rclone to cloud — TBD)
- [ ] Create `settings.conf` (`printf '\n\n\n192.168.254.200\n' > ~/.dune/settings.conf`) — skipped during manual bootstrap, may be needed if external connectivity issues arise

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
   (or add to rc.local above the scheduler if you want fully automatic)

## Key paths

| Thing | Path |
|---|---|
| Server files | `~/dune-server/server/` |
| Funcom scripts | `~/dune-server/server/scripts/` |
| Our scripts | `~/dune-server/scripts/` |
| Battlegroup mgmt | `~/dune-server/server/scripts/battlegroup.sh` |
| Scheduler daemon | `~/dune-server/scripts/memory-focused-scheduler.sh` |
| Scheduler log | `~/dune-server/logs/memory-focused-scheduler.log` |
| k3s log | `~/dune-server/logs/k3s.log` |
| World config | `~/.dune/sh-db3533a2d5a25fb-xyyxbx.yaml` |
| DOWNLOAD_PATH | `~/.dune/download` → `~/dune-server/server/` |
| Dune backups | `/srv/backups/dune/` |
| Conan backups | `/srv/backups/conan/` |
| VPA scripts | `~/dune-server/scripts/vpa/` |
| Map toggle | `~/dune-server/scripts/map-toggle.sh` |
| Windows reference | `~/steamcmd/dune_server/` |
