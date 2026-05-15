#!/bin/bash
# Local security exposure checks for the Dune host.

set -euo pipefail

BATTLEGROUP_PREFIX="funcom-seabass-"

warn=0
critical=0

section() {
    echo ""
    echo "=== $* ==="
}

ok() {
    echo "OK   $*"
}

info() {
    echo "INFO $*"
}

warning() {
    warn=$((warn + 1))
    echo "WARN $*"
}

fail() {
    critical=$((critical + 1))
    echo "CRIT $*"
}

select_namespace() {
    sudo kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name 2>/dev/null \
        | grep "^$BATTLEGROUP_PREFIX" \
        | head -n1
}

firewall_public_ports() {
    local services ports service

    services="$(firewall-cmd --zone=public --list-services 2>/dev/null || true)"
    ports="$(firewall-cmd --zone=public --list-ports 2>/dev/null || true)"

    for service in $services; do
        firewall-cmd --info-service="$service" 2>/dev/null \
            | sed -n 's/^  ports: //p' \
            | tr ' ' '\n'
    done

    printf '%s\n' "$ports" | tr ' ' '\n'
}

port_spec_allows() {
    local query_port="$1"
    local query_proto="$2"
    local spec port proto start end

    for spec in "${public_port_specs[@]}"; do
        [ -n "$spec" ] || continue
        port="${spec%/*}"
        proto="${spec#*/}"
        [ "$proto" = "$query_proto" ] || continue

        if [[ "$port" == *-* ]]; then
            start="${port%-*}"
            end="${port#*-}"
            if [ "$query_port" -ge "$start" ] && [ "$query_port" -le "$end" ]; then
                return 0
            fi
        elif [ "$query_port" = "$port" ]; then
            return 0
        fi
    done

    return 1
}

collect_nodeports() {
    sudo kubectl get svc -n "$ns" -o json 2>/dev/null \
        | jq -r '
            .items[]
            | select(.spec.type == "NodePort")
            | .metadata.name as $name
            | .spec.ports[]
            | [$name, (.name // "-"), (.protocol // "TCP"), (.port|tostring), (.nodePort|tostring)]
            | @tsv
        '
}

classify_sensitive_service() {
    local service="$1"
    local port_name="$2"
    local target_port="$3"
    local node_port="$4"

    case "$service:$port_name:$target_port:$node_port" in
        *bgd*|*director*|*filebrowser*|*file-browser*|*db*|*pghero*|*mon*)
            return 0
            ;;
        *mq-admin*|*:15672:*|*:9187:*|*:11717:*|*:8888:*|*:18888:*)
            return 0
            ;;
    esac

    return 1
}

ns="$(select_namespace || true)"
if [ -z "$ns" ]; then
    echo "ERROR: no battlegroup namespace found" >&2
    exit 1
fi

section "Target"
echo "Namespace: $ns"
echo "Battlegroup: ${ns#$BATTLEGROUP_PREFIX}"

section "firewalld public zone"
backend="$(sed -n 's/^FirewallBackend=//p' /etc/firewalld/firewalld.conf 2>/dev/null || true)"
if [ "$backend" = "iptables" ]; then
    ok "FirewallBackend=iptables"
else
    warning "FirewallBackend=${backend:-unknown}; expected iptables on this k3s host"
fi

nft_tables="$(/usr/sbin/nft list tables 2>/dev/null || true)"
if [ "$backend" = "iptables" ] && printf '%s\n' "$nft_tables" | grep -qx 'table inet firewalld'; then
    fail "stale nft table inet firewalld exists while firewalld backend is iptables"
elif [ "$backend" = "iptables" ]; then
    ok "no stale nft table inet firewalld"
fi

mapfile -t public_port_specs < <(firewall_public_ports | sed '/^$/d' | sort -u)
echo "Public zone effective ports:"
if [ "${#public_port_specs[@]}" -eq 0 ]; then
    echo "  none"
else
    printf '  %s\n' "${public_port_specs[@]}"
fi

section "sensitive Kubernetes NodePorts"
nodeport_rows="$(collect_nodeports || true)"
if [ -z "$nodeport_rows" ]; then
    info "No NodePort services found in $ns"
else
    printf '%-54s %-14s %-6s %-8s %-8s %-10s\n' "Service" "Name" "Proto" "Port" "NodePort" "Public"
    while IFS=$'\t' read -r service port_name proto target_port node_port; do
        [ -n "$service" ] || continue
        public="no"
        if port_spec_allows "$node_port" "${proto,,}"; then
            public="yes"
        fi

        printf '%-54s %-14s %-6s %-8s %-8s %-10s\n' "$service" "$port_name" "$proto" "$target_port" "$node_port" "$public"

        if classify_sensitive_service "$service" "$port_name" "$target_port" "$node_port"; then
            if [ "$public" = "yes" ]; then
                fail "$service nodePort $node_port/$proto is sensitive and allowed by firewalld public zone"
            else
                ok "$service nodePort $node_port/$proto is not allowed by firewalld public zone"
            fi
        fi
    done <<< "$nodeport_rows"
fi

section "well-known sensitive public ports"
for spec in \
    "11717/tcp:director API" \
    "30299/tcp:observed director NodePort" \
    "8888/tcp:filebrowser" \
    "18888/tcp:filebrowser observed LAN URL" \
    "5432/tcp:postgres" \
    "6443/tcp:k3s API" \
    "15672/tcp:rabbitmq management" \
    "31241/tcp:observed RMQ admin management NodePort" \
    "31958/tcp:observed RMQ admin AMQP NodePort"; do
    port_proto="${spec%%:*}"
    label="${spec#*:}"
    port="${port_proto%/*}"
    proto="${port_proto#*/}"
    if port_spec_allows "$port" "$proto"; then
        fail "$label is allowed by firewalld public zone on $port_proto"
    else
        ok "$label is not allowed by firewalld public zone on $port_proto"
    fi
done

section "router reminder"
info "This host can verify firewalld, not Frontier router forwards. Router should expose only Dune UDP 7782-7790 and RMQ game TCP 31982/30196 unless intentionally changed."

section "summary"
if [ "$critical" -gt 0 ]; then
    echo "Security audit FAILED: $critical critical, $warn warning."
    exit 2
fi
if [ "$warn" -gt 0 ]; then
    echo "Security audit completed with warnings: $warn warning."
    exit 1
fi
echo "Security audit OK."
