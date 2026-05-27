#!/bin/bash
# Apply public RMQ gateway arguments to the gateway deployment (idempotent).
#
# The server-operator regenerates the gateway Deployment from the BattleGroup CR
# on every restart or update, stripping any manual patches.  Run this after
# every battlegroup restart or update to restore the fix.
#
# Background: the gateway Python code looks up mq-game-svc NodePorts via the
# Kubernetes API, finds the amqp port (31982) but not the http port because the
# port name doesn't match what it expects.  Without this arg it sends
# GameRmqHttpAddress: "47.145.31.211:None" to FLS.

set -euo pipefail

RMQ_GAME_HTTP_PORT="${RMQ_GAME_HTTP_PORT:-30196}"

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

PUBLIC_IP="${PUBLIC_IP:-$(sudo kubectl get battlegroup "$BG" -n "$NS" -o json \
    | jq -r '.spec.utilities.serverGateway.spec.envVars[]? | select(.name == "HOST_DATACENTER_IP_ADDRESS") | .value' \
    | head -n1)}"
if [ -z "$PUBLIC_IP" ] || [ "$PUBLIC_IP" = "null" ]; then
    echo "ERROR: could not derive HOST_DATACENTER_IP_ADDRESS from BattleGroup $BG." >&2
    echo "Set PUBLIC_IP=<address> explicitly or fix the BattleGroup spec." >&2
    exit 1
fi
echo "Public IP: $PUBLIC_IP"
echo ""

if ! sudo kubectl get deployment "$GW_DEPLOY" -n "$NS" >/dev/null 2>&1; then
    echo "Gateway deployment is not present yet; waiting for operator to create it..."
    wait_for_gateway_deploy 180
    echo "Gateway: $GW_DEPLOY"
    echo ""
fi

GW_JSON="$(sudo kubectl get deployment "$GW_DEPLOY" -n "$NS" -o json)"

host_idx="$(printf '%s' "$GW_JSON" | jq -r '.spec.template.spec.containers[0].args | to_entries[] | select(.value | startswith("--RMQGameHostname=")) | .key' | head -n1)"
host_arg="$(printf '%s' "$GW_JSON" | jq -r '.spec.template.spec.containers[0].args[]? | select(startswith("--RMQGameHostname="))' | head -n1)"
has_http_port="$(printf '%s' "$GW_JSON" | jq -r --arg port "$RMQ_GAME_HTTP_PORT" '.spec.template.spec.containers[0].args | any(. == ("--RMQGameHttpPort=" + $port))')"

patch='[]'
if [ -n "$host_idx" ] && [ "$host_arg" != "--RMQGameHostname=$PUBLIC_IP" ]; then
    echo "Patching gateway to set --RMQGameHostname=$PUBLIC_IP ..."
    patch="$(printf '%s' "$patch" | jq \
        --arg path "/spec/template/spec/containers/0/args/$host_idx" \
        --arg value "--RMQGameHostname=$PUBLIC_IP" \
        '. + [{op:"replace", path:$path, value:$value}]')"
fi

if [ "$has_http_port" != "true" ]; then
    echo "Patching gateway to add --RMQGameHttpPort=$RMQ_GAME_HTTP_PORT ..."
    patch="$(printf '%s' "$patch" | jq \
        --arg value "--RMQGameHttpPort=$RMQ_GAME_HTTP_PORT" \
        '. + [{op:"add", path:"/spec/template/spec/containers/0/args/-", value:$value}]')"
fi

if [ "$patch" = "[]" ]; then
    echo "--RMQGameHostname=$PUBLIC_IP and --RMQGameHttpPort=$RMQ_GAME_HTTP_PORT already present — nothing to do."
    exit 0
fi

sudo kubectl patch deployment "$GW_DEPLOY" -n "$NS" --type='json' -p="$patch"

echo "Waiting for rollout ..."
sudo kubectl rollout status deployment/"$GW_DEPLOY" -n "$NS" --timeout=120s

echo ""
echo "Done. Gateway will send GameRmqAddress: \"$PUBLIC_IP:31982\" and GameRmqHttpAddress: \"$PUBLIC_IP:$RMQ_GAME_HTTP_PORT\" to FLS on next start."
