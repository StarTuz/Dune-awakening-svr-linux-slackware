#!/bin/bash
# Apply --RMQGameHttpPort=30196 to the gateway deployment (idempotent).
#
# The server-operator regenerates the gateway Deployment from the BattleGroup CR
# on every restart or update, stripping any manual patches.  Run this after
# every battlegroup restart or update to restore the fix.
#
# Background: the gateway Python code looks up mq-game-svc NodePorts via the
# Kubernetes API, finds the amqp port (31982) but not the http port because the
# port name doesn't match what it expects.  Without this arg it sends
# GameRmqHttpAddress: "47.145.51.160:None" to FLS.

set -euo pipefail

NS=$(sudo kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name \
     | grep "^funcom-seabass-" | head -1)
BG="${NS#funcom-seabass-}"
GW_DEPLOY="${BG}-sgw-deploy"

echo "Gateway: $GW_DEPLOY"
echo "Namespace: $NS"
echo ""

if sudo kubectl get deployment "$GW_DEPLOY" -n "$NS" -o json \
   | jq -e '.spec.template.spec.containers[0].args | any(. == "--RMQGameHttpPort=30196")' \
   > /dev/null 2>&1; then
    echo "--RMQGameHttpPort=30196 already present — nothing to do."
    exit 0
fi

echo "Patching gateway to add --RMQGameHttpPort=30196 ..."
sudo kubectl patch deployment "$GW_DEPLOY" -n "$NS" --type='json' \
  -p='[{"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--RMQGameHttpPort=30196"}]'

echo "Waiting for rollout ..."
sudo kubectl rollout status deployment/"$GW_DEPLOY" -n "$NS" --timeout=120s

echo ""
echo "Done. Gateway will send GameRmqHttpAddress: \"47.145.51.160:30196\" to FLS on next start."
