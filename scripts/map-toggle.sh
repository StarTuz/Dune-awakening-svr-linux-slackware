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

# 1. Patch the BattleGroup CR (propagates to ServerGroup + ServerSet)
sudo kubectl patch battlegroup "$BG" -n "$NS" \
  --type='json' \
  -p="[{\"op\":\"replace\",\"path\":\"/spec/serverGroup/template/spec/sets/${INDEX}/replicas\",\"value\":${REPLICAS}}]"
echo "BattleGroup CR patched."

# 2. Patch the ServerSetScale (final pod-creation trigger)
sudo kubectl patch serversetscale "$SCALE" -n "$NS" \
  --type='json' \
  -p="[{\"op\":\"replace\",\"path\":\"/spec/replicas\",\"value\":${REPLICAS}}]"
echo "ServerSetScale patched."

echo ""
if [[ "$CMD" == "start" ]]; then
    echo "Starting — watch progress with:"
    echo "  sudo kubectl get serverset $SERVERSET -n $NS -w"
    echo "  ~/dune-server/scripts/vpa/watch-gameservers.sh --once"
else
    echo "Stopping — pod will terminate within terminationGracePeriodSeconds (120s)."
fi
