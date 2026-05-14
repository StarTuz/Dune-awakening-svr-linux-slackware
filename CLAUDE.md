# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Repository Is

Operations repository for a Dune: Awakening self-hosted battlegroup running natively on Slackware Linux, co-hosted with an existing Conan Exiles Enhanced server. `README.md` is an early research snapshot; `STATUS.md` is the current authoritative state.

**Current state**: Fully running as of 2026-05-13. Both Survival_1 and Overmap are up. Overmap is swap-backed (200 Mi request / 1 Gi limit) via `experimental_swap.sh`. Conan Exiles Enhanced co-tenant uses ~9.5 GB RSS. Total swap: 62 GB (zram + dune-vg SSD + sdc1). VPA recommender is live (Off mode, memory only) tracking 9 standard workloads. Motherboard replacement to 64 GB pending — once done, Overmap can run at full allocation without swap.

---

## Host: arrakis.algieba.org

- **OS**: Slackware 15.0+, kernel 6.18.27, glibc 2.42, GCC 15.2.0
- **CPU**: Intel Core i7-9700 (8 cores, no SMT)
- **RAM**: 16 GB → 64 GB after ASUS Prime Z390-A motherboard swap
- **LAN**: `192.168.254.200/24`

---

## Conan Exiles (co-tenant — do not touch)

User `conan` (uid 1001), shell `/bin/ksh`. RSS ~9.5 GB.

Occupied ports: UDP 7777, 7778, 14001, 27015 — TCP 25575, 8088.

---

## Storage Layout

| Device | Size | Use |
|---|---|---|
| `/dev/sdc2` | 916 GB HDD | btrfs root (`/`) |
| `/dev/sdc1` | 15.4 GB | swap, priority -2 (slowest fallback) |
| `/dev/zram0` | 15.5 GB | swap, priority 100 (used first) |
| `/dev/sdb2` | 182.9 GB **SSD** | LVM VG `dune-vg` |
| `dune-vg/swap` | 32 GB | swap LV, priority -1 |
| `dune-vg/backups` | ~150 GB | btrfs, mounted `/srv/backups` |

Swap order: zram (RAM-backed, fastest) → dune-vg SSD → sdc1 HDD.

`/srv/backups/dune/` — owned `dune:users`
`/srv/backups/conan/` — owned `conan:users`

Off-server backup strategy TBD.

LVM setup is handled by `root-setup.sh` Step 8 (idempotent).

---

## Architecture

Funcom ships everything as offline OCI image tarballs — no internet required after the initial SteamCMD download. A k3s single-node cluster runs four Kubernetes operators in the `funcom-operators` namespace:

| Operator | Manages |
|---|---|
| `battlegroup-operator` | `BattleGroup` CRs |
| `database-operator` | Postgres StatefulSets |
| `server-operator` | Game server pods |
| `utilities-operator` | Supporting services (filebrowser, etc.) |

Each battlegroup gets its own namespace `funcom-seabass-<name>`. Inside: postgres, rabbitmq, gateway, director, text-router, filebrowser, and the game server pods.

**Current battlegroup**: `sh-db3533a2d5a25fb-xyyxbx` ("Slackware-Arrakis")
**Namespace**: `funcom-seabass-sh-db3533a2d5a25fb-xyyxbx`

### Windows vs our deployment

Funcom's Windows depot (3104831) ships a pre-built Hyper-V VM (`.vhdx` + `.vmcx`) running Alpine Linux with k3s inside. `battlegroup.bat` → PowerShell UI → SSH into VM → same Funcom scripts. Our deployment skips the Hyper-V layer and runs k3s directly on Slackware.

---

## Script Trees

### Funcom scripts — `~/dune-server/server/scripts/`

| Script | Purpose |
|---|---|
| `setup.sh` | One-shot first-time setup: k3s → system → world → images |
| `battlegroup.sh` | Day-to-day management: list, status, start, stop, restart, update, logs-export, apply-default-usersettings |
| `setup/k3s.sh` | Install k3s, load core images, start operators |
| `setup/system.sh` | Create `~/.dune/bin/battlegroup` symlink |
| `setup/world.sh` | Interactive world creation (name, region, FLS token, secrets, BattleGroup CR) |
| `setup/operator.sh` | Load operator images, apply CRDs, scale operator deployments |
| `setup/helper.sh` | Shared: `load_image_from_file` (with retry), `kubectl_retry`, `scale_deployment` |
| `setup/experimental_swap.sh` | Enable swap + patch battlegroup memory requests down for swap-backed scheduling |
| `setup/config/UserEngine.ini` | Game console variables (server name, password, mining, sandstorm, sandworm) |
| `setup/config/UserGame.ini` | Script sections (PvP/PvE, security zones, deterioration, building limits) |

### Our scripts — `~/dune-server/scripts/`

| Script | Purpose |
|---|---|
| `root-setup.sh` | Run once as root: installs k3s, creates shims, writes rc.k3s, sets sudoers, sets up LVM swap + backup volume |
| `memory-focused-scheduler.sh` | Custom Kubernetes scheduler daemon — binds pending pods to the single k3s node. Auto-starts via rc.local |
| `map-toggle.sh` | Start/stop individual maps. Usage: `map-toggle.sh list`, `map-toggle.sh start DeepDesert_1` |
| `sudoer.sh` | One-liner fallback to patch sudoers + restart k3s (emergency use) |
| `vpa/install.sh` | Install VPA recommender: downloads CRDs, applies RBAC + deployment, runs vpa-objects.sh |
| `vpa/recommender-rbac.yaml` | ServiceAccount + ClusterRoles + bindings for vpa-recommender in kube-system |
| `vpa/recommender-deployment.yaml` | vpa-recommender Deployment (image 1.6.0, tuned to 100Mi req / 256Mi limit) |
| `vpa/vpa-objects.sh` | Creates Off-mode VPA objects for every Deployment and StatefulSet in battlegroup namespaces |
| `vpa/watch-gameservers.sh` | Polls metrics-server for game server pod memory; logs RECOMMEND when usage > request + threshold |
| `vpa/vpa-v1-crd-gen.yaml` | VPA CRDs downloaded by install.sh (v1.6.0, do not hand-edit) |

---

## Slackware Adaptations

The Funcom scripts target Alpine Linux (OpenRC). These shims/fixes make them work on Slackware:

**`/usr/local/bin/rc-service`** (created by `root-setup.sh`):
```sh
#!/bin/sh
exec /etc/rc.d/rc.${1} ${2}
```
Translates `rc-service k3s start` → `/etc/rc.d/rc.k3s start`.

**`/usr/local/bin/rc-update`** (stub):
```sh
#!/bin/sh
echo "rc-update: $*  (stubbed on Slackware)"
```
`rc-update add k3s` calls are no-ops; k3s boot is handled by rc.local instead.

**`operator.sh`** — changed `kubectl replace` → `kubectl apply --server-side --force-conflicts` so CRD installs work on a fresh cluster.

**`world.sh`** — added "Europe Test" / "North America Test" to the region list.

**`experimental_swap.sh`** — patched for Slackware:
- Skips swapfile creation if swap is already active (we have ~30 GB via sdc1 + zram0)
- Replaces Alpine cgroup path (`/sys/fs/cgroup/openrc.k3s/memory.swap.max`) with a dynamic lookup of the k3s process cgroup using `/proc/<pid>/cgroup` and the cgroup v1 `memory.memsw.limit_in_bytes` interface

**`~/.dune/bin/battlegroup`** symlink — `system.sh` creates this but was never run during our manual bootstrap. Created manually: `ln -s ~/dune-server/server/scripts/battlegroup.sh ~/.dune/bin/battlegroup`.

**Bootstrapping fixes applied during initial setup** (documented in `STATUS.md`):
- cert-manager v1.8.0 installed via official manifest (not in download package)
- ServiceMonitor CRD installed (required by database operator)
- Operator deployments created from scratch (namespace, SA, CRB, Deployments)
- Webhook TLS: self-signed cert mounted into all 4 operator pods

---

## Missing: `settings.conf`

The Windows wizard writes the external IP to `/home/dune/.dune/settings.conf` before running `setup.sh`:
```
\n\n\n<external_ip>\n
```
`k3s.sh` expects this file to already exist. It **does not exist** on our deployment — we bootstrapped manually and this step was skipped. No known runtime failures from this, but worth investigating if external connectivity issues arise.

---

## Key Paths

| Thing | Path |
|---|---|
| Server files / `DOWNLOAD_PATH` | `~/dune-server/server/` (symlink: `~/.dune/download`) |
| Battlegroup CLI | `~/dune-server/server/scripts/battlegroup.sh` (also `~/.dune/bin/battlegroup`) |
| World config YAML | `~/.dune/sh-db3533a2d5a25fb-xyyxbx.yaml` |
| FLS / RMQ secrets | `~/.dune/sh-db3533a2d5a25fb-xyyxbx-{fls,rmq}-secret.yaml` |
| Game server config | `~/dune-server/server/scripts/setup/config/User{Engine,Game}.ini` |
| Scheduler daemon | `~/dune-server/scripts/memory-focused-scheduler.sh` |
| Scheduler log | `~/dune-server/logs/memory-focused-scheduler.log` |
| k3s log | `~/dune-server/logs/k3s.log` |
| Backup volumes | `/srv/backups/{dune,conan}/` |
| VPA scripts | `~/dune-server/scripts/vpa/` |
| Windows package | `~/steamcmd/dune_server/` (depot 3104831) |

---

## Management Commands

```sh
# Battlegroup
~/dune-server/server/scripts/battlegroup.sh list
~/dune-server/server/scripts/battlegroup.sh status
~/dune-server/server/scripts/battlegroup.sh start|stop|restart
~/dune-server/server/scripts/battlegroup.sh update              # SteamCMD pull + apply
~/dune-server/server/scripts/battlegroup.sh logs-export
~/dune-server/server/scripts/battlegroup.sh operator-logs-export
~/dune-server/server/scripts/battlegroup.sh apply-default-usersettings

# Cluster state
sudo kubectl get nodes
sudo kubectl get pods -A
sudo kubectl get battlegroups -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx
sudo kubectl get serverstats  -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx

# Director NodePort (internal port 11717, nodePort is dynamic)
sudo kubectl get svc -A -o jsonpath='{.items[*].spec.ports[?(@.port==11717)].nodePort}'
# File browser: http://192.168.254.200:18888/

# System health
free -h
swapon --show
ps -eo pid,user,rss,vsz,pmem,pcpu,cmd --sort=-rss | head
/usr/sbin/ss -tulpen

# VPA recommendations (populate after ~24h)
sudo kubectl get vpa -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx
~/dune-server/scripts/vpa/watch-gameservers.sh --once
```

---

## Boot Sequence (after reboot)

`/etc/rc.d/rc.local` starts automatically:
1. QEMU guest agent
2. `memory-focused-scheduler` daemon

Then manually (or add to rc.local for fully automatic):
```sh
sudo rc-service k3s start
```

---

## Memory Requirements

Official Funcom tiers (from `initial-setup.ps1`):

| RAM | Coverage |
|---|---|
| 10 GB | Absolute minimum — experimental swap required |
| 20 GB | Hagga Basin Sietch only |
| 30 GB | Hagga Basin + Story/Social maps |
| 40 GB | Hagga Basin + Story/Social + Deep Desert (full) |

Per-map Kubernetes limits (from `experimental_swap.sh`):

| Map | Limit | Request (swap mode) |
|---|---|---|
| `Survival_1` | 12 Gi | 5 Gi |
| `DeepDesert_1` | 10 Gi | 3 Gi |
| `Overmap`, all Story/Social maps | 1 Gi | 200 Mi |

Experimental swap lowers *requests* so Kubernetes will schedule pods even when free RAM is tight, using swap to back the gap between request and limit. Enable with:
```sh
~/dune-server/server/scripts/setup/experimental_swap.sh
```

---

## VPA (Vertical Pod Autoscaler)

VPA 1.6.0 runs in **recommender-only / Off mode**: it collects metrics and writes memory recommendations into VPA object status, but never mutates pod specs automatically. We use it to observe real usage and manually tune the request/limit splits in `experimental_swap.sh`.

### What VPA covers

VPA watches standard Kubernetes controllers (Deployments, StatefulSets). In the battlegroup namespace these are the infra workloads: postgres, rabbitmq, gateway, director, text-router, filebrowser, db-util-mon, db-util-pghero.

Funcom's game server pods (Survival_1, Overmap) are owned by the **ServerSet** custom resource — not a standard controller. VPA cannot target them via `scaleTargetRef`. Use `watch-gameservers.sh` instead.

### Deployed resources

All live in `kube-system`:
- `vpa-recommender` Deployment — 1 replica, 100Mi req / 256Mi limit
- ServiceAccount `vpa-recommender` with scoped ClusterRoles (read-only; no admission webhook, no updater)

### VPA objects

9 Off-mode VPA objects in `funcom-seabass-sh-db3533a2d5a25fb-xyyxbx`, one per Deployment/StatefulSet, named `vpa-<workload>`. Created by `vpa-objects.sh` (idempotent).

Recommendations appear after ~24 h of data collection and are visible under `.status.recommendation` in each VPA object.

### Reading recommendations

```sh
# Summary table — MEM column fills in after ~24h
sudo kubectl get vpa -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx

# Full recommendation for a specific workload
sudo kubectl describe vpa vpa-sh-db3533a2d5a25fb-xyyxbx-db-dbdepl-sts \
  -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx
```

### Monitoring game server memory

```sh
# One-shot check (Survival_1 and Overmap usage vs request/limit)
~/dune-server/scripts/vpa/watch-gameservers.sh --once

# Continuous (default 120s interval, logs RECOMMEND when usage > request + 20%)
~/dune-server/scripts/vpa/watch-gameservers.sh

# Tune interval or threshold
~/dune-server/scripts/vpa/watch-gameservers.sh --interval 300 --threshold 30
```

### Starting and stopping individual maps

Use `map-toggle.sh` — do not patch the ServerSet directly.

```sh
~/dune-server/scripts/map-toggle.sh list
~/dune-server/scripts/map-toggle.sh start DeepDesert_1
~/dune-server/scripts/map-toggle.sh stop  DeepDesert_1
```

**Why the script exists:** Starting a map requires patching two objects. The BattleGroup operator propagates `BattleGroup CR → ServerGroup → ServerSet` correctly, but the `ServerSetScale` (the final pod-creation trigger, owned by the ServerSet) does **not** auto-update. Without patching ServerSetScale, the ServerSet stays in `Stopped` phase indefinitely even though its `spec.replicas` is 1. `map-toggle.sh` patches both in one command.

The full chain: `BattleGroup CR sets[n].replicas` → `ServerGroup sets[n].replicas` → `ServerSet spec.replicas` → **`ServerSetScale spec.replicas`** → pod created.

### Adjusting game server memory

Tuning is done via `experimental_swap.sh`'s `map_to_requests` map or a direct BattleGroup CR patch:

```sh
# Re-run the script after editing map_to_requests in experimental_swap.sh
~/dune-server/server/scripts/setup/experimental_swap.sh

# Or patch directly (index from `kubectl get battlegroups ... -o json | jq ...`)
sudo kubectl patch battlegroup -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx \
  sh-db3533a2d5a25fb-xyyxbx --type='json' \
  -p='[{"op":"replace","path":"/spec/serverGroup/template/spec/sets/0/resources","value":{"limits":{"memory":"12Gi"},"requests":{"memory":"5Gi"}}}]'
```

### Re-installing or upgrading VPA

```sh
# Idempotent — safe to re-run against the live cluster
~/dune-server/scripts/vpa/install.sh
```

---

## SteamCMD / Updates

SteamCMD is at `~/steamcmd/steamcmd.sh`. The server (app 3104830) is installed at `~/dune-server/server/`.

```sh
# Update Linux server (depot 3104832)
~/dune-server/server/scripts/battlegroup.sh update

# Re-download Windows package for reference (depot 3104831)
~/steamcmd/steamcmd.sh +force_install_dir ./dune_server \
  +login anonymous +@sSteamCmdForcePlatformType windows \
  +app_update 3104830 validate +quit
# Output lands in ~/steamcmd/dune_server/
```

---

## Windows Package Reference

`~/steamcmd/dune_server/` — downloaded 2026-05-13.

| File | Purpose |
|---|---|
| `battlegroup.bat` | Entry point → `battlegroup.ps1` |
| `battlegroup-management/initial-setup.ps1` | Imports VHDX into Hyper-V, sets RAM, writes `settings.conf`, bootstraps via SSH |
| `battlegroup-management/battlegroup.ps1` | Management menu: status, start, stop, update, backup, import, open-director, open-file-browser, enable-experimental-swap |
| `battlegroup-management/vm-utilities.ps1` | SSH key rotation, password change helpers |
| `battlegroup-management/bootstrap/setup` | Shell script uploaded to `~/.dune/bin/setup` inside the VM; validates disk, runs SteamCMD if needed, calls `setup.sh` |
| `battlegroup-management/ssh/bundledSshKey` | Publicly known ed25519 key — used to bootstrap SSH before key rotation |
| `Virtual Hard Disks/dune-server.vhdx` | Pre-built Alpine Linux VM image |
