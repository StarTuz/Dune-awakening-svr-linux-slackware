#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BATTLEGROUP_SCRIPT="$REPO_ROOT/server/scripts/battlegroup.sh"
BATTLEGROUP_PREFIX="funcom-seabass-"
BACKUP_ROOT="${BACKUP_ROOT:-/srv/backups/dune}"

usage() {
    cat <<EOF
Usage: $0 [--bg NAME] [--name BACKUP_NAME] [--skip-db]

Creates a host-side Dune backup bundle under:
  $BACKUP_ROOT/<battlegroup>/<timestamp>/

The bundle includes a Funcom DatabaseOperation dump, Kubernetes metadata,
deployed UserSettings, local User*.ini defaults, diagnostics, and a manifest.

Options:
  --bg NAME        Battlegroup name without funcom-seabass- prefix
  --name NAME      Database backup filename. Defaults to <bg>-<timestamp>.backup
  --skip-db        Collect metadata/settings only; do not create a DB dump
  -h, --help       Show this help
EOF
}

bgname=""
backup_name=""
skip_db=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --bg)
            bgname="${2:-}"
            shift 2
            ;;
        --name)
            backup_name="${2:-}"
            shift 2
            ;;
        --skip-db)
            skip_db=1
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

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: required command not found: $1" >&2
        exit 1
    fi
}

need_cmd sudo
need_cmd kubectl
need_cmd tar

if [ -z "$bgname" ]; then
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
else
    ns="$BATTLEGROUP_PREFIX$bgname"
fi

timestamp="$(date -u +%Y%m%d-%H%M%S)"
if [ -z "$backup_name" ]; then
    backup_name="$bgname-$timestamp.backup"
fi

dest="$BACKUP_ROOT/$bgname/$timestamp"
database_dest="$dest/database"
k8s_dest="$dest/k8s"
settings_dest="$dest/user-settings"
mkdir -p "$database_dest" "$k8s_dest" "$settings_dest/deployed" "$settings_dest/local"

manifest="$dest/MANIFEST.txt"
{
    echo "Dune backup manifest"
    echo "created_utc=$timestamp"
    echo "battlegroup=$bgname"
    echo "namespace=$ns"
    echo "backup_name=$backup_name"
    echo "repo_root=$REPO_ROOT"
    git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null | sed 's/^/repo_commit=/'
} > "$manifest"

echo "Backup bundle: $dest"

if [ "$skip_db" -eq 0 ]; then
    echo "Creating database dump via Funcom DatabaseOperation..."
    "$BATTLEGROUP_SCRIPT" backup "$backup_name"

    src_dir="/funcom/artifacts/database-dumps/$bgname"
    src_backup="$src_dir/$backup_name"
    if sudo test ! -f "$src_backup"; then
        echo "ERROR: expected database dump not found: $src_backup" >&2
        exit 1
    fi

    echo "Copying database dump into backup bundle..."
    sudo cp "$src_backup" "$database_dest/"
    if sudo test -f "$src_backup.yaml"; then
        sudo cp "$src_backup.yaml" "$database_dest/"
    fi
    sudo chown -R "$(id -u):$(id -g)" "$database_dest"
else
    echo "Skipping database dump by request."
fi

echo "Capturing Kubernetes metadata..."
sudo kubectl get battlegroup "$bgname" -n "$ns" -o yaml > "$k8s_dest/battlegroup.yaml" 2>/dev/null || true
sudo kubectl get databasedeployments -n "$ns" -o yaml > "$k8s_dest/databasedeployments.yaml" 2>/dev/null || true
sudo kubectl get databaseoperations -n "$ns" -o yaml > "$k8s_dest/databaseoperations.yaml" 2>/dev/null || true
sudo kubectl get databasebackups -n "$ns" -o yaml > "$k8s_dest/databasebackups.yaml" 2>/dev/null || true
sudo kubectl get databasebackupschedules -n "$ns" -o yaml > "$k8s_dest/databasebackupschedules.yaml" 2>/dev/null || true
sudo kubectl get pvc -n "$ns" -o yaml > "$k8s_dest/pvc.yaml" 2>/dev/null || true
sudo kubectl get pv -o yaml > "$k8s_dest/pv-all.yaml" 2>/dev/null || true
sudo kubectl get all -n "$ns" -o wide > "$k8s_dest/get-all.txt" 2>/dev/null || true

echo "Capturing local User*.ini defaults..."
cp "$REPO_ROOT"/server/scripts/setup/config/User*.ini "$settings_dest/local/" 2>/dev/null || true

echo "Capturing deployed UserSettings from filebrowser pod..."
fb_pod="$(sudo kubectl get pods -n "$ns" -l role=igw-filebrowser --no-headers -o custom-columns=NAME:.metadata.name 2>/dev/null | head -n1 || true)"
if [ -n "$fb_pod" ]; then
    for file in UserEngine.ini UserGame.ini; do
        sudo kubectl exec -n "$ns" "$fb_pod" -- cat "/srv/UserSettings/$file" > "$settings_dest/deployed/$file" 2>/dev/null || true
    done
else
    echo "WARNING: filebrowser pod not found; deployed UserSettings not captured" | tee -a "$manifest" >&2
fi

echo "Capturing dune-ctl diagnostics..."
if command -v dune-ctl >/dev/null 2>&1; then
    dune-ctl settings list > "$dest/dune-ctl-settings.txt" 2>&1 || true
    dune-ctl diagnostics > "$dest/dune-ctl-diagnostics.txt" 2>&1 || true
elif [ -x "$REPO_ROOT/dune-ctl/target/release/dune-ctl" ]; then
    "$REPO_ROOT/dune-ctl/target/release/dune-ctl" settings list > "$dest/dune-ctl-settings.txt" 2>&1 || true
    "$REPO_ROOT/dune-ctl/target/release/dune-ctl" diagnostics > "$dest/dune-ctl-diagnostics.txt" 2>&1 || true
fi

echo "Creating compressed metadata archive..."
tar -C "$dest" -czf "$dest/metadata.tar.gz" MANIFEST.txt k8s user-settings dune-ctl-settings.txt dune-ctl-diagnostics.txt 2>/dev/null || true

{
    echo "completed_utc=$(date -u +%Y%m%d-%H%M%S)"
    du -sh "$dest" 2>/dev/null | awk '{print "bundle_size=" $1}'
} >> "$manifest"

echo "Backup complete: $dest"
find "$dest" -maxdepth 2 -type f -printf "%p\n" | sort
