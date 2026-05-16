# File Locations

Common paths for the native Slackware Dune: Awakening deployment.

## Repository and Server Package

| Item | Path |
|---|---|
| Operations repository | `/home/dune/dune-server` |
| Funcom server package / `DOWNLOAD_PATH` | `/home/dune/dune-server/server` |
| Download symlink | `/home/dune/.dune/download -> /home/dune/dune-server/server` |
| Windows package reference | `/home/dune/steamcmd/dune_server` |
| SteamCMD | `/home/dune/steamcmd/steamcmd.sh` |
| Rust control tool | `/home/dune/dune-server/dune-ctl` |
| TUI mascot design note | `/home/dune/dune-server/dune-ctl/TUI-MASCOT.md` |

## Main Documentation

| Item | Path |
|---|---|
| Quick overview | `/home/dune/dune-server/README.md` |
| Current operational state | `/home/dune/dune-server/STATUS.md` |
| Architecture | `/home/dune/dune-server/ARCHITECTURE.md` |
| File index | `/home/dune/dune-server/FILE-LOCATIONS.md` |
| Agent/operator guidance | `/home/dune/dune-server/CLAUDE.md` |

## Funcom Scripts

| Item | Path |
|---|---|
| Funcom scripts root | `/home/dune/dune-server/server/scripts` |
| Battlegroup management | `/home/dune/dune-server/server/scripts/battlegroup.sh` |
| First-time setup | `/home/dune/dune-server/server/scripts/setup.sh` |
| k3s setup | `/home/dune/dune-server/server/scripts/setup/k3s.sh` |
| World creation | `/home/dune/dune-server/server/scripts/setup/world.sh` |
| Operator setup | `/home/dune/dune-server/server/scripts/setup/operator.sh` |
| Swap/request patch script | `/home/dune/dune-server/server/scripts/setup/experimental_swap.sh` |
| Config directory | `/home/dune/dune-server/server/scripts/setup/config` |
| Engine config | `/home/dune/dune-server/server/scripts/setup/config/UserEngine.ini` |
| Game config | `/home/dune/dune-server/server/scripts/setup/config/UserGame.ini` |
| Battlegroup symlink | `/home/dune/.dune/bin/battlegroup` |

## Local Operations Scripts

| Item | Path |
|---|---|
| Local scripts root | `/home/dune/dune-server/scripts` |
| Root bootstrap | `/home/dune/dune-server/scripts/root-setup.sh` |
| Update pipeline | `/home/dune/dune-server/scripts/update.sh` |
| DB credential guard | `/home/dune/dune-server/scripts/db-credentials.sh` |
| Gateway patch | `/home/dune/dune-server/scripts/gateway-patch.sh` |
| Security exposure audit | `/home/dune/dune-server/scripts/security-audit.sh` |
| Live system snapshot | `/home/dune/dune-server/scripts/system-snapshot.sh` |
| Resource snapshot | `/home/dune/dune-server/scripts/resource-snapshot.sh` |
| Map toggle | `/home/dune/dune-server/scripts/map-toggle.sh` |
| Funcom patch driver | `/home/dune/dune-server/scripts/funcom-patches.sh` |
| Funcom patch baselines | `/home/dune/dune-server/scripts/funcom-patches/*.upstream` |
| Scheduler daemon | `/home/dune/dune-server/scripts/memory-focused-scheduler.sh` |
| Port preemption helper | `/home/dune/dune-server/scripts/port-preempt.py` |
| VPA scripts | `/home/dune/dune-server/scripts/vpa` |
| Game server memory watcher | `/home/dune/dune-server/scripts/vpa/watch-gameservers.sh` |

## Dune Runtime State

| Item | Path |
|---|---|
| Dune home config | `/home/dune/.dune` |
| World config YAML | `/home/dune/.dune/sh-db3533a2d5a25fb-xyyxbx.yaml` |
| FLS secret YAML | `/home/dune/.dune/sh-db3533a2d5a25fb-xyyxbx-fls-secret.yaml` |
| RMQ secret YAML | `/home/dune/.dune/sh-db3533a2d5a25fb-xyyxbx-rmq-secret.yaml` |
| Optional Windows-style settings file | `/home/dune/.dune/settings.conf` |

The YAML files under `/home/dune/.dune` contain secrets and should remain mode
`600`.

## Kubernetes and k3s

| Item | Path |
|---|---|
| k3s config | `/etc/rancher/k3s/config.yaml` |
| k3s rc script | `/etc/rc.d/rc.k3s` |
| k3s log | `/home/dune/dune-server/logs/k3s.log` |
| Scheduler log | `/home/dune/dune-server/logs/memory-focused-scheduler.log` |
| k3s kubeconfig | `/etc/rancher/k3s/k3s.yaml` |
| Local path storage | `/var/lib/rancher/k3s/storage` |
| containerd/k3s data | `/var/lib/rancher/k3s` |

## Game Server Logs

Game server logs live inside each game server pod at:

```text
/home/dune/server/DuneSandbox/Saved/Logs/DuneSandbox_PIDX-<partition>.log
```

Examples:

| Map | Typical Pod | Log |
|---|---|---|
| Survival_1 | `sg-survival-1-pod-1` | `/home/dune/server/DuneSandbox/Saved/Logs/DuneSandbox_PIDX-1.log` |
| Overmap | `sg-overmap-pod-2` | `/home/dune/server/DuneSandbox/Saved/Logs/DuneSandbox_PIDX-2.log` |
| DeepDesert_1 | `sg-deepdesert-1-pod-8` | `/home/dune/server/DuneSandbox/Saved/Logs/DuneSandbox_PIDX-8.log` |

Use `kubectl exec` or Funcom's `logs-export` command to access them.

## firewalld and Networking

| Item | Path |
|---|---|
| firewalld config | `/etc/firewalld/firewalld.conf` |
| firewalld zones | `/etc/firewalld/zones` |
| Public zone XML | `/etc/firewalld/zones/public.xml` |
| Trusted zone XML | `/etc/firewalld/zones/trusted.xml` |
| firewalld services | `/etc/firewalld/services` |
| Dune game service | `/etc/firewalld/services/dune-game.xml` |
| Dune RMQ service | `/etc/firewalld/services/dune-rmq.xml` |
| Conan service | `/etc/firewalld/services/conan-exiles.xml` |
| rc.local boot script | `/etc/rc.d/rc.local` |

`/etc/firewalld/firewalld.conf` should contain:

```text
FirewallBackend=iptables
```

With that backend, `nft list tables` should not show `table inet firewalld`.

## Backups and Storage

| Item | Path |
|---|---|
| Backup mount | `/srv/backups` |
| Dune backups | `/srv/backups/dune` |
| System snapshot reports | `/srv/backups/dune/system-snapshots` |
| Resource snapshot reports | `/srv/backups/dune/resource-snapshots` |
| Root btrfs snapshots | `/.snapshots` |
| Backup btrfs snapshots | `/srv/backups/.snapshots` |
| Conan backups | `/srv/backups/conan` |
| Dune backup wrapper | `scripts/dune-backup.sh` |
| Backup/restore runbook | `BACKUP-RESTORE.md` |
| Funcom DB dump staging | `/funcom/artifacts/database-dumps/<battlegroup>` |
| btrfs root | `/` on `/dev/sdc2` |
| SSD LVM VG | `dune-vg` on `/dev/sdb2` |

## Conan Co-Tenant

| Item | Path |
|---|---|
| Conan home | `/home/conan` |
| Conan enhanced server | `/home/conan/conan-enhanced-server` |
| Conan server binary | `/home/conan/conan-enhanced-server/server/ConanSandbox/Binaries/Linux/ConanSandboxServer-Linux-Shipping` |

Do not modify Conan files as part of Dune work unless explicitly requested.
