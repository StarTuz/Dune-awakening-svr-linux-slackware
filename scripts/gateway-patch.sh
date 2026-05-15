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

find_gateway_deploy() {
    sudo kubectl get deployments -n "$NS" --no-headers -o custom-columns=NAME:.metadata.name 2>/dev/null \
        | grep -- '-sgw-deploy$' \
        | head -n1
}

wait_for_gateway_deploy() {
    local timeout="${1:-180}"
    local elapsed=0
    local interval=5
    local found

    while [ "$elapsed" -lt "$timeout" ]; do
        found="$(find_gateway_deploy || true)"
        if [ -n "$found" ]; then
            GW_DEPLOY="$found"
            return 0
        fi
        sleep "$interval"
        elapsed=$((elapsed + interval))
        echo "  Still waiting for gateway deployment... (${elapsed}s / ${timeout}s)"
    done

    echo "ERROR: gateway deployment not found in $NS after ${timeout}s." >&2
    echo "If the battlegroup is stopped, start it first and rerun this script." >&2
    return 1
}

echo "Gateway: $GW_DEPLOY"
echo "Namespace: $NS"
echo ""

if ! sudo kubectl get deployment "$GW_DEPLOY" -n "$NS" >/dev/null 2>&1; then
    echo "Gateway deployment is not present yet; waiting for operator to create it..."
    wait_for_gateway_deploy 180
    echo "Gateway: $GW_DEPLOY"
    echo ""
fi

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
