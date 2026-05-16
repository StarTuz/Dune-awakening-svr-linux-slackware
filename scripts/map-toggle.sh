#!/bin/bash
# Start or stop an individual map (ServerSet) within the active battlegroup.
#
# Usage:
#   map-toggle.sh start DeepDesert_1
#   map-toggle.sh stop  DeepDesert_1
#   map-toggle.sh list
#
# How it works:
#   Starting/stopping a map requires patching four objects in sequence:
#     BattleGroup CR  →  ServerGroup  →  ServerSet  →  ServerSetScale
#   The BattleGroup operator propagates CR changes down to ServerGroup and
#   ServerSet, but the ServerSetScale (the final pod-creation trigger) does
#   not auto-update from those changes alone.  This script patches both the
#   BattleGroup CR and the ServerSetScale directly.

set -euo pipefail

BATTLEGROUP_PREFIX="funcom-seabass-"
NS=$(sudo kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name \
     | grep "^$BATTLEGROUP_PREFIX" | head -1)
BG="${NS#$BATTLEGROUP_PREFIX}"
WAIT_TIMEOUT="${WAIT_TIMEOUT:-60}"

usage() {
    echo "Usage: $0 <start|stop|list> [MapName]"
    echo "  list              — show all maps and their current replica count"
    echo "  start <MapName>   — start the named map"
    echo "  stop  <MapName>   — stop the named map"
    exit 1
}

[[ $# -lt 1 ]] && usage

CMD="$1"
MAP="${2:-}"

case "$CMD" in
    list)
        echo "=== Maps in $NS ==="
        sudo kubectl get battlegroups -n "$NS" "$BG" -o json \
          | jq -r '.spec.serverGroup.template.spec.sets | to_entries[]
                   | "\(.value.replicas // 0)\t\(.value.map)"' \
          | sort -k2 \
          | awk '{printf "  %-3s %s\n", ($1=="1"?"ON":"off"), $2}'
        echo ""
        echo "Live ServerSet phases:"
        sudo kubectl get serverset -n "$NS" \
          --sort-by=.metadata.name \
          -o custom-columns='MAP:.spec.map,PHASE:.status.phase,READY:.status.readyReplicas,TARGET:.status.targetReplicas'
        exit 0
        ;;
    start) REPLICAS=1 ;;
    stop)  REPLICAS=0 ;;
    *) usage ;;
esac

[[ -z "$MAP" ]] && usage

# Find index of this map in the BattleGroup CR sets array
INDEX=$(sudo kubectl get battlegroups -n "$NS" "$BG" -o json \
        | jq -r --arg map "$MAP" \
          '.spec.serverGroup.template.spec.sets | to_entries[]
           | select(.value.map==$map) | .key' 2>/dev/null)

if [[ -z "$INDEX" ]]; then
    echo "ERROR: map '$MAP' not found in battlegroup '$BG'"
    echo "Run '$0 list' to see available maps."
    exit 1
fi

# Derive the ServerSet and ServerSetScale names from the map name
# Convention: lowercase, underscores → hyphens, prefixed with BG name
MAP_SLUG=$(echo "$MAP" | tr '[:upper:]' '[:lower:]' | tr '_' '-')
SERVERSET="${BG}-sg-${MAP_SLUG}"
SCALE="${BG}-${MAP_SLUG}"

echo "Map:           $MAP (sets index $INDEX)"
echo "ServerSet:     $SERVERSET"
echo "ServerSetScale: $SCALE"
echo "Action:        $CMD (replicas → $REPLICAS)"
echo ""

# Resolve stable partition IDs from the database world partition list. A map
# started without this field launches without -PartitionIndex and can crash when
# it joins a farm where existing servers use stable indices.
PATCH="[{\"op\":\"replace\",\"path\":\"/spec/serverGroup/template/spec/sets/${INDEX}/replicas\",\"value\":${REPLICAS}}]"
if [[ "$CMD" == "start" ]]; then
    PARTITIONS=$(sudo kubectl get battlegroups -n "$NS" "$BG" -o json \
        | jq -c --arg map "$MAP" \
          '[.spec.database.template.spec.deployment.spec.worldPartitions[]
            | select(.map==$map)
            | .partitions[]
            | select((.disable // false) == false)
            | .id]' 2>/dev/null)

    if [[ -z "$PARTITIONS" || "$PARTITIONS" == "[]" ]]; then
        echo "ERROR: no enabled world partition IDs found for '$MAP'"
        exit 1
    fi

    PARTITION_COUNT=$(jq -r 'length' <<<"$PARTITIONS")
    PARTITION_INDEX=$(jq -r '.[0]' <<<"$PARTITIONS")
    HAS_PARTITION_ARG=$(sudo kubectl get battlegroups -n "$NS" "$BG" -o json \
        | jq -r --argjson index "$INDEX" \
          'any(.spec.serverGroup.template.spec.sets[$index].arguments[]?; startswith("-PartitionIndex="))')

    PATCH="[{\"op\":\"replace\",\"path\":\"/spec/serverGroup/template/spec/sets/${INDEX}/partitions\",\"value\":${PARTITIONS}}"
    if [[ "$PARTITION_COUNT" == "1" && "$HAS_PARTITION_ARG" != "true" ]]; then
        PATCH="${PATCH},{\"op\":\"add\",\"path\":\"/spec/serverGroup/template/spec/sets/${INDEX}/arguments/-\",\"value\":\"-PartitionIndex=${PARTITION_INDEX}\"}"
        echo "Partition arg: -PartitionIndex=$PARTITION_INDEX"
    fi
    PATCH="${PATCH},{\"op\":\"replace\",\"path\":\"/spec/serverGroup/template/spec/sets/${INDEX}/replicas\",\"value\":${REPLICAS}}]"
    echo "Partitions:    $PARTITIONS"
fi

# 1. Patch the BattleGroup CR (propagates to ServerGroup + ServerSet)
sudo kubectl patch battlegroup "$BG" -n "$NS" \
  --type='json' \
  -p="$PATCH"
echo "BattleGroup CR patched."

if [[ "$CMD" == "start" ]]; then
    echo "Waiting for ServerSet to receive partitions before scaling..."
    deadline=$((SECONDS + WAIT_TIMEOUT))
    while true; do
        SERVERSET_STATE=$(sudo kubectl get serverset "$SERVERSET" -n "$NS" -o json \
            | jq -c '{replicas:(.spec.replicas // 0), partitions:(.spec.partitions // [])}' 2>/dev/null || true)

        if [[ "$SERVERSET_STATE" == "{\"replicas\":${REPLICAS},\"partitions\":${PARTITIONS}}" ]]; then
            break
        fi

        if (( SECONDS >= deadline )); then
            echo "ERROR: ServerSet did not receive expected state before timeout."
            echo "Expected: {\"replicas\":${REPLICAS},\"partitions\":${PARTITIONS}}"
            echo "Current:  ${SERVERSET_STATE:-unavailable}"
            exit 1
        fi

        sleep 2
    done
    echo "ServerSet synchronized."
fi

# 2. Patch the ServerSetScale if it exists (final pod-creation trigger for maps
#    that were stopped at cluster start; maps that were running do not have one)
if sudo kubectl get serversetscale "$SCALE" -n "$NS" &>/dev/null; then
    sudo kubectl patch serversetscale "$SCALE" -n "$NS" \
      --type='json' \
      -p="[{\"op\":\"replace\",\"path\":\"/spec/replicas\",\"value\":${REPLICAS}}]"
    echo "ServerSetScale patched."
else
    echo "No ServerSetScale for $MAP — BattleGroup CR patch is sufficient."
fi

echo ""
if [[ "$CMD" == "start" ]]; then
    echo "Starting — watch progress with:"
    echo "  sudo kubectl get serverset $SERVERSET -n $NS -w"
    echo "  ~/dune-server/scripts/vpa/watch-gameservers.sh --once"
else
    echo "Stopping — pod will terminate within terminationGracePeriodSeconds (120s)."
fi
