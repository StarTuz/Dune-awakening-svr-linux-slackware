#!/bin/bash
# Capture current host and Kubernetes resource usage for capacity comparisons.

set -euo pipefail

DUNE_REPO="${DUNE_REPO:-/home/dune/dune-server}"
SNAP_NAME="${1:-resources-$(date -u +%Y%m%d-%H%M%S)}"
REPORT_ROOT="${REPORT_ROOT:-/srv/backups/dune/resource-snapshots}"
REPORT_DIR="$REPORT_ROOT/$SNAP_NAME"
BATTLEGROUP_PREFIX="funcom-seabass-"

usage() {
    cat <<EOF
Usage: $0 [snapshot-name]

Creates:
  $REPORT_ROOT/<snapshot-name>/

Captured data includes host memory/swap, top processes, filesystem usage,
kubectl pod/resource views, serverstats, metrics-server pod usage, and the
game-server memory watcher output.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: run with sudo so Kubernetes and process data can be captured consistently." >&2
    exit 1
fi

mkdir -p "$REPORT_DIR"

run_capture() {
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

capture_mounts() {
    findmnt -no TARGET,SOURCE,FSTYPE,OPTIONS /
    findmnt -no TARGET,SOURCE,FSTYPE,OPTIONS /srv/backups
    findmnt -no TARGET,SOURCE,FSTYPE,OPTIONS /var/lib/rancher/k3s
    findmnt -no TARGET,SOURCE,FSTYPE,OPTIONS /var/lib/kubelet
}

capture_process_memory() {
    ps -eo pid,user,rss,vsz,pmem,pcpu,cmd --sort=-rss | head -40
}

capture_kubectl_summary() {
    kubectl get nodes -o wide
    echo ""
    kubectl get pods -A -o wide
}

capture_namespace_resources() {
    local ns="$1"
    kubectl get pods -n "$ns" -o custom-columns='NAME:.metadata.name,PHASE:.status.phase,READY:.status.containerStatuses[0].ready,CPU_REQ:.spec.containers[0].resources.requests.cpu,MEM_REQ:.spec.containers[0].resources.requests.memory,CPU_LIM:.spec.containers[0].resources.limits.cpu,MEM_LIM:.spec.containers[0].resources.limits.memory,NODE:.spec.nodeName'
}

capture_namespace_top() {
    local ns="$1"
    kubectl top pod -n "$ns" --containers
}

capture_vpa() {
    local ns="$1"
    kubectl get vpa -n "$ns"
}

ns="$(kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name 2>/dev/null | grep "^$BATTLEGROUP_PREFIX" | head -n1 || true)"

{
    echo "snapshot=$SNAP_NAME"
    echo "created_utc=$(date -u -Iseconds)"
    echo "host=$(hostname -f 2>/dev/null || hostname)"
    echo "report_dir=$REPORT_DIR"
    echo "namespace=${ns:-}"
} > "$REPORT_DIR/MANIFEST.txt"

run_capture uname uname -a
run_capture uptime uptime
run_capture free free -h
run_capture swapon swapon --show
run_capture vmstat vmstat 1 5
run_capture disk df -h
run_capture mounts capture_mounts
run_capture process-memory capture_process_memory
run_capture k8s-summary capture_kubectl_summary

if [ -n "$ns" ]; then
    run_capture namespace-resources capture_namespace_resources "$ns"
    run_capture namespace-top capture_namespace_top "$ns"
    run_capture serverstats kubectl get serverstats,serversetscale -n "$ns"
    run_capture vpa capture_vpa "$ns"
fi

if [ -x "$DUNE_REPO/scripts/vpa/watch-gameservers.sh" ]; then
    run_capture game-server-memory "$DUNE_REPO/scripts/vpa/watch-gameservers.sh" --once
fi

if command -v git >/dev/null 2>&1 && [ -d "$DUNE_REPO/.git" ]; then
    run_capture git-status git -C "$DUNE_REPO" status --short
    run_capture git-head git -C "$DUNE_REPO" log --oneline -1
fi

echo "completed_utc=$(date -u -Iseconds)" >> "$REPORT_DIR/MANIFEST.txt"

echo ""
echo "Resource snapshot complete."
echo "$REPORT_DIR/MANIFEST.txt"
