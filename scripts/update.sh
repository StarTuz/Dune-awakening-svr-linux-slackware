#!/bin/bash
# Update Dune: Awakening to the latest Steam build with local safety rails.
#
# This wrapper deliberately does more than Funcom's battlegroup.sh update:
#   1. Take a host backup bundle, including a DB dump by default.
#   2. Stop the battlegroup before applying the update.
#   3. SteamCMD pre-fetch with validate, avoiding revoked PTC manifest failures.
#   4. Run Funcom's update flow.
#   5. Re-apply local Slackware patches after any script overwrite.
#   6. Re-assert/check/repair DB credentials expected by Funcom db utils.
#   7. Re-apply the gateway RMQ HTTP port patch.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DOWNLOAD_PATH="$REPO_ROOT/server"
BATTLEGROUP_PREFIX="funcom-seabass-"

skip_backup=0
skip_stop=0
skip_db_fix=0
start_after=0
bgname=""
steamcmd_path="${STEAMCMD:-}"

usage() {
    cat <<EOF
Usage: $0 [options]

Options:
  --bg NAME          Battlegroup name without funcom-seabass- prefix
  --steamcmd PATH    SteamCMD script path. Defaults to /home/<sudo-user>/steamcmd/steamcmd.sh
  --skip-backup      Do not run scripts/dune-backup.sh before updating
  --skip-stop        Do not stop the battlegroup before updating
  --skip-db-fix      Do not run DB credential check/repair after updating
  --start-after      Start the battlegroup after successful update
  -h, --help         Show this help
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --bg)
            bgname="${2:-}"
            shift 2
            ;;
        --steamcmd)
            steamcmd_path="${2:-}"
            shift 2
            ;;
        --skip-backup)
            skip_backup=1
            shift
            ;;
        --skip-stop)
            skip_stop=1
            shift
            ;;
        --skip-db-fix)
            skip_db_fix=1
            shift
            ;;
        --start-after)
            start_after=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

select_battlegroup() {
    if [ -n "$bgname" ]; then
        ns="$BATTLEGROUP_PREFIX$bgname"
        return
    fi

    mapfile -t namespaces < <(sudo kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name | grep "^$BATTLEGROUP_PREFIX" || true)
    if [ "${#namespaces[@]}" -eq 0 ]; then
        echo "ERROR: no battlegroup namespace found" >&2
        exit 1
    elif [ "${#namespaces[@]}" -eq 1 ]; then
        ns="${namespaces[0]}"
        bgname="${ns#$BATTLEGROUP_PREFIX}"
    else
        echo "Available battlegroups:"
        for i in "${!namespaces[@]}"; do
            printf "  %d. %s\n" "$((i+1))" "${namespaces[$i]#$BATTLEGROUP_PREFIX}"
        done
        read -r -p "Select battlegroup: " index
        if ! [[ "$index" =~ ^[0-9]+$ ]] || [ "$index" -lt 1 ] || [ "$index" -gt "${#namespaces[@]}" ]; then
            echo "ERROR: invalid selection" >&2
            exit 1
        fi
        ns="${namespaces[$((index-1))]}"
        bgname="${ns#$BATTLEGROUP_PREFIX}"
    fi
}

patch_stop() {
    local value="$1"
    sudo kubectl patch battlegroup "$bgname" -n "$ns" --type=merge -p "{\"spec\":{\"stop\":$value}}"
}

resolve_steamcmd() {
    if [ -n "$steamcmd_path" ]; then
        return
    fi

    local owner="${SUDO_USER:-$USER}"
    local owner_home=""
    if command -v getent >/dev/null 2>&1; then
        owner_home="$(getent passwd "$owner" | cut -d: -f6)"
    fi
    if [ -z "$owner_home" ]; then
        owner_home="/home/$owner"
    fi

    for candidate in \
        "$owner_home/steamcmd/steamcmd.sh" \
        "/home/dune/steamcmd/steamcmd.sh" \
        "$HOME/steamcmd/steamcmd.sh"; do
        if [ -x "$candidate" ]; then
            steamcmd_path="$candidate"
            return
        fi
    done

    echo "ERROR: steamcmd.sh not found." >&2
    echo "Set it explicitly with: $0 --steamcmd /path/to/steamcmd.sh" >&2
    exit 1
}

wait_stopped() {
    local timeout="${1:-300}"
    local elapsed=0
    local interval=5
    echo "Waiting for game server pods to stop..."
    while [ "$elapsed" -lt "$timeout" ]; do
        local running
        running="$(sudo kubectl get pods -n "$ns" --no-headers 2>/dev/null | grep -- '-sg-' | grep -c 'Running' || true)"
        if [ "$running" = "0" ]; then
            echo "Battlegroup game pods are stopped."
            return 0
        fi
        sleep "$interval"
        elapsed=$((elapsed + interval))
        echo "  Still stopping... (${elapsed}s / ${timeout}s, running game pods=$running)"
    done
    echo "WARNING: game pods still running after ${timeout}s; continuing cautiously." >&2
}

select_battlegroup
resolve_steamcmd

echo "=== Dune update target ==="
echo "Battlegroup: $bgname"
echo "Namespace:   $ns"
echo "SteamCMD:    $steamcmd_path"

if [ "$skip_backup" -eq 0 ]; then
    echo ""
    echo "=== Pre-update backup ==="
    "$SCRIPT_DIR/dune-backup.sh" --bg "$bgname"
else
    echo ""
    echo "=== Pre-update backup skipped by request ==="
fi

if [ "$skip_stop" -eq 0 ]; then
    echo ""
    echo "=== Stopping battlegroup before update ==="
    patch_stop true
    wait_stopped 300
else
    echo ""
    echo "=== Battlegroup stop skipped by request ==="
fi

echo ""
echo "=== Pre-fetching with validate (works around revoked PTC manifests) ==="
"$steamcmd_path" +force_install_dir "$DOWNLOAD_PATH" \
    +login anonymous +app_update 3104830 validate +quit

echo ""
echo "=== Re-applying Slackware patches to Funcom scripts ==="
"$SCRIPT_DIR/funcom-patches.sh"

echo ""
echo "=== Running Funcom update flow ==="
"$DOWNLOAD_PATH/scripts/battlegroup.sh" update

echo ""
echo "=== Re-applying Slackware patches after Funcom update flow ==="
"$SCRIPT_DIR/funcom-patches.sh"

if [ "$skip_db_fix" -eq 0 ]; then
    echo ""
    echo "=== Verifying database utility credentials ==="
    if "$SCRIPT_DIR/db-credentials.sh" check --bg "$bgname"; then
        echo "Database credentials are valid."
    else
        echo "Database credential check failed; attempting repair..."
        "$SCRIPT_DIR/db-credentials.sh" fix --bg "$bgname"
    fi
else
    echo ""
    echo "=== Database credential repair skipped by request ==="
fi

echo ""
echo "=== Re-applying gateway RMQ HTTP port patch ==="
"$SCRIPT_DIR/gateway-patch.sh"

if [ "$start_after" -eq 1 ]; then
    echo ""
    echo "=== Starting battlegroup after update ==="
    patch_stop false
else
    echo ""
    echo "Battlegroup remains stopped. Start when ready:"
    echo "  ~/dune-server/server/scripts/battlegroup.sh start"
fi

echo ""
echo "=== Update complete ==="
