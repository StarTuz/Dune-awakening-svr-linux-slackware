# Public IP Runbook

This deployment advertises the home Internet public IP to Funcom Live Services
and to clients. The IP lives in **two independent places**, and a rotation must
update **both**:

1. **k3s `node-external-ip`** (`/etc/rancher/k3s/config.yaml`) — the
   server-operator derives the gateway `--RMQGameHostname` and every FLS-declared
   address (`status.servers[].ip`, RMQ/director/server addresses) from the k3s
   **Node ExternalIP**. This is the authoritative source for what clients are
   told to connect to. **`dune-ctl public-ip set` does NOT touch this** — it must
   be changed by hand and picked up via a k3s + operator restart.
2. **Advertised config** — local `~/.dune` world/capsule files and the live
   `BattleGroup.spec.utilities.*.envVars[HOST_DATACENTER_IP_ADDRESS]`. This is
   what `dune-ctl public-ip set` updates.

If you change only (2), the operator keeps re-stamping the OLD IP from (1) into
the gateway and FLS declarations on every reconcile — the bug fixed 2026-06-02
(stale `47.145.51.160` lingering after the rotation to `47.145.31.211`). See the
memory note `reference_operator_host_ip_source`.

Current public IP:

```text
47.145.31.211
```

The TP-Link A7 currently handles LAN hairpin/NAT reflection, so LAN clients can
use the normal FLS/browser path. The old Steam launcher
`-ConnectToIP=192.168.254.200:7784` override is not needed.

## Check Current Configuration

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip show
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip check
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware preflight   # "gateway IP" row
```

`show` reports:

- local world/capsule file IPs
- live BattleGroup spec IPs
- gateway `--RMQGameHostname` (operator-derived from `node-external-ip`)

`preflight`'s **gateway IP** row fails when the gateway's advertised
`--RMQGameHostname` does not match the configured public IP — the fastest way to
catch a missed `node-external-ip` update.

`check` queries HTTPS public-IP providers, requires at least two matching valid
public IP responses, and compares the detected value with local/live/gateway
configuration. It exits non-zero when the detected IP differs, which makes it
suitable for a manual check or a cron alert.

Default providers:

```text
https://api.ipify.org
https://ifconfig.me/ip
https://icanhazip.com
```

Override providers when needed:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip check \
  --provider https://api.ipify.org \
  --provider https://icanhazip.com
```

## Rotation Procedure

### Step 1 — k3s node external IP (authoritative; needs interactive root)

```sh
sudo cp /etc/rancher/k3s/config.yaml /etc/rancher/k3s/config.yaml.bak-$(date +%Y%m%d)
sudo sed -i 's#node-external-ip: <old-ip>#node-external-ip: <new-ip>#' /etc/rancher/k3s/config.yaml
sudo rc-service k3s restart                                  # containerd re-adopts pods; brief control-plane blip
sudo kubectl rollout restart deployment -n funcom-operators  # the k3s restart alone does NOT refresh the cached host IP
```

Within ~5s of the operator restart the gateway `--RMQGameHostname`,
`status.*` addresses, and FLS declarations flip to the new IP, and the operator
rolls a fresh gateway pod. No manual gateway patch is required.

### Step 2 — advertised config (files + spec env)

Preview, then apply:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip set <new-ip> --dry-run
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip set <new-ip> --yes
```

Or apply a detected IP after inspecting the plan:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip apply-detected --dry-run
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip apply-detected --yes
```

What `public-ip set` updates:

- local `~/.dune` world/capsule files
- live `BattleGroup.spec.utilities.{director,serverGateway,textRouter}.spec.envVars`
- live gateway Deployment `--RMQGameHostname=<new-ip>` (a courtesy patch; the
  operator re-derives this from `node-external-ip`, so Step 1 is what makes it
  durable)
- stale `kubectl.kubernetes.io/last-applied-configuration` annotation, if present

### Step 3 — verify

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware preflight   # gateway IP row should be OK
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip show
```

`public-ip show` should report local / live spec / gateway RMQ all equal to the
new IP. There should be zero occurrences of the old IP in the BattleGroup CR:

```sh
sudo kubectl get battlegroup <bg> -n <ns> -o json | grep -c '<old-ip>'   # expect 0
```

## Manual Update Locations

If `dune-ctl` is unavailable, update these local files:

```text
~/.dune/<battlegroup>.yaml
~/.dune/capsules/<env>/<battlegroup>/capsule.env
~/.dune/capsules/<env>/<battlegroup>/battlegroup.yaml
~/.dune/settings.conf  # optional/cosmetic when present
```

Then patch the live BattleGroup utility env vars:

```text
BattleGroup.spec.utilities.director.spec.envVars[HOST_DATACENTER_IP_ADDRESS]
BattleGroup.spec.utilities.serverGateway.spec.envVars[HOST_DATACENTER_IP_ADDRESS]
BattleGroup.spec.utilities.textRouter.spec.envVars[HOST_DATACENTER_IP_ADDRESS]
```

The gateway `--RMQGameHostname` and FLS addresses still come from
`node-external-ip` (Step 1) — there is no separate gateway-patch step.
(`scripts/gateway-patch.sh` is a deprecated no-op kept only for reference.)

## Router Checklist

The router must forward:

```text
UDP 7782-7790
TCP 31982          # RMQ game AMQP (the RMQ management HTTP port stays private)
```

The host can verify firewalld exposure, but not router forwarding:

```sh
~/dune-server/scripts/security-audit.sh
```
