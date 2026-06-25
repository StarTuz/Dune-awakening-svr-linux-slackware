#!/bin/bash
#
# offsite-sync.sh — replicate the local Dune backup bundles to two independent
# off-server restic repositories:
#
#   1. PRIMARY:   restic repo on Backblaze B2 (via rclone b2 backend).
#                 Immutable with bucket Object Lock + Governance retention.
#   2. SECONDARY: restic repo on Google Drive (via rclone drive backend).
#                 Independent vendor; uses existing Google One spend.
#
# Both repos are client-side encrypted by restic with the same master
# passphrase (one secret to escrow), and both are integrity-checkable and
# deduplicated. We use restic for both legs because restic drives rclone via
# its stable `serve` path; the standalone `rclone copy` command had an
# intermittent panic in rclone 1.74.3 that made it unsafe for unattended cron.
#
# Only the durable game-state bundles under /srv/backups/dune/live/<bg>/ are
# replicated. PTC (retired test env), root-owned system-snapshots/, and
# resource-snapshots/ are excluded.
#
# Config lives in ~/.dune/offsite.env (chmod 600, NOT committed). See
# OFFSITE-BACKUP.md for the full runbook, account setup, and key escrow.
#
# Usage:
#   offsite-sync.sh init            # one-time: init every repo
#   offsite-sync.sh run             # back up to every repo (default)
#   offsite-sync.sh check           # integrity-check every repo
#   offsite-sync.sh snapshots       # list snapshots in every repo
#   offsite-sync.sh prune           # apply retention (needs delete rights)
#   offsite-sync.sh --repo <name>   # limit to one repo (substring match)
#   offsite-sync.sh --dry-run run
#
set -u

# restic + rclone live in ~/.local/bin; cron's minimal PATH would miss them, and
# restic shells out to rclone for the b2/drive backends, so ensure it's found.
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) PATH="$HOME/.local/bin:$PATH" ;; esac
export PATH

ENV_FILE="${OFFSITE_ENV_FILE:-$HOME/.dune/offsite.env}"
SRC_ROOT="/srv/backups/dune"
# Live world only. PTC was the retired test environment — not replicated.
ENV_DIRS="live"
LOG_DIR="$HOME/dune-server/logs"
LOG_FILE="$LOG_DIR/offsite-sync.log"

DRY_RUN=0
REPO_FILTER=""

log() { printf '%s  %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$LOG_FILE"; }
die() { log "ERROR: $*"; exit 1; }

retry() {
    # retry <n> <sleep> -- <command...>
    local n=$1 s=$2; shift 2; [ "$1" = "--" ] && shift
    local attempt=1
    while true; do
        "$@" && return 0
        if [ "$attempt" -ge "$n" ]; then
            log "command failed after $attempt attempts: $*"
            return 1
        fi
        log "attempt $attempt failed; retrying in ${s}s: $*"
        sleep "$s"; attempt=$((attempt + 1))
    done
}

load_env() {
    [ -f "$ENV_FILE" ] || die "missing config $ENV_FILE (see OFFSITE-BACKUP.md)"
    # shellcheck disable=SC1090
    set -a; . "$ENV_FILE"; set +a
    : "${RESTIC_PASSWORD_FILE:?set RESTIC_PASSWORD_FILE in $ENV_FILE}"
    : "${RCLONE_CONFIG:?set RCLONE_CONFIG in $ENV_FILE}"
    : "${OFFSITE_REPOS:?set OFFSITE_REPOS (space-separated restic repo URLs) in $ENV_FILE}"
    export RESTIC_PASSWORD_FILE RCLONE_CONFIG
    [ -r "$RESTIC_PASSWORD_FILE" ] || die "restic password file not readable: $RESTIC_PASSWORD_FILE"
    command -v restic >/dev/null 2>&1 || die "restic not found on PATH"
    command -v rclone >/dev/null 2>&1 || die "rclone not found on PATH"
}

selected_repos() {
    local r out=""
    for r in $OFFSITE_REPOS; do
        if [ -z "$REPO_FILTER" ] || [[ "$r" == *"$REPO_FILTER"* ]]; then
            out="$out $r"
        fi
    done
    [ -n "$out" ] || die "no repos match filter '$REPO_FILTER' in OFFSITE_REPOS"
    echo "$out"
}

existing_src_dirs() {
    local d out=""
    for d in $ENV_DIRS; do
        [ -d "$SRC_ROOT/$d" ] && out="$out $SRC_ROOT/$d"
    done
    echo "$out"
}

do_init() {
    local r
    for r in $(selected_repos); do
        log "init repo: $r"
        if restic -r "$r" snapshots >/dev/null 2>&1; then
            log "  already initialized"
        else
            restic -r "$r" init || die "restic init failed for $r"
            log "  initialized"
        fi
    done
}

do_backup() {
    local dirs; dirs=$(existing_src_dirs)
    [ -n "$dirs" ] || die "no source dirs under $SRC_ROOT ($ENV_DIRS)"
    local dry=""; [ "$DRY_RUN" = 1 ] && dry="--dry-run"
    local r
    for r in $(selected_repos); do
        log "backup -> $r : $dirs"
        # shellcheck disable=SC2086
        retry 3 30 -- restic -r "$r" backup $dry --tag offsite --host arrakis $dirs \
            || die "restic backup failed for $r"
        log "  backup complete"
    done
}

do_check() {
    local r
    for r in $(selected_repos); do
        log "check repo: $r"
        restic -r "$r" check || die "restic check reported problems for $r"
        log "  check OK"
    done
}

do_snapshots() {
    local r
    for r in $(selected_repos); do
        log "snapshots: $r"
        restic -r "$r" snapshots
    done
}

do_prune() {
    local r
    for r in $(selected_repos); do
        log "forget+prune (retention): $r — needs delete rights (Object Lock may block)"
        restic -r "$r" forget --prune \
            --keep-daily 14 --keep-weekly 8 --keep-monthly 12 --keep-yearly 3 \
            || die "restic forget/prune failed for $r"
        log "  retention applied"
    done
}

main() {
    mkdir -p "$LOG_DIR"
    local cmd="run" expect_repo=0
    for a in "$@"; do
        if [ "$expect_repo" = 1 ]; then REPO_FILTER="$a"; expect_repo=0; continue; fi
        case "$a" in
            --dry-run) DRY_RUN=1 ;;
            --repo) expect_repo=1 ;;
            init|run|check|snapshots|prune) cmd="$a" ;;
            *) die "unknown argument: $a" ;;
        esac
    done

    load_env
    log "=== offsite-sync: $cmd (dry-run=$DRY_RUN, repo-filter='${REPO_FILTER:-all}') ==="

    case "$cmd" in
        init)      do_init ;;
        run)       do_backup; log "=== offsite-sync complete ===" ;;
        check)     do_check ;;
        snapshots) do_snapshots ;;
        prune)     do_prune ;;
        *) die "unknown command: $cmd" ;;
    esac
}

main "$@"
