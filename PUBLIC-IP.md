# Public IP Runbook

This deployment advertises the home Internet public IP to Funcom Live Services
and to clients through the BattleGroup utility specs and gateway arguments.

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
```

`show` reports:

- local world/capsule file IPs
- live BattleGroup spec IPs
- gateway `--RMQGameHostname`
- whether `--RMQGameHttpPort=30196` is present

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

## Change With dune-ctl

Preview first:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip set <new-public-ip> --dry-run
```

Apply explicitly:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip set <new-public-ip> --yes
```

Apply a detected IP only after inspecting the plan:

```sh
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip apply-detected --dry-run
~/dune-server/dune-ctl/target/release/dune-ctl --world Ixware public-ip apply-detected --yes
```

What this updates:

- local `~/.dune` world/capsule files
- live `BattleGroup.spec.utilities.{director,serverGateway,textRouter}.spec.envVars`
- live gateway Deployment `--RMQGameHostname=<new-public-ip>`
- live gateway Deployment `--RMQGameHttpPort=30196`
- stale `kubectl.kubernetes.io/last-applied-configuration` annotation, if present

The BattleGroup `status.*` addresses can lag after the spec and gateway are
correct. Treat `spec` and the active gateway Deployment as authoritative while
controllers refresh.

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

Finally re-run:

```sh
~/dune-server/scripts/gateway-patch.sh
```

`gateway-patch.sh` derives the public IP from the live BattleGroup spec and
repairs both:

```text
--RMQGameHostname=<public-ip>
--RMQGameHttpPort=30196
```

## Router Checklist

The router must forward:

```text
UDP 7782-7790
TCP 31982
TCP 30196
```

The host can verify firewalld exposure, but not router forwarding:

```sh
~/dune-server/scripts/security-audit.sh
```
