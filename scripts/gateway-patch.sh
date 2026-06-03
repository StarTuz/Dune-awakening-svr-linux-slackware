#!/bin/bash
# DEPRECATED / HISTORICAL — do not use in normal operation.
#
# This script used to (a) add --RMQGameHttpPort=30196 to the gateway Deployment
# and (b) force --RMQGameHostname to the BattleGroup spec's public IP, and had to
# be re-run after every restart/update. Both halves are now retired:
#
#  - --RMQGameHostname is derived by the server-operator from the k3s NODE
#    EXTERNAL IP (`node-external-ip:` in /etc/rancher/k3s/config.yaml). The
#    operator stamps it on every reconcile, so a manual patch was only ever a
#    band-aid that the operator reverted — which is why this had to be re-run.
#    The real fix when the public IP changes is to update node-external-ip and
#    restart k3s (see PUBLIC-IP.md), not to run this script.
#
#  - --RMQGameHttpPort=30196 was unnecessary (GameRmqHttpAddress / the RMQ
#    management API is off the gameplay path) AND stale: the live RMQ management
#    NodePort is dynamic (31506 at time of writing), not 30196. Advertising 30196
#    pointed FLS at a dead port.
#
# Root cause documented and fixed 2026-06-02. Verify the gateway's advertised IP
# with:  dune-ctl --world <world> preflight   (the "gateway IP" row), or
#        dune-ctl --world <world> diagnostics  (gateway RMQ host).
#
# This file is kept only for historical reference. It intentionally does nothing.

set -euo pipefail

cat >&2 <<'EOF'
gateway-patch.sh is DEPRECATED and no longer patches anything.

Why: the gateway --RMQGameHostname is operator-managed from the k3s node
external IP, and the old --RMQGameHttpPort=30196 arg was stale/unnecessary.

To change the advertised public IP (the thing this used to band-aid):
  1. edit /etc/rancher/k3s/config.yaml -> node-external-ip: <new-ip>
  2. sudo rc-service k3s restart
  3. sudo kubectl rollout restart deployment -n funcom-operators
  4. dune-ctl --world <world> public-ip set <new-ip> --yes   # files + spec env
See PUBLIC-IP.md for the full runbook.

To verify the gateway advertises the right IP:
  dune-ctl --world <world> preflight     # "gateway IP" row
EOF
exit 0
