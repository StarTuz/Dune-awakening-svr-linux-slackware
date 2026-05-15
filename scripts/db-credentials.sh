#!/bin/bash
set -euo pipefail

BATTLEGROUP_PREFIX="funcom-seabass-"

usage() {
    cat <<EOF
Usage: $0 <check|fix|patch-spec> [--bg NAME]

Checks or repairs the expected Dune Postgres credentials using the live
BattleGroup/DatabaseDeployment specs. The updated operator may expose Postgres
on port 5432 even if older local assumptions expected 15432.

Commands:
  check       Verify both dune and postgres can authenticate.
  fix         ALTER USER dune/postgres back to the BattleGroup spec passwords.
  patch-spec  Patch BattleGroup and DatabaseDeployment specs to expected values.

Options:
  --bg NAME   Battlegroup name without funcom-seabass- prefix.
  --wait SEC  Wait up to SEC seconds for Postgres to accept TCP connections.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" || "$#" -eq 0 ]]; then
    usage
    exit 0
fi

cmd="$1"
shift
bgname=""
wait_timeout=180

while [ "$#" -gt 0 ]; do
    case "$1" in
        --bg)
            bgname="${2:-}"
            shift 2
            ;;
        --wait)
            wait_timeout="${2:-}"
            shift 2
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

case "$cmd" in
    check|fix|patch-spec) ;;
    *)
        usage >&2
        exit 1
        ;;
esac

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

jsonpath_or_default() {
    local path="$1"
    local fallback="$2"
    local value
    value="$(sudo kubectl get battlegroup "$bgname" -n "$ns" -o "jsonpath=$path" 2>/dev/null || true)"
    if [ -n "$value" ]; then
        printf '%s' "$value"
    else
        printf '%s' "$fallback"
    fi
}

kubectl_jsonpath() {
    local resource="$1"
    local path="$2"
    sudo kubectl get "$resource" -n "$ns" -o "jsonpath=$path" 2>/dev/null || true
}

find_db_pod() {
    sudo kubectl get pods -n "$ns" --no-headers -o custom-columns=NAME:.metadata.name \
        | grep -- '-db-dbdepl-sts-' \
        | head -n1
}

find_dbdepl() {
    sudo kubectl get databasedeployments -n "$ns" --no-headers -o custom-columns=NAME:.metadata.name 2>/dev/null \
        | head -n1
}

discover_db_port() {
    local port=""

    if [ -n "$dbdepl" ]; then
        port="$(kubectl_jsonpath "databasedeployment/$dbdepl" '{.spec.port}')"
        if [ -z "$port" ]; then
            port="$(kubectl_jsonpath "databasedeployment/$dbdepl" '{.status.address}' | sed -n 's/.*:\([0-9][0-9]*\)$/\1/p')"
        fi
    fi

    if [ -z "$port" ]; then
        port="$(sudo kubectl get svc -n "$ns" "${bgname}-db-dbdepl-svc" -o jsonpath='{.spec.ports[0].port}' 2>/dev/null || true)"
    fi

    if [ -z "$port" ]; then
        port="$(jsonpath_or_default '{.spec.database.template.spec.deployment.spec.port}' '')"
    fi

    if [ -n "$port" ]; then
        printf '%s' "$port"
    else
        printf '5432'
    fi
}

psql_exec() {
    local user="$1"
    local password="$2"
    local database="$3"
    local sql="$4"

    sudo kubectl exec -n "$ns" "$db_pod" -- env PGPASSWORD="$password" \
        psql -h 127.0.0.1 -p "$db_port" -U "$user" -d "$database" -Atc "$sql" >/dev/null
}

wait_for_db() {
    local timeout="${1:-180}"
    local elapsed=0
    local interval=5

    echo "Waiting for Postgres in $db_pod to accept TCP on port $db_port..."
    while [ "$elapsed" -lt "$timeout" ]; do
        if sudo kubectl exec -n "$ns" "$db_pod" -- sh -c "pg_isready -h 127.0.0.1 -p '$db_port' >/dev/null 2>&1"; then
            echo "Postgres is accepting connections."
            return 0
        fi
        sleep "$interval"
        elapsed=$((elapsed + interval))
        echo "  Still waiting for Postgres... (${elapsed}s / ${timeout}s)"

        local replacement
        replacement="$(find_db_pod || true)"
        if [ -n "$replacement" ] && [ "$replacement" != "$db_pod" ]; then
            db_pod="$replacement"
            echo "Database pod changed; now checking $db_pod"
        fi
    done

    echo "ERROR: timed out waiting for Postgres readiness in $db_pod" >&2
    return 1
}

patch_specs() {
    echo "Patching BattleGroup database credential spec for $bgname..."
    sudo kubectl patch battlegroup "$bgname" -n "$ns" --type=merge -p \
        "{\"spec\":{\"database\":{\"template\":{\"spec\":{\"deployment\":{\"spec\":{\"user\":\"$db_user\",\"password\":\"$db_password\",\"superUser\":\"$super_user\",\"superPassword\":\"$super_password\"}}}}}}}"

    if [ -n "$dbdepl" ]; then
        echo "Patching DatabaseDeployment credential spec $dbdepl..."
        sudo kubectl patch databasedeployment "$dbdepl" -n "$ns" --type=merge -p \
            "{\"spec\":{\"user\":\"$db_user\",\"password\":\"$db_password\",\"superUser\":\"$super_user\",\"superPassword\":\"$super_password\"}}"
    fi
}

fix_passwords() {
    local sql
    sql="ALTER USER \"$db_user\" WITH PASSWORD '$db_password'; ALTER USER \"$super_user\" WITH PASSWORD '$super_password';"

    echo "Repairing Postgres role passwords inside $db_pod..."
    if sudo kubectl exec -n "$ns" "$db_pod" -- env PGPASSWORD="$super_password" \
        psql -h 127.0.0.1 -p "$db_port" -U "$super_user" -d postgres -v ON_ERROR_STOP=1 -c "$sql"; then
        return 0
    fi

    echo "Password login failed; trying local socket as $super_user..."
    if sudo kubectl exec -n "$ns" "$db_pod" -- \
        psql -p "$db_port" -U "$super_user" -d postgres -v ON_ERROR_STOP=1 -c "$sql"; then
        return 0
    fi

    echo "ERROR: unable to repair database passwords automatically." >&2
    echo "Manual equivalent inside the DB pod:" >&2
    echo "  ALTER USER \"$db_user\" WITH PASSWORD '$db_password';" >&2
    echo "  ALTER USER \"$super_user\" WITH PASSWORD '$super_password';" >&2
    return 1
}

select_battlegroup

db_name="$(jsonpath_or_default '{.spec.database.template.spec.deployment.spec.gameDatabaseName}' dune)"
db_user="$(jsonpath_or_default '{.spec.database.template.spec.deployment.spec.user}' dune)"
db_password="$(jsonpath_or_default '{.spec.database.template.spec.deployment.spec.password}' dune)"
super_user="$(jsonpath_or_default '{.spec.database.template.spec.deployment.spec.superUser}' postgres)"
super_password="$(jsonpath_or_default '{.spec.database.template.spec.deployment.spec.superPassword}' postgres)"
dbdepl="$(find_dbdepl)"
db_port="$(discover_db_port)"

db_pod="$(find_db_pod)"
if [ -z "$db_pod" ]; then
    echo "ERROR: database pod not found in $ns" >&2
    exit 1
fi

case "$cmd" in
    patch-spec)
        patch_specs
        ;;
    check)
        echo "Checking DB credentials against $db_pod on port $db_port..."
        wait_for_db "$wait_timeout"
        psql_exec "$super_user" "$super_password" postgres "select 1"
        psql_exec "$db_user" "$db_password" "$db_name" "select 1"
        echo "Database credentials OK."
        ;;
    fix)
        patch_specs
        wait_for_db "$wait_timeout"
        fix_passwords
        echo "Re-checking DB credentials..."
        psql_exec "$super_user" "$super_password" postgres "select 1"
        psql_exec "$db_user" "$db_password" "$db_name" "select 1"
        echo "Database credentials repaired."
        ;;
esac
