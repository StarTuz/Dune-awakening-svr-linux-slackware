# Dune: Awakening Slackware Self-Host

Operations repository for running a Dune: Awakening self-hosted battlegroup
natively on Slackware Linux, co-hosted with the existing Conan Exiles Enhanced
server on `arrakis.algieba.org`.

`STATUS.md` is the current source of truth. `ARCHITECTURE.md` explains the
system shape and control loops. `FILE-LOCATIONS.md` indexes important paths.
`CLAUDE.md` contains detailed operator notes for future agent sessions.

## Current State

- Host: `arrakis.algieba.org`
- LAN IP: `192.168.254.200`
- Public IP: `47.145.51.160`
- Battlegroup: `sh-db3533a2d5a25fb-xyyxbx` / `Slackware-Arrakis`
- Namespace: `funcom-seabass-sh-db3533a2d5a25fb-xyyxbx`
- Current maps: `Survival_1` and `Overmap` running
- `DeepDesert_1`: cleanly stopped unless explicitly started with `map-toggle.sh`
- Server browser: visible in the PTC/Experimental browser
- Hagga Basin travel: confirmed working after firewall cleanup

## Important Operational Notes

- Re-run `~/dune-server/scripts/gateway-patch.sh` after every battlegroup
  restart or update. The operator can regenerate the gateway deployment and
  lose `--RMQGameHttpPort=30196`.
- Start and stop maps only with `~/dune-server/scripts/map-toggle.sh`. Do not
  patch `ServerSet` or `ServerGroup` replicas directly.
- `DeepDesert_1` must be cleanly on or off. The bad split state is
  `BattleGroup replicas=1` with `ServerSetScale=0`.
- firewalld must use `FirewallBackend=iptables`. If travel packets are rejected
  despite correct firewalld services, check for stale nft state:

  ```sh
  nft list tables
  ```

  There should be no `table inet firewalld`. If it appears while the backend is
  iptables, remove the stale table and reload firewalld:

  ```sh
  nft delete table inet firewalld
  firewall-cmd --reload
  ```

## Common Commands

```sh
# Overall status
~/dune-server/server/scripts/battlegroup.sh status
sudo kubectl get battlegroup -A -o wide
sudo kubectl get serverset,serversetscale,serverstats -n funcom-seabass-sh-db3533a2d5a25fb-xyyxbx

# Restart/update
~/dune-server/server/scripts/battlegroup.sh restart
~/dune-server/scripts/gateway-patch.sh
~/dune-server/scripts/update.sh

# Maps
~/dune-server/scripts/map-toggle.sh list
~/dune-server/scripts/map-toggle.sh start DeepDesert_1
~/dune-server/scripts/map-toggle.sh stop DeepDesert_1

# Firewall sanity
grep -n '^FirewallBackend' /etc/firewalld/firewalld.conf
firewall-cmd --info-service=dune-game
nft list tables

# Memory
free -h
swapon --show
~/dune-server/scripts/vpa/watch-gameservers.sh --once
```

## Networking

The Dune game UDP range is `7782-7790`. Conan Exiles owns `7777-7778` and other
ports documented in `CLAUDE.md`, so Dune is kept above that range.

LAN clients behind the Frontier router need an OUTPUT DNAT rule because the
router does not provide NAT hairpin:

```sh
sudo firewall-cmd --permanent --direct --add-rule ipv4 nat OUTPUT 0 \
  -d 47.145.51.160 -j DNAT --to-destination 192.168.254.200
sudo firewall-cmd --reload
```

## Repository Layout

```text
server/                  Funcom server package and scripts
scripts/                 Local Slackware/operations scripts
scripts/funcom-patches/  Local patches re-applied after SteamCMD updates
dune-ctl/                Rust control/status tooling
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
