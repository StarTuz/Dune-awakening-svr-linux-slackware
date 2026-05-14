# Dune: Awakening Native Slackware Self-Host Notes

Research and handoff notes for running the Dune: Awakening self-hosted server
fully natively on Slackware Linux.

No server code has been installed here yet. These notes capture the state of the
host and the intended direction before continuing work as the future `dune`
user.

## Goal

Run one Dune: Awakening battlegroup natively on Slackware, co-hosted with the
existing Conan Exiles Enhanced server.

This machine will not use Windows or Hyper-V. Funcom's first supported
self-hosted path is Windows Pro plus Hyper-V orchestration, but their public
statements and Steam metadata indicate Linux server payloads exist. The work
here is to unwrap/adapt the Linux side for Slackware once the self-hosted server
package is available.

## Current Hardware And OS

- Hostname: `arrakis.algieba.org`
- OS: `Slackware 15.0+`
- Kernel: `6.18.26`
- CPU: `Intel Core i7-9700`, 8 physical cores, no SMT
- Current RAM: `16 GB`
- Expected RAM: `64 GB` after mainboard replacement
- Swap: about `30 GB`
  - `/dev/sdc1`: about `15.4 GB`
  - `/dev/zram0`: about `15.5 GB`, higher priority
- Root/storage: btrfs on `/dev/sdc2`, about `917 GB`, about `877 GB` free
- glibc: `2.42`
- GCC: `15.2.0`
- Multilib/compat32: present and broad
- Network:
  - LAN IP: `192.168.254.200/24`
  - Gateway: `192.168.254.254`

## Current Services Relevant To Dune

Conan Exiles Enhanced is already running under its own user:

```text
user: conan
uid: 1001
gid: 100/users
home: /home/conan
shell: /bin/ksh
```

The Conan server process observed:

```text
/home/conan/conan-enhanced-server/server/ConanSandbox/Binaries/Linux/ConanSandboxServer-Linux-Shipping
```

Current Conan launch arguments include:

```text
-MaxPlayers=20 -Port=7777 -QueryPort=27015 -RconPort=25575 -log -useallavailablecores
```

Observed memory use while running:

```text
RSS: about 9.5 GB
CPU: about 20-25% at the moment checked
```

Ports already occupied by Conan or related tooling:

```text
UDP 7777
UDP 7778
UDP 14001
UDP 27015
TCP 25575
TCP 8088 on 127.0.0.1
```

Other listening services observed:

```text
TCP 22       sshd
TCP 80       httpd
UDP 123      ntpd
UDP 5353     avahi
UDP 161      snmpd
```

## Installed/Absent Pieces

Present:

- Slackware rc.d service model
- `screen`
- `lxc`
- `qemu-system-x86_64`
- `iptables`
- `nftables`
- `rsync`
- `smartmontools`
- `sysstat`
- `logrotate`
- `btrfs-progs`
- 32-bit runtime compatibility libraries
- Vulkan/Mesa packages, including compat32 variants

Not currently on PATH:

- `steamcmd`
- `psql`
- `postgres`
- `docker`
- `podman`

Virtualization note:

- `/dev/kvm` was not present.
- The kernel reported VMX unsupported for KVM mitigation output.
- This is probably firmware/mainboard configuration or the current board issue.
- Native Dune is still the goal, but KVM should be enabled later if possible as
  a fallback tool.

## Slackware Assessment

Slackware is not the weak point. This host has a modern kernel, modern glibc,
current compiler stack, btrfs, multilib, and a proven Unreal dedicated server
workflow from Conan.

The likely problems are Funcom packaging assumptions:

- Their first public setup flow targets Windows Pro plus Hyper-V.
- Direct Linux is described as technically possible but not streamlined.
- Scripts may assume Ubuntu/Debian paths, packages, or systemd.
- Scripts may expect cgroup v2 behavior; this host is currently using cgroup v1.
- PostgreSQL bootstrap details are unknown until the real package/docs land.
- Port layout for one battlegroup is unknown and may conflict with Conan.

The intended Slackware solution is to run Dune under its own user with explicit
shell scripts and an `/etc/rc.d/rc.dune` style wrapper once the process layout is
known.

## Intended Dune User

The user should create this account manually:

```text
login: dune
primary group: users
home: /home/dune
shell: /bin/ksh
```

This mirrors the existing `conan` account style.

Suggested home layout after logging in as `dune`:

```text
/home/dune/
  steamcmd/
  dune-server/
    server/
    config/
    logs/
    scripts/
    backups/
    steamapps/
```

The exact layout can change once the official files are available. Keep Dune
separate from Conan even if they share system packages.

## Suggested Next Steps As `dune`

Do not try to run the full battlegroup until RAM is upgraded.

Safe preparation work:

1. Confirm the new account:

   ```sh
   id
   pwd
   umask
   ```

2. Create a lightweight directory skeleton:

   ```sh
   mkdir -p ~/dune-server/{server,config,logs,scripts,backups,steamapps}
   mkdir -p ~/steamcmd
   chmod 700 ~/dune-server/config ~/dune-server/backups
   ```

3. Confirm SteamCMD availability from the Conan setup or install a separate
   copy for `dune`.

4. Once the self-hosted server package is available, download only. Do not start
   it immediately.

5. Inspect launch scripts and binaries first:

   ```sh
   find ~/dune-server -maxdepth 4 -type f
   find ~/dune-server -maxdepth 4 -type f -name '*.sh' -o -name '*.ini' -o -name '*.json'
   ```

6. Check linked library requirements on Linux binaries:

   ```sh
   ldd /path/to/DuneServerBinary
   ```

7. Identify default ports before launch and avoid Conan's occupied ports.

8. Identify database requirements:

   - Does Funcom ship a local Postgres bundle?
   - Does it expect system PostgreSQL?
   - What database name, role, and schema bootstrap does it require?

9. Only after config and ports are understood, perform a dry foreground launch
   with logs redirected to `~/dune-server/logs/`.

## Port Planning

Conan already uses the common Unreal/Steam ports. Dune should get a distinct
range.

Avoid:

```text
7777/udp
7778/udp
14001/udp
27015/udp
25575/tcp
```

Candidate Dune range, subject to official config support:

```text
UDP 7787-7799
UDP 27025-27039
TCP 25585-25599
```

Do not commit to those numbers until the Dune package reveals how many services
and map processes a battlegroup starts.

## RAM Constraint

Current 16 GB is below the practical threshold, especially with Conan using
about 9.5 GB RSS.

After the board/RAM fix:

- `64 GB` should be enough for first native Dune testing beside Conan.
- Watch actual RSS per Dune process, not just total system memory.
- Avoid swap during gameplay; zram is useful as protection, not capacity.

Useful checks:

```sh
free -h
ps -eo pid,user,rss,vsz,pmem,pcpu,cmd --sort=-rss | head
/usr/sbin/ss -tulpen
```

## Future Service Model

Once Dune can launch manually, wrap it in Slackware-native scripts:

```text
/home/dune/dune-server/scripts/start.sh
/home/dune/dune-server/scripts/stop.sh
/home/dune/dune-server/scripts/status.sh
/etc/rc.d/rc.dune
```

The rc script should:

- run as `dune`, not root
- start database dependency first if needed
- start the Dune coordinator/battlegroup process
- write logs under `/home/dune/dune-server/logs`
- stop cleanly before killing
- avoid interfering with Conan

## Sources Checked

- Official Dune: Awakening self-hosting/developer update pages
- Steam news for Dune: Awakening
- Steam metadata indicating Linux server launch entries
- Local Slackware host inventory

