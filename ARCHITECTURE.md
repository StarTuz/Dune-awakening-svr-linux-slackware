# Architecture

This document describes the current architecture of the native Slackware Dune:
Awakening self-hosted deployment on `arrakis.algieba.org`.

`STATUS.md` remains the source of truth for current health and incidents. This
file describes the stable system shape and the control loops that make it work.

## Host Layout

- Host: `arrakis.algieba.org`
- OS: Slackware 15.0+
- LAN IP: `192.168.254.200`
- Public IP: `47.145.51.160`
- Co-tenant: Conan Exiles Enhanced server
- Dune user: `dune`
- Dune repository and server package: `/home/dune/dune-server`
- Backups: `/srv/backups/dune` and `/srv/backups/conan`

Funcom's supported self-host flow wraps a Linux VM in Windows Pro + Hyper-V.
This deployment runs the Linux side directly on Slackware: k3s, Funcom's
operators, Postgres, RabbitMQ, gateway, director, and game server pods all run
on the bare host.

## Network Path

External clients connect through the Frontier router to arrakis:

```text
Internet client
  -> 47.145.51.160:<Dune port>
  -> Frontier router port-forward
  -> 192.168.254.200:<same port>
  -> firewalld/iptables
  -> hostNetwork game server pod
```

LAN clients need a hairpin workaround because the Frontier router does not
reflect LAN traffic back to LAN through the public IP:

```text
LAN client
  -> local OUTPUT DNAT 47.145.51.160 -> 192.168.254.200
  -> arrakis firewalld/iptables
  -> hostNetwork game server pod
```

Dune game UDP ports are kept in `7782-7790`, above Conan's `7777-7778` range.
The active game port is assigned at pod startup and can change after restarts.

## Firewall Model

firewalld is configured with `FirewallBackend=iptables`. This is intentional:
the nft backend conflicts with k3s/flannel networking on this host.

Important zones:

- `public`: interface `eth0`, exposes SSH, Dune, RMQ, and Conan services.
- `trusted`: LAN, pod CIDR, service CIDR, `cni0`, and `flannel.1`.

Important custom firewalld services:

- `dune-game`: UDP `7782-7790`
- `dune-rmq`: TCP `31982` and `30196`
- `conan-exiles`: Conan UDP/TCP ports

The public zone must not expose Director, Filebrowser, Postgres, the k3s API, or
RabbitMQ admin ports. Check sensitive Kubernetes NodePorts against firewalld's
effective public services with:

```sh
~/dune-server/scripts/security-audit.sh
```

The host cannot verify Frontier router forwarding directly; router forwards
still need to be checked in the router UI.

Known failure mode: a stale nftables `table inet firewalld` can remain active
even while firewalld is using the iptables backend. In that state, iptables may
allow Dune UDP while the stale nft input hook still rejects packets with
`ICMP admin prohibited`.

Check:

```sh
grep -n '^FirewallBackend' /etc/firewalld/firewalld.conf
nft list tables
```

If `FirewallBackend=iptables`, `nft list tables` should not show
`table inet firewalld`.

## k3s Base

The deployment is a single-node k3s cluster. The node runs:

- kubelet/containerd
- flannel CNI
- CoreDNS
- local-path storage
- Traefik
- metrics-server
- VPA recommender in Off mode
- Funcom operators

The local kubectl/k3s client currently reports `v1.36.0+k3s1`. That is newer
than many examples and older self-hosting notes assume, so avoid over-fitting to
old Kubernetes limitations without checking the live cluster first. Newer k3s
may support resource fields, CRD behavior, server-side apply semantics, or
networking defaults that Funcom's older VM scripts did not rely on.

Slackware does not use systemd, so the Funcom/OpenRC assumptions are shimmed:

- `rc-service` translates to `/etc/rc.d/rc.<service>`
- `rc-update` is stubbed
- k3s startup is managed through rc scripts
- `memory-focused-scheduler` starts from `rc.local`

The k3s API must listen on the node address, not only `127.0.0.1`, because pods
reach it through the Kubernetes service endpoint. Do not set
`bind-address: 127.0.0.1` in k3s config.

## Funcom Operator Model

Funcom ships several Kubernetes operators as offline OCI images. They reconcile
custom resources into running infrastructure and game server pods.

```text
BattleGroup operator
  -> owns BattleGroup lifecycle
  -> creates/updates ServerGroup, database, message queues, gateway, director

Database operator
  -> owns Postgres StatefulSet and service

Server operator
  -> owns ServerGroup, ServerSet, ServerSetScale, and game server pods

Utilities operator
  -> owns utility services such as filebrowser
```

Each battlegroup gets its own namespace:

```text
funcom-seabass-<battlegroup-id>
```

The current namespace is:

```text
funcom-seabass-sh-db3533a2d5a25fb-xyyxbx
```

## Battlegroup Runtime Components

Inside the battlegroup namespace:

- Postgres stores persistent game/farm/world state.
- RabbitMQ game/admin queues carry server state and coordination messages.
- Gateway declares farm status to Funcom Live Services.
- Director manages battlegroup state, travel grants, capacity, and FLS updates.
- Text router handles text/chat-related service routing.
- Filebrowser provides a browser-accessible file utility.
- Game server pods run maps such as `Survival_1` and `Overmap`.

The gateway patch is currently required because the gateway discovers the AMQP
NodePort but not the RabbitMQ HTTP NodePort. The local patch adds:

```text
--RMQGameHttpPort=30196
```

Re-run after restarts or updates:

```sh
~/dune-server/scripts/gateway-patch.sh
```

## Map Lifecycle

Maps are not plain Kubernetes Deployments. They are controlled through a
multi-level custom-resource chain:

```text
BattleGroup
  spec.serverGroup.template.spec.sets[n].replicas
    -> ServerGroup
       spec.sets[n].replicas
         -> ServerSet
            spec.replicas
              -> ServerSetScale
                 spec.replicas
                   -> game server pod
```

The important subtlety is `ServerSetScale`: it is the final pod-creation
trigger and does not always follow higher-level replica patches automatically.

Use:

```sh
~/dune-server/scripts/map-toggle.sh start <MapName>
~/dune-server/scripts/map-toggle.sh stop  <MapName>
```

Do not directly patch `ServerSet` or `ServerGroup` replicas.

Bad split state:

```text
BattleGroup replicas=1
ServerSetScale=0
```

That leaves a map logically desired but physically absent. It can make the
battlegroup report a larger size than the actual farm and produce confusing S2S
farm-size or partition log noise.

Good states:

```text
off: BattleGroup replicas=0, ServerSetScale=0
on:  BattleGroup replicas=1, ServerSetScale=1, ServerSet READY=1
```

## Current Map Strategy

Normal low-footprint runtime:

- `Survival_1`: on
- `Overmap`: on
- `DeepDesert_1`: off
- social/story/CB/DLC maps: off unless needed

Deep Desert and social-zone travel should be tested by cleanly starting the
target map, verifying `ServerSet` and `ServerSetScale`, then watching current
ports and memory.

## FLS and Travel Flow

High-level flow:

```text
Gateway
  -> declares farm/RMQ endpoints to Funcom Live Services

Game server
  -> sends ready/server state through RMQ

Director
  -> receives server state
  -> declares battlegroup population/capacity to FLS
  -> issues travel grants to players

Client
  -> receives travel target IP/port
  -> connects by UDP to the game server pod's hostNetwork port
```

For the LAN client on `defiant`, the travel grant still uses the public IP, so
defiant rewrites outbound traffic locally:

```text
47.145.51.160 -> 192.168.254.200
```

If travel hangs, check the active target port from `BattleGroup.status.servers`,
then capture:

```sh
tcpdump -ni any 'host <client-ip> and (udp port <port> or icmp)'
```

No inbound packets means client/router/DNAT path. Inbound packets plus ICMP
`admin prohibited` means firewall. Two-way UDP means the packet path is open and
the next problem is game/auth/state.

## Persistent State

Postgres contains durable game and farm state. Relevant tables observed during
troubleshooting:

- `dune.farm_state`
- `dune.world_partition`
- `dune.player_travel_state`

Do not edit these directly unless there is a very specific, verified recovery
plan. Most problems so far have been Kubernetes desired-state or firewall
state, not DB corruption.

Backups use Funcom's database utility path first. The local wrapper
`scripts/dune-backup.sh` creates a `DatabaseOperation` dump through
`server/scripts/battlegroup.sh backup`, then copies the resulting database dump,
Kubernetes metadata, and UserSettings into `/srv/backups/dune/<battlegroup>/`.
Restore remains deliberately manual and is documented in `BACKUP-RESTORE.md`
because database import is destructive.

## Memory and Scheduling

The host currently has 16 GB RAM plus large swap headroom. Conan uses roughly
9.5 GB RSS, so Dune runs in a constrained but workable envelope.

The local `experimental_swap.sh` patch lowers Kubernetes memory requests so the
operators can schedule maps on this single node. Limits remain closer to
Funcom's intended ceilings.

Swap behavior should be interpreted against the live stack, not older k3s
assumptions. This host currently uses a modern k3s (`v1.36.0+k3s1` client),
Slackware's current kernel, cgroup v1 memory+memsw accounting, zram, and two
disk-backed swap devices. Older notes that state Kubernetes/k3s "does not
support swap" may not describe this deployment accurately. Verify with live
signals (`swapon --show`, cgroup memory settings, pod scheduling behavior, and
actual RSS/swap pressure) before carrying forward old workarounds.

Observed single-user map RSS:

- `Survival_1`: about 3.3 Gi
- `Overmap`: about 165 Mi
- `DeepDesert_1`: about 954 Mi when previously running

VPA watches ordinary Deployments and StatefulSets only. Game server pods are
owned by Funcom `ServerSet` custom resources, so use:

```sh
~/dune-server/scripts/vpa/watch-gameservers.sh --once
```

## Slackware Deviations From Funcom VM

The live deployment diverges from Funcom's expected VM in these ways:

- Slackware instead of Alpine.
- No Hyper-V wrapper.
- OpenRC commands shimmed to Slackware rc scripts.
- k3s runs directly on the host.
- firewalld is configured manually with iptables backend.
- SteamCMD update flow includes a local pre-fetch/validate step.
- Funcom script patches are maintained in `scripts/funcom-patches/`.
- Gateway deployment needs a local RMQ HTTP port patch after regeneration.
- LAN client needs local OUTPUT DNAT because the Frontier router lacks hairpin
  NAT.
- The local update wrapper adds safety around Funcom's update flow: backup,
  stop, double patch re-application, DB credential verification/repair, and
  gateway patch. This is intentionally more conservative than invoking
  `server/scripts/battlegroup.sh update` directly.
- Before invoking Funcom's update flow, the wrapper removes existing
  `~/.dune/bin/battlegroup` and `~/.dune/bin/bg-util` symlinks. Funcom's
  `setup/system.sh` recreates them with plain `ln -s`, which otherwise exits
  nonzero if the links already exist and prevents local post-update steps.
- Database credential checks discover the live Postgres port from the current
  DatabaseDeployment/status/service. This matters because the updated operator
  can listen on `5432` even when older local assumptions expected `15432`.

## Operational Invariants

- `STATUS.md` is the current-state source of truth.
- `FirewallBackend=iptables`.
- No `table inet firewalld` when firewalld backend is iptables.
- Dune UDP `7782-7790` allowed by firewalld and router.
- Gateway has `--RMQGameHttpPort=30196`.
- Maps are started/stopped only with `map-toggle.sh`.
- DeepDesert must be cleanly on or cleanly off.
- Do not use the removed S2S watchdog or Farm-session timing workaround.
