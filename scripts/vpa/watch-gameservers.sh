#!/bin/bash
# Monitor memory usage of game server pods (Survival_1, Overmap) and log
# recommendations, since VPA cannot target Funcom's ServerSet CRD directly.
#
# Reads actual RSS from metrics-server every INTERVAL seconds and prints a
# recommendation when observed peak exceeds the current memory request by
# THRESHOLD_PCT percent.  No automatic patching — operator reviews and applies
# via experimental_swap.sh or a manual kubectl patch.
#
# Usage: watch-gameservers.sh [--interval 120] [--threshold 20] [--once]

set -euo pipefail

BATTLEGROUP_PREFIX="funcom-seabass-"
INTERVAL=120        # seconds between polls
THRESHOLD_PCT=20    # recommend bump when usage > request * (1 + threshold/100)
ONCE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --interval)  INTERVAL="$2";    shift 2 ;;
        --threshold) THRESHOLD_PCT="$2"; shift 2 ;;
        --once)      ONCE=true;         shift   ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

# Convert a Kubernetes memory string (e.g. 200Mi, 1Gi, 512M) to bytes
to_bytes() {
    local val="$1"
    case "$val" in
        *Ki) echo $(( ${val%Ki} * 1024 )) ;;
        *Mi) echo $(( ${val%Mi} * 1024 * 1024 )) ;;
        *Gi) echo $(( ${val%Gi} * 1024 * 1024 * 1024 )) ;;
        *K)  echo $(( ${val%K}  * 1000 )) ;;
        *M)  echo $(( ${val%M}  * 1000 * 1000 )) ;;
        *G)  echo $(( ${val%G}  * 1000 * 1000 * 1000 )) ;;
        *)   echo "$val" ;;
    esac
}

# Convert bytes to a readable MiB string
to_mib() { echo "$(( $1 / 1024 / 1024 ))Mi"; }

poll_once() {
    local ns
    ns=$(sudo kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name \
         | grep "^$BATTLEGROUP_PREFIX" | head -1)
    if [[ -z "$ns" ]]; then
        echo "$(date -Iseconds) WARN: no battlegroup namespace found"
        return
    fi

    # Get all pods whose names look like game server maps (contain underscore or
    # start with known prefixes).  We identify them by their owning ServerSet via
    # the label set.dune.io/server-set or simply by the app label.
    # Fallback: grep pods that are NOT the standard infrastructure names.
    local infra_names="postgres|rabbitmq|gateway|director|text-router|filebrowser"

    local pods
    pods=$(sudo kubectl get pods -n "$ns" --no-headers \
           -o custom-columns=NAME:.metadata.name,STATUS:.status.phase \
           | grep "Running" | awk '{print $1}' \
           | grep -vE "^($infra_names)-" || true)

    if [[ -z "$pods" ]]; then
        echo "$(date -Iseconds) INFO [$ns]: no game server pods running"
        return
    fi

    while IFS= read -r pod; do
        # Get current memory request from the pod spec (first container)
        local request_raw
        request_raw=$(sudo kubectl get pod "$pod" -n "$ns" \
            -o jsonpath='{.spec.containers[0].resources.requests.memory}' 2>/dev/null || echo "")
        local limit_raw
        limit_raw=$(sudo kubectl get pod "$pod" -n "$ns" \
            -o jsonpath='{.spec.containers[0].resources.limits.memory}' 2>/dev/null || echo "")

        # Get current usage from metrics-server
        local usage_raw
        usage_raw=$(sudo kubectl top pod "$pod" -n "$ns" --no-headers 2>/dev/null \
            | awk '{print $3}' || echo "")

        if [[ -z "$usage_raw" ]]; then
            echo "$(date -Iseconds) WARN [$ns/$pod]: metrics not yet available"
            continue
        fi

        local usage_bytes req_bytes limit_bytes
        usage_bytes=$(to_bytes "$usage_raw")
        req_bytes=$(to_bytes "${request_raw:-0}")
        limit_bytes=$(to_bytes "${limit_raw:-0}")

        local threshold_bytes=$(( req_bytes + req_bytes * THRESHOLD_PCT / 100 ))
        local pct_of_limit=""
        if [[ $limit_bytes -gt 0 ]]; then
            pct_of_limit=" ($(( usage_bytes * 100 / limit_bytes ))% of limit)"
        fi

        if [[ $usage_bytes -gt $threshold_bytes ]] && [[ $req_bytes -gt 0 ]]; then
            local suggested=$(( usage_bytes * 120 / 100 ))  # usage + 20% headroom
            echo "$(date -Iseconds) RECOMMEND [$ns/$pod]: usage=$(to_mib $usage_bytes)${pct_of_limit} > request=${request_raw} — suggest raising request to $(to_mib $suggested) (limit=${limit_raw})"
        else
            echo "$(date -Iseconds) OK [$ns/$pod]: usage=$(to_mib $usage_bytes)${pct_of_limit} request=${request_raw:-unset} limit=${limit_raw:-unset}"
        fi
    done <<< "$pods"
}

echo "$(date -Iseconds) watch-gameservers starting (interval=${INTERVAL}s, threshold=${THRESHOLD_PCT}%)"

if $ONCE; then
    poll_once
    exit 0
fi

while true; do
    poll_once
    sleep "$INTERVAL"
done
