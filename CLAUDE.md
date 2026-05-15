# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Repository Is

Operations repository for a Dune: Awakening self-hosted battlegroup running natively on Slackware Linux, co-hosted with an existing Conan Exiles Enhanced server. `README.md` is the quick operations overview, `ARCHITECTURE.md` describes the stable system shape, `FILE-LOCATIONS.md` indexes important paths, and `STATUS.md` is the current authoritative state.

**Current state**: Fully running; security hardening applied 2026-05-14; Hagga Basin travel fixed 2026-05-15. Survival_1 and Overmap are running. DeepDesert_1 is cleanly stopped unless explicitly started with `map-toggle.sh`. Conan Exiles Enhanced co-tenant uses ~9.5 GB RSS. Total swap: 62 GB (zram + dune-vg SSD + sdc1) available as headroom. VPA recommender live (Off mode, memory only). Motherboard replacement to 64 GB pending. FLS token expires 2027-05-08 — rotate by 2027-04-08.

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
| `server-operator` | `ServerGroup` and `ServerSet` CRs |
| `utilities-operator` | Supporting services (filebrowser, etc.) |

Each battlegroup gets its own namespace `funcom-seabass-<name>`. Inside: postgres, rabbitmq, gateway, director, text-router, filebrowser, and the game server pods.

**Current battlegroup**: `sh-db3533a2d5a25fb-xyyxbx` ("Slackware-Arrakis")
**Namespace**: `funcom-seabass-sh-db3533a2d5a25fb-xyyxbx`

### Server operator chain (critical — read before touching maps)

Starting or stopping a map involves a four-level ownership chain. Each level is reconciled by the server-operator:

```
BattleGroup CR
  spec.serverGroup.template.spec.sets[n].replicas
    ↓ battlegroup-operator propagates
  ServerGroup
    spec.sets[n].replicas
      ↓ server-operator propagates
      ServerSet
        spec.replicas
          ↓ serverset controller creates/owns
          ServerSetScale        ← final pod-creation trigger
            spec.replicas
              ↓
              Pod
```

**The catch**: the `ServerSetScale` does **not** auto-update when the levels above it are patched. Patching only the BattleGroup CR (or ServerSet) leaves the ServerSet in `Stopped` phase indefinitely — `spec.replicas=1` is set but `ServerSetScale.spec.replicas` stays 0 and no pod is created. Always use `map-toggle.sh`, which patches both the BattleGroup CR and the ServerSetScale in one step.

### Windows vs our deployment

Funcom's Windows depot (3104831) ships a pre-built Hyper-V VM (`.vhdx` + `.vmcx`) running Alpine Linux with k3s inside. `battlegroup.bat` → PowerShell UI → SSH into VM → same Funcom scripts. Our deployment skips the Hyper-V layer and runs k3s directly on Slackware.

The live k3s/kubectl client is currently `v1.36.0+k3s1`, which is likely newer
than the versions assumed by some older examples or Funcom VM-era notes. Before
working around an apparent Kubernetes limitation, check the live API behavior;
newer k3s may already support cleaner approaches.

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
| `root-setup.sh` | Run once as root: installs k3s, creates shims (incl. steamcmd wrapper), writes rc.k3s, sets sudoers, sets up LVM swap + backup volume |
| `memory-focused-scheduler.sh` | Custom Kubernetes scheduler daemon — binds pending pods to the single k3s node. Auto-starts via rc.local |
| `map-toggle.sh` | Start/stop individual maps; handles the full BattleGroup CR + ServerSetScale chain |
| `update.sh` | Full update flow: steamcmd pre-fetch with `validate`, re-apply funcom patches, run Funcom update, re-apply gateway patch |
| `gateway-patch.sh` | Apply `--RMQGameHttpPort=30196` to gateway Deployment (idempotent; re-run after every restart) |
| `security-audit.sh` | Check for accidental public exposure of sensitive services and NodePorts |
| `funcom-patches.sh` | Re-apply Slackware patches to Funcom-shipped scripts after SteamCMD overwrites (uses baselines in `funcom-patches/`) |
| `funcom-patches/` | Patched copies of Funcom scripts + `.upstream` baselines for drift detection |
| `port-preempt.py` | Hold UDP 7779-7781 to prevent Dune game servers from binding ports owned by Path of Titans on the router |
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

**`world.sh`** — added "Europe Test" / "North America Test" to the region list. Note: SteamCMD updates overwrite this file; if Funcom hasn't merged these regions upstream by then, our additions will be lost on next update. Not currently managed by `funcom-patches/` because `world.sh` is only used during initial world creation.

**`experimental_swap.sh`** — patched for Slackware (durable via `funcom-patches/`):
- Skips swapfile creation if swap is already active (we have ~62 GB via zram + dune-vg + sdc1)
- Replaces Alpine cgroup path (`/sys/fs/cgroup/openrc.k3s/memory.swap.max`) with a dynamic lookup of the k3s process cgroup using `/proc/<pid>/cgroup` and the cgroup v1 `memory.memsw.limit_in_bytes` interface
- Idempotency guards on `sudo tee` / `sudo cp` for k3s config (so re-running works under our sudoers whitelist)

The patched file lives at `scripts/funcom-patches/experimental_swap.sh` and is re-applied by `scripts/funcom-patches.sh` after every SteamCMD update (wired into `update.sh`). The driver detects upstream drift via a stored baseline (`*.upstream` sidecar) — if Funcom changes the script underneath us, the driver warns instead of silently clobbering.

**`operator.sh`** — `kubectl replace` in `replace_custom_resources` fails on a fresh cluster where CRDs do not yet exist. On existing clusters (our normal update path) it works fine. **Fresh-install workaround**: before running `setup.sh`, manually apply the CRDs once with `sudo kubectl apply --server-side -f ~/dune-server/server/images/operators/crds/`. Not patched via `funcom-patches/` because it only matters during one-time bootstrap.

**`~/.dune/bin/battlegroup`** symlink — `system.sh` creates this but was never run during our manual bootstrap. Created manually: `ln -s ~/dune-server/server/scripts/battlegroup.sh ~/.dune/bin/battlegroup`.

**Bootstrapping fixes applied during initial setup** (documented in `STATUS.md`):
- cert-manager v1.8.0 installed via official manifest (not in download package)
- ServiceMonitor CRD installed (required by database operator)
- Operator deployments created from scratch (namespace, SA, CRB, Deployments)
- Webhook TLS: self-signed cert mounted into all 4 operator pods

---

## Security

Hardening applied 2026-05-14. Read this section before touching the firewall, SSH, or k3s networking.

### firewalld

firewalld 1.3.3 is installed and starts from `/etc/rc.d/rc.local`. **Must use `FirewallBackend=iptables`** — the nftables backend conflicts with k3s CNI (flannel) and corrupts pod networking. Set in `/etc/firewalld/firewalld.conf`.

Two zones:
- **`public`** (eth0): ssh, dune-game (UDP 7782-7790), dune-rmq (TCP 31982+30196), conan-exiles (UDP 7777-7778/14001/27015, TCP 25575/8088). Masquerade on.
- **`trusted`** (target ACCEPT): sources 127.0.0.1/8, 192.168.254.0/24 (LAN), 10.42.0.0/16 (pod CIDR), 10.43.0.0/16 (service CIDR); interfaces cni0, flannel.1.

Custom service XMLs live in `/etc/firewalld/services/`. **Zone XML files must begin with `<?xml` as the very first byte** — leading whitespace causes `INVALID_SERVICE: XML or text declaration not at start of entity`. Verify with `head -c1 /etc/firewalld/zones/public.xml | xxd -p` (must output `3c`).

After editing XML files, run `sudo firewall-cmd --reload` and verify the generated iptables rules. If firewalld reports XML parsing errors or stale state persists, do a full stop+start: `sudo /etc/rc.d/rc.firewalld stop && sudo /etc/rc.d/rc.firewalld start`.

Run `~/dune-server/scripts/security-audit.sh` when you want a quick host-side exposure check. It flags accidental public exposure of Director, Filebrowser, Postgres, the k3s API, and RabbitMQ admin ports. It also treats the intentionally public `mq-game-svc` ports (`31982` and `30196`) as expected.

### stale nft firewalld table failure mode

Hagga Basin travel timed out on 2026-05-15 because a stale nftables `table inet firewalld` remained active while firewalld was configured for the iptables backend. The iptables firewalld rules correctly allowed Dune UDP `7782-7790`, but the stale nft firewalld input hook rejected client packets with `ICMP admin prohibited`.

Diagnosis:

```sh
tcpdump -ni any 'host 192.168.254.17 and (udp port 7783 or icmp)'
nft list tables
```

If `tcpdump` shows `ICMP ... admin prohibited` and `nft list tables` includes `table inet firewalld` while `FirewallBackend=iptables`, remove the stale nft table:

```sh
nft delete table inet firewalld
firewall-cmd --reload
```

After the fix, `nft list tables` should not show `table inet firewalld`, and `tcpdump` should show two-way UDP between the client and the active Survival_1 port.

### SSH

Key-only authentication. `/etc/ssh/sshd_config`:
```
PasswordAuthentication no
KbdInteractiveAuthentication no
```

Only `startux` and `dune` have authorized keys. Keys are RSA-4096 (defiant's OpenSSH is too old for ed25519 — `unsupported` error from libcrypto).

### k3s API security — do NOT use bind-address

**Do not add `bind-address: 127.0.0.1` to `/etc/rancher/k3s/config.yaml`.** The Kubernetes `kubernetes` service has a ClusterIP (10.43.0.1) with an Endpoint pointing to the node IP (192.168.254.200:6443). kube-proxy DNATs pod→API traffic to that endpoint. If the API server only listens on 127.0.0.1, nothing answers on 192.168.254.200:6443 and every operator crashes with `connection refused`. The firewall (trusted zone) is sufficient — external API access is blocked without needing bind-address.

### Update flow notes

- `scripts/db-credentials.sh` discovers the live Postgres port from the DatabaseDeployment or service before checking credentials. The old `15432` assumption no longer matches the current operator revision.
- `scripts/update.sh --post-update-only --start-after` is the resume path after a Funcom update has already completed. It now starts the battlegroup before reapplying the gateway patch, because the gateway deployment is recreated when the battlegroup comes back.

### FLS JWT token

The FLS JWT is in each BattleGroup CR set's `arguments` array, in the form:
```
-ini:engine:[FuncomLiveServices]:ServiceAuthToken=<jwt>
```
It appears 28 times (once per map set). **Current token expires 2027-05-08. Rotate by 2027-04-08.**

Check expiry at any time: `~/dune-server/dune-ctl/target/release/dune-ctl token-check`

When rotating: get a new token from the Funcom portal, patch all 28 occurrences in the BattleGroup CR, then run `gateway-patch.sh`. Tracked in dune-ctl (`token-check` exits 2 when ≤14 days remain).

### Operator recovery after stuck state

If battlegroup gets stuck in `Stopped` after a restart:

1. Check `MessageQueue` CRs: `sudo kubectl get messagequeues -n funcom-seabass-<bg>`. If any show `spec.suspend: True`, patch them false: `sudo kubectl patch messagequeue <name> -n <ns> --type=merge -p '{"spec":{"suspend":false}}'`
2. If operators show `Error` status: `sudo kubectl rollout restart deployment -n funcom-operators` — let them stabilize (1-2 min) before checking battlegroup status.
3. **Do not manually scale StatefulSets** owned by MessageQueue CRs. Manual scaling bypasses the operator's lifecycle state machine and leaves `status.phase` and `status.managementAddress` stuck, causing the battlegroup to remain Stopped even after pods appear.
4. After operators recover, the battlegroup reconciles automatically. Then run `gateway-patch.sh`.

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
| Map toggle | `~/dune-server/scripts/map-toggle.sh` |
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

# Individual map control
~/dune-server/scripts/map-toggle.sh list                        # all maps + live phases
~/dune-server/scripts/map-toggle.sh start DeepDesert_1
~/dune-server/scripts/map-toggle.sh stop  DeepDesert_1

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
1. firewalld
2. QEMU guest agent
3. `memory-focused-scheduler` daemon

Then manually (or add to rc.local for fully automatic):
```sh
sudo rc-service k3s start
```

After k3s is up, maps that were running before the reboot do **not** restart automatically — the BattleGroup CR retains the last replica counts, but the ServerSetScale objects reset to 0. Use `map-toggle.sh start <map>` for each map, or restart the whole battlegroup:
```sh
~/dune-server/server/scripts/battlegroup.sh restart
```

---

## Map Inventory

All 28 maps defined in the BattleGroup CR. Observed RSS values from 2026-05-13 (single user, idle/light load):

| Map | Limit | Request | Observed RSS | Notes |
|---|---|---|---|---|
| `Survival_1` | 12 Gi | 5 Gi | ~3.3 Gi | Main world — always on |
| `DeepDesert_1` | 10 Gi | 3 Gi | ~954 Mi | Stopped by default; start only with `map-toggle.sh` |
| `Overmap` | 1 Gi | 200 Mi | ~165 Mi | Running; swap-backed by request |
| `SH_Arrakeen` | 1 Gi | 200 Mi | — | Stopped |
| `SH_HarkoVillage` | 1 Gi | 200 Mi | — | Stopped |
| Story / CB / DLC maps (23 others) | 1–6 Gi | 200 Mi | — | All stopped |

`map-toggle.sh list` shows current on/off state. To start any stopped map:
```sh
~/dune-server/scripts/map-toggle.sh start <MapName>
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

Funcom's tiers assume full player load. In practice with a single user:
- Survival_1 + Overmap are the normal low-footprint set. Survival_1 + DeepDesert_1 + Overmap together previously used ~4.4 Gi RSS with one user, but DeepDesert must be cleanly started/stopped through `map-toggle.sh`.
- Experimental swap lowers *requests* so Kubernetes schedules pods against available RAM + swap headroom. The gap between request and actual RSS is wide for all maps so far.
- Do not rely on older blanket claims that k3s/Kubernetes cannot use swap. This host is running a modern k3s client (`v1.36.0+k3s1`) on Slackware with cgroup v1 memory+memsw accounting, zram, and disk-backed swap. Judge swap behavior from live evidence: `swapon --show`, cgroup settings, scheduling behavior, RSS, and actual swap pressure.

Per-map Kubernetes limits and requests (from `experimental_swap.sh`):

| Map | Limit | Request (swap mode) |
|---|---|---|
| `Survival_1` | 12 Gi | 5 Gi |
| `DeepDesert_1` | 10 Gi | 3 Gi |
| `Overmap`, all Story/Social/CB/DLC maps | 1 Gi | 200 Mi |

Enable or re-run experimental swap with:
```sh
~/dune-server/server/scripts/setup/experimental_swap.sh
```

---

## VPA (Vertical Pod Autoscaler)

VPA 1.6.0 runs in **recommender-only / Off mode**: it collects metrics and writes memory recommendations into VPA object status, but never mutates pod specs automatically. We use it to observe real usage and manually tune the request/limit splits in `experimental_swap.sh`.

### What VPA covers

VPA watches standard Kubernetes controllers (Deployments, StatefulSets). In the battlegroup namespace these are the infra workloads: postgres, rabbitmq, gateway, director, text-router, filebrowser, db-util-mon, db-util-pghero.

Funcom's game server pods are owned by the **ServerSet** custom resource — not a standard controller. VPA cannot target them via `scaleTargetRef`. Use `watch-gameservers.sh` instead.

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
# One-shot check (all game server pods — usage vs request/limit)
~/dune-server/scripts/vpa/watch-gameservers.sh --once

# Continuous (default 120s interval, logs RECOMMEND when usage > request + 20%)
~/dune-server/scripts/vpa/watch-gameservers.sh

# Tune interval or threshold
~/dune-server/scripts/vpa/watch-gameservers.sh --interval 300 --threshold 30
```

### Adjusting game server memory

Tuning is done via `experimental_swap.sh`'s `map_to_requests` map or a direct BattleGroup CR patch:

```sh
# Re-run the script after editing map_to_requests in experimental_swap.sh
~/dune-server/server/scripts/setup/experimental_swap.sh

# Or patch directly — get the set index first:
sudo kubectl get battlegroups sh-db3533a2d5a25fb-xyyxbx \
  -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx -o json \
  | jq -r '.spec.serverGroup.template.spec.sets | to_entries[]
           | "\(.key): \(.value.map) (replicas=\(.value.replicas))"'

# Then patch by index (example: index 0 = Survival_1)
sudo kubectl patch battlegroup sh-db3533a2d5a25fb-xyyxbx \
  -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx --type='json' \
  -p='[{"op":"replace","path":"/spec/serverGroup/template/spec/sets/0/resources",
        "value":{"limits":{"memory":"12Gi"},"requests":{"memory":"5Gi"}}}]'
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
