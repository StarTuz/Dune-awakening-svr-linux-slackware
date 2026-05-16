#!/bin/bash
# Capture a known-good live system point: logical Dune backup plus btrfs snapshots.

set -euo pipefail

DUNE_REPO="${DUNE_REPO:-/home/dune/dune-server}"
BTRFS="${BTRFS:-/sbin/btrfs}"
SNAP_NAME="${1:-known-good-$(date -u +%Y%m%d-%H%M%S)}"
ROOT_SNAPSHOT_DIR="${ROOT_SNAPSHOT_DIR:-/.snapshots}"
BACKUPS_SNAPSHOT_DIR="${BACKUPS_SNAPSHOT_DIR:-/srv/backups/.snapshots}"
REPORT_ROOT="${REPORT_ROOT:-/srv/backups/dune/system-snapshots}"
REPORT_DIR="$REPORT_ROOT/$SNAP_NAME"

usage() {
    cat <<EOF
Usage: $0 [snapshot-name]

Creates:
  $ROOT_SNAPSHOT_DIR/<snapshot-name>       read-only btrfs snapshot of /
  $BACKUPS_SNAPSHOT_DIR/<snapshot-name>    read-only btrfs snapshot of /srv/backups
  $REPORT_ROOT/<snapshot-name>/            status, audit, and command output

Run with sudo from arrakis. The script takes a logical Dune backup first so the
running Postgres state has a clean restore artifact in addition to the live
filesystem snapshot.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: run with sudo so btrfs snapshots and Kubernetes checks can complete." >&2
    exit 1
fi

if [ ! -x "$BTRFS" ]; then
    echo "ERROR: btrfs tool not found at $BTRFS" >&2
    exit 1
fi

if [ ! -d "$DUNE_REPO" ]; then
    echo "ERROR: DUNE_REPO not found: $DUNE_REPO" >&2
    exit 1
fi

mkdir -p "$ROOT_SNAPSHOT_DIR" "$BACKUPS_SNAPSHOT_DIR" "$REPORT_DIR"

if [ -e "$ROOT_SNAPSHOT_DIR/$SNAP_NAME" ]; then
    echo "ERROR: root snapshot already exists: $ROOT_SNAPSHOT_DIR/$SNAP_NAME" >&2
    exit 1
fi
if [ -e "$BACKUPS_SNAPSHOT_DIR/$SNAP_NAME" ]; then
    echo "ERROR: backups snapshot already exists: $BACKUPS_SNAPSHOT_DIR/$SNAP_NAME" >&2
    exit 1
fi

run_capture() {
    local name="$1"
    shift

    echo "=== $name ==="
    "$@" 2>&1 | tee "$REPORT_DIR/$name.txt"
}

run_capture_allow_fail() {
    local name="$1"
    shift

    echo "=== $name ==="
    set +e
    "$@" 2>&1 | tee "$REPORT_DIR/$name.txt"
    local status=${PIPESTATUS[0]}
    set -e
    echo "$status" > "$REPORT_DIR/$name.exit"
    return 0
}

capture_findmnt() {
    findmnt -no TARGET,SOURCE,FSTYPE,OPTIONS /
    findmnt -no TARGET,SOURCE,FSTYPE,OPTIONS /srv/backups
}

{
    echo "snapshot=$SNAP_NAME"
    echo "created_utc=$(date -u -Iseconds)"
    echo "host=$(hostname -f 2>/dev/null || hostname)"
    echo "root_snapshot=$ROOT_SNAPSHOT_DIR/$SNAP_NAME"
    echo "backups_snapshot=$BACKUPS_SNAPSHOT_DIR/$SNAP_NAME"
    echo "report_dir=$REPORT_DIR"
} > "$REPORT_DIR/MANIFEST.txt"

run_capture uname uname -a
run_capture findmnt capture_findmnt

if command -v git >/dev/null 2>&1; then
    run_capture_allow_fail git-status git -C "$DUNE_REPO" status --short
    run_capture_allow_fail git-head git -C "$DUNE_REPO" log --oneline -1
fi

echo "=== dune-backup ==="
"$DUNE_REPO/scripts/dune-backup.sh" 2>&1 | tee "$REPORT_DIR/dune-backup.txt"

run_capture_allow_fail security-audit "$DUNE_REPO/scripts/security-audit.sh"

if command -v kubectl >/dev/null 2>&1; then
    ns="$(kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name 2>/dev/null | grep '^funcom-seabass-' | head -n1 || true)"
    if [ -n "$ns" ]; then
        run_capture_allow_fail pods kubectl get pods -n "$ns" -o wide
        run_capture_allow_fail serverstats kubectl get serverstats,serversetscale -n "$ns"
    fi
fi

sync
"$BTRFS" filesystem sync /
"$BTRFS" filesystem sync /srv/backups

echo "=== btrfs snapshots ==="
"$BTRFS" subvolume snapshot -r / "$ROOT_SNAPSHOT_DIR/$SNAP_NAME" | tee "$REPORT_DIR/root-snapshot.txt"
"$BTRFS" subvolume snapshot -r /srv/backups "$BACKUPS_SNAPSHOT_DIR/$SNAP_NAME" | tee "$REPORT_DIR/backups-snapshot.txt"

{
    echo "completed_utc=$(date -u -Iseconds)"
    echo "root_snapshot=$ROOT_SNAPSHOT_DIR/$SNAP_NAME"
    echo "backups_snapshot=$BACKUPS_SNAPSHOT_DIR/$SNAP_NAME"
} >> "$REPORT_DIR/MANIFEST.txt"

echo ""
echo "System snapshot complete."
echo "$REPORT_DIR/MANIFEST.txt"
