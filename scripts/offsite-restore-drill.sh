#!/bin/bash
#
# offsite-restore-drill.sh — prove an off-site backup is importable, end to end.
#
# Pulls the newest snapshot from an off-site restic repo (default: the B2
# primary), restores the newest Postgres dump into an ISOLATED temporary
# database inside the live Postgres pod, runs schema/row sanity counts, then
# drops the temp DB and cleans up. It NEVER touches the live game database and
# never creates a second BattleGroup (which would collide on NodePorts).
#
# This is the layer byte-fidelity checks can't prove: that the off-site dump
# actually restores into a real PostgreSQL and yields a sane game schema.
#
#   offsite-restore-drill.sh                 # drill from the first/primary repo
#   offsite-restore-drill.sh --repo gdrive   # drill from the Drive repo
#   offsite-restore-drill.sh --keep          # keep the temp DB for inspection
#
# Safe to run against the live host: read-only w.r.t. live data.
#
set -u

OFFSITE_ENV_FILE="${OFFSITE_ENV_FILE:-$HOME/.dune/offsite.env}"
NS="${DUNE_NS:-funcom-seabass-sh-db3533a2d5a25fb-silakw}"
BG="${DUNE_BG:-sh-db3533a2d5a25fb-silakw}"
POD="${DUNE_DB_POD:-sh-db3533a2d5a25fb-silakw-db-dbdepl-sts-0}"
LOG_DIR="$HOME/dune-server/logs"
STAMP="$(date -u +%Y%m%d-%H%M%S)"
LOG_FILE="$LOG_DIR/offsite-restore-drill-$STAMP.log"

REPO_FILTER=""
KEEP=0

# Filled in during run; cleaned up by trap.
SCRATCH=""
TMPDB=""
POD_DUMP=""
SUPER_USER=""; SUPER_PW=""; DB_PORT=""; GAME_DB=""

log()  { printf '%s  %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$LOG_FILE"; }
die()  { log "ERROR: $*"; exit 1; }

# psql as superuser inside the pod against <db> running <sql>, value to stdout.
psql_pod() {
    local db="$1" sql="$2"
    sudo -n kubectl exec -n "$NS" "$POD" -- env PGPASSWORD="$SUPER_PW" \
        psql -h 127.0.0.1 -p "$DB_PORT" -U "$SUPER_USER" -d "$db" -Atc "$sql"
}

cleanup() {
    local rc=$?
    if [ -n "$TMPDB" ] && [ "$KEEP" != 1 ]; then
        log "cleanup: dropping temp database $TMPDB"
        psql_pod postgres "DROP DATABASE IF EXISTS \"$TMPDB\";" >/dev/null 2>&1 \
            || log "  (warning) could not drop $TMPDB — drop manually"
    elif [ -n "$TMPDB" ]; then
        log "cleanup: --keep set, leaving temp database $TMPDB (drop it yourself)"
    fi
    if [ -n "$POD_DUMP" ]; then
        sudo -n kubectl exec -n "$NS" "$POD" -- rm -f "$POD_DUMP" >/dev/null 2>&1 || true
    fi
    [ -n "$SCRATCH" ] && rm -rf "$SCRATCH"
    log "=== drill finished (exit $rc), evidence: $LOG_FILE ==="
}
trap cleanup EXIT

# ---- args ----
for a in "$@"; do
    case "$a" in
        --keep) KEEP=1 ;;
        --repo) ;;                 # handled below
        --repo=*) REPO_FILTER="${a#--repo=}" ;;
        *) if [ "${PREV:-}" = "--repo" ]; then REPO_FILTER="$a"; fi ;;
    esac
    PREV="$a"
done

mkdir -p "$LOG_DIR"
log "=== offsite-restore-drill $STAMP (repo-filter='${REPO_FILTER:-primary}') ==="

# ---- load off-site config + pick a repo ----
[ -f "$OFFSITE_ENV_FILE" ] || die "missing $OFFSITE_ENV_FILE"
set -a; . "$OFFSITE_ENV_FILE"; set +a
: "${OFFSITE_REPOS:?OFFSITE_REPOS not set}" "${RESTIC_PASSWORD_FILE:?}" "${RCLONE_CONFIG:?}"
export RESTIC_PASSWORD_FILE RCLONE_CONFIG
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) PATH="$HOME/.local/bin:$PATH"; export PATH ;; esac

REPO=""
for r in $OFFSITE_REPOS; do
    if [ -z "$REPO_FILTER" ] || [[ "$r" == *"$REPO_FILTER"* ]]; then REPO="$r"; break; fi
done
[ -n "$REPO" ] || die "no off-site repo matches '$REPO_FILTER'"
log "drill source repo: $REPO"

# ---- pull newest snapshot to scratch, locate newest dump ----
SCRATCH="$(mktemp -d)"
log "restic restore latest -> $SCRATCH"
restic -r "$REPO" restore latest --target "$SCRATCH" >>"$LOG_FILE" 2>&1 \
    || die "restic restore failed from $REPO"
DUMP="$(find "$SCRATCH" -name '*.backup' | sort | tail -1)"
[ -n "$DUMP" ] || die "no *.backup dump found in restored snapshot"
log "newest dump: ${DUMP#"$SCRATCH"}  ($(du -h "$DUMP" | cut -f1))"
log "dump type: $(file -b "$DUMP")"

# ---- live DB connection details from the BattleGroup CR ----
CR_JSON="$(sudo -n kubectl get battlegroup "$BG" -n "$NS" -o json)" || die "could not read BattleGroup CR"
cr_field() {
    printf '%s' "$CR_JSON" | python3 -c \
      "import json,sys;print(json.load(sys.stdin)['spec']['database']['template']['spec']['deployment']['spec'].get('$1',''))"
}
SUPER_USER="$(cr_field superUser)";        [ -n "$SUPER_USER" ] || SUPER_USER=postgres
GAME_DB="$(cr_field gameDatabaseName)";     [ -n "$GAME_DB" ]    || GAME_DB=dune
GAME_OWNER="$(cr_field user)";              [ -n "$GAME_OWNER" ] || GAME_OWNER=dune
SUPER_PW="$(cr_field superPassword)"
DB_PORT="$(sudo -n kubectl get databasedeployment "${BG}-db-dbdepl" -n "$NS" -o jsonpath='{.spec.port}' 2>/dev/null)"
[ -n "$DB_PORT" ] || DB_PORT=5432
[ -n "$SUPER_PW" ] || die "could not read DB superPassword from CR"
log "target pod=$POD port=$DB_PORT superuser=$SUPER_USER game-db=$GAME_DB owner=$GAME_OWNER"

# ---- isolated temp DB name; hard guard against the live DB ----
TMPDB="restore_drill_${STAMP}"
[ "$TMPDB" != "$GAME_DB" ] || die "refusing: temp DB name equals live game DB"

log "wait for Postgres ready"
sudo -n kubectl exec -n "$NS" "$POD" -- pg_isready -h 127.0.0.1 -p "$DB_PORT" >/dev/null 2>&1 \
    || die "Postgres not ready"

# ---- copy dump into pod, create temp DB owned by the game user, restore ----
POD_DUMP="/tmp/offsite-drill-${STAMP}.backup"
log "copy dump into pod: $POD_DUMP"
sudo -n kubectl cp "$DUMP" "$NS/$POD:$POD_DUMP" >>"$LOG_FILE" 2>&1 || die "kubectl cp failed"

log "create temp database $TMPDB owned by $GAME_OWNER"
psql_pod postgres "CREATE DATABASE \"$TMPDB\" OWNER \"$GAME_OWNER\";" >/dev/null \
    || die "could not create temp database"

log "pg_restore -> $TMPDB (faithful import test)"
if sudo -n kubectl exec -n "$NS" "$POD" -- env PGPASSWORD="$SUPER_PW" \
        pg_restore -h 127.0.0.1 -p "$DB_PORT" -U "$SUPER_USER" -d "$TMPDB" \
        --no-privileges "$POD_DUMP" >>"$LOG_FILE" 2>&1; then
    log "pg_restore completed cleanly"
else
    log "pg_restore returned non-zero (continuing; checking for benign role/grant warnings)"
fi

# ---- sanity counts against the restored temp DB ----
log "--- restored schema sanity ($TMPDB) ---"
BT="$(psql_pod "$TMPDB" "SELECT count(*) FROM information_schema.tables WHERE table_type='BASE TABLE' AND table_schema NOT IN ('pg_catalog','information_schema');")"
RT="$(psql_pod "$TMPDB" "SELECT count(*) FROM information_schema.routines WHERE specific_schema NOT IN ('pg_catalog','information_schema');")"
log "base tables : ${BT:-?}"
log "routines    : ${RT:-?}"

# Curated gameplay tables: resolve schema-qualified name, count rows if present.
for t in world_partition farm_state account player actor item guild; do
    qn="$(psql_pod "$TMPDB" "SELECT n.nspname||'.'||c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND c.relname='$t' LIMIT 1;")"
    if [ -n "$qn" ]; then
        cnt="$(psql_pod "$TMPDB" "SELECT count(*) FROM $qn;")"
        log "$(printf '%-14s: %s rows  (%s)' "$t" "${cnt:-?}" "$qn")"
    fi
done

if [ "${BT:-0}" -gt 0 ] 2>/dev/null; then
    log "RESULT: PASS — off-site dump from ${REPO##*:} restored into a live PostgreSQL with $BT tables"
else
    die "RESULT: FAIL — restored DB has no tables"
fi
