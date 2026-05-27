# Installer Design

This document captures the recommended design for a future robust installer for
this Dune: Awakening self-hosting stack. It is intentionally a design note, not
an implementation plan.

The goal is a simple operator experience across Slackware-style SysV/rc hosts
and systemd distributions such as Ubuntu, while keeping enough manual override
points for Linux's real-world variation.

## Goals

- Install the same repository and control stack on Slackware, Ubuntu, Debian,
  and similar Linux hosts.
- Support both systemd and non-systemd service management.
- Prefer clear, inspectable shell behavior over opaque bootstrap tooling.
- Detect OS/init defaults automatically, but allow explicit overrides.
- Produce a dry-run plan before changing the host.
- Be idempotent: rerunning the installer should converge or explain exactly why
  it cannot.
- Keep day-2 operations in `dune-ctl`; keep first-install bootstrapping in a
  small installer layer.

## Non-Goals

- No attempt to hide Kubernetes, k3s, SteamCMD, or Funcom package mechanics from
  advanced operators.
- No Docker Compose replacement for Funcom's k3s/operator model.
- No Ansible/Puppet/Chef dependency for the initial bootstrap.
- No Rust-only first bootstrapper that cannot run before toolchains or binaries
  are present.
- No destructive host changes without an explicit `--force` or equivalent.

## Recommended Shape

Use a two-layer installer:

1. A small POSIX `sh` frontend, for example `install.sh`.
2. Provider modules for OS families and init systems.

The intended operator flow:

```sh
sudo ./install.sh --plan
sudo ./install.sh --install
```

Manual overrides must be first-class:

```sh
sudo ./install.sh --install --os slackware --init slackware-rc
sudo ./install.sh --install --os ubuntu --init systemd
sudo ./install.sh --install --init sysv
sudo ./install.sh --install --prefix /opt/dune-server
```

The installer should print the resolved plan before taking action:

```text
Install plan
  os              ubuntu 24.04
  init            systemd
  install root    /opt/dune-server
  config root     /etc/opt/dune-server
  state root      /var/opt/dune-server
  service         dune-server.service
  k3s mode        install or reuse existing
  package root    /var/opt/dune-server/packages/live/app-4754530/server
```

## Detection Model

Detection should be conservative and explainable. Use it to choose defaults,
not to lock the operator into a path.

Suggested detection order:

1. Command-line override: `--os`, `--init`, `--prefix`.
2. Environment override: `DUNE_INSTALL_OS`, `DUNE_INSTALL_INIT`.
3. `/etc/os-release` values: `ID`, `ID_LIKE`, `VERSION_ID`.
4. Init detection:
   - systemd: `/run/systemd/system` exists and `systemctl` is executable.
   - Slackware rc: `/etc/slackware-version` or `/etc/rc.d` conventions.
   - SysV: `/etc/init.d` plus `service`, `update-rc.d`, `chkconfig`, or
     distro-specific equivalent.
5. Fallback to "unknown" with a required manual override.

The installer should always show both the detected value and the reason:

```text
init=systemd detected because /run/systemd/system exists
os=ubuntu detected from /etc/os-release ID=ubuntu
```

## Provider Layout

A provider layout keeps distro-specific behavior out of the main installer:

```text
installer/
  install.sh
  lib/
    detect.sh
    plan.sh
    fs.sh
    log.sh
    state.sh
  providers/
    init-systemd.sh
    init-sysv.sh
    init-slackware-rc.sh
    os-ubuntu.sh
    os-debian.sh
    os-slackware.sh
```

Providers should expose small common functions:

```sh
provider_preflight
provider_install_dependencies
provider_install_service
provider_enable_service
provider_start_service
provider_status
```

The main installer should call provider functions through a narrow interface and
avoid embedding distro case statements throughout the code.

## Filesystem Layout

Use an FHS-style layout for future general-purpose installs:

| Purpose | Preferred path |
|---|---|
| Static application/repo files | `/opt/dune-server` |
| Host-specific config | `/etc/opt/dune-server` |
| Runtime state and capsules | `/var/opt/dune-server` |
| Logs | `/var/log/dune-server` |
| Backups | `/var/backups/dune-server` |
| k3s/containerd data | k3s defaults, usually `/var/lib/rancher/k3s` |

This differs from the current Arrakis development layout under
`/home/dune/dune-server`. The installer can support that layout for local
development, but packaged installs should not require a user's home directory.

The Filesystem Hierarchy Standard defines `/opt` for add-on application
software, `/etc/opt` for host-specific configuration for `/opt` packages, and
`/var/opt` for variable package data.

References:

- https://specifications.freedesktop.org/fhs/latest/opt.html
- https://specifications.freedesktop.org/fhs/latest-single

## Service Management

Install native service definitions for the host init system.

### systemd

Install native units instead of relying on SysV compatibility:

```text
/etc/systemd/system/dune-server.service
/etc/systemd/system/dune-scheduler.service
```

Use `systemctl daemon-reload`, `enable`, `start`, and `status`.

The systemd unit load path and unit semantics are documented in:

- https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html
- https://www.freedesktop.org/software/systemd/man/253/systemd.service.html

### SysV

Install an init script with normal LSB-style actions:

```text
/etc/init.d/dune-server
```

Support:

```sh
service dune-server start
service dune-server stop
service dune-server restart
service dune-server status
```

The LSB init-script model expects standard actions such as `start`, `stop`,
`restart`, `try-restart`, `reload`, `force-reload`, and `status`.

Reference:

- https://refspecs.linuxfoundation.org/LSB_2.0.1/LSB-Core/LSB-Core.html

### Slackware rc

Slackware should get native rc scripts:

```text
/etc/rc.d/rc.dune-server
/etc/rc.d/rc.k3s
/etc/rc.d/rc.memory-focused-scheduler
```

The installer should make scripts executable, add documented `rc.local` hooks
only when needed, and avoid pretending Slackware is Debian SysV.

## Install Phases

Keep phases explicit. Each phase should be individually loggable and, where
reasonable, individually resumable.

1. `detect`
   - Identify OS, init system, architecture, memory, disk, network tools.
2. `preflight`
   - Check root privileges, required commands, ports, kernel features, cgroups,
     firewall backend, and existing k3s state.
3. `install-deps`
   - Install or verify `curl`, `jq`, `tar`, `sudo`, `iptables`, `steamcmd`
     prerequisites, and distro-specific packages.
4. `install-files`
   - Install repo/static files into the install root.
5. `configure`
   - Write config, directories, users/groups, permissions.
6. `install-service`
   - Install native systemd/SysV/Slackware service definitions.
7. `bootstrap-k3s`
   - Install or reuse k3s, configure kubeconfig, verify node health.
8. `package`
   - Install/validate Funcom package through SteamCMD.
9. `images`
   - Import package image tarballs into k3s/containerd and verify expected tags.
10. `capsule`
   - Create or activate a world capsule.
11. `verify`
   - Run `dune-ctl preflight`, status checks, gateway patch checks, and image
     inventory checks.

## Safety and State

The installer should maintain a machine-readable state file:

```text
/var/opt/dune-server/install-state.json
```

Record:

- installer version
- selected OS/init provider
- install paths
- package app id/build/image tags
- k3s version
- created services
- active battlegroup/capsule id
- last successful phase

Safety defaults:

- `--plan` performs no writes.
- `--install` writes only after printing a plan.
- `--force` is required for overwrites.
- `--uninstall` should be conservative and should never delete backups,
  capsules, or databases unless an explicit destructive flag is provided.
- Every run writes a log under `/var/log/dune-server/install-YYYYmmdd-HHMMSS.log`.

## Dependency Strategy

Start minimal:

- POSIX `sh`
- coreutils-like tools
- `awk`, `sed`, `grep`
- `tar`
- `curl` or package manager equivalent
- `jq`

Avoid requiring Python, Rust, Ansible, or Docker for the initial bootstrap.
After installation, `dune-ctl` should be the primary operator tool.

## Packaging Strategy

The first version should be a portable installer script. Later wrappers can
package the same files:

- `.deb` for Debian/Ubuntu
- `.rpm` for RHEL-like systems
- Slackware package or SlackBuild
- tarball release for manual installs

Debian policy explicitly allows packages to include a systemd service unit and
optionally an init script for non-systemd systems, which matches this provider
model.

Reference:

- https://www.debian.org/doc/debian-policy/ch-opersys.html

## Verification Commands

The installer should end by printing commands the operator can rerun:

```sh
dune-ctl status
dune-ctl preflight
dune-ctl capsules inventory
dune-ctl capsules images verify --env live
sudo kubectl get battlegroups -A -o wide
sudo ctr -n k8s.io images ls -q | sort
```

For systemd hosts:

```sh
systemctl status dune-server
journalctl -u dune-server -n 200 --no-pager
```

For Slackware hosts:

```sh
/etc/rc.d/rc.dune-server status
tail -n 200 /var/log/dune-server/*.log
```

## Recommended First Implementation Scope

Keep the first real implementation narrow:

1. `--plan`
2. `--install`
3. `--os slackware|ubuntu|debian|auto`
4. `--init slackware-rc|systemd|sysv|auto`
5. Slackware and Ubuntu providers
6. k3s install/reuse detection
7. native service installation
8. package image import and verification
9. final `dune-ctl preflight`

Do not attempt multi-world or multi-Sietch orchestration in the installer. That
belongs in `dune-ctl` once the base host is installed and healthy.

## External Ubuntu Reference

When implementing the Ubuntu provider, compare against adainrivers'
manual Ubuntu setup guide:

```text
https://github.com/adainrivers/dune-dedicated-server-manager/blob/main/docs/ubuntu-manual-setup-guide.md
```

Useful areas to cross-check:

- Fresh Ubuntu package prerequisites and SteamCMD setup.
- k3s installation through systemd.
- kubelet swap configuration before k3s install.
- Official Live Steam app `4754530` package handling.
- Fresh-cluster import of bundled images, cert-manager, Funcom CRDs,
  operators, webhook secrets, RBAC, and node labels.
- Ubuntu replacements for Funcom's Alpine/OpenRC assumptions.
- Generated `HOST_DATACENTER_IP_ADDRESS` repair.
- Removal of custom `schedulerName` fields for default-scheduler Ubuntu hosts.
- Database credential alignment after world creation.

Treat it as a bootstrap reference, not a replacement for this repository's
day-2 operations model. The installer should still preserve our stricter
backup/restore environment boundaries, capsule separation, public-IP checks,
gateway/RMQ HTTP verification, map lifecycle guardrails, and security audit
posture.
