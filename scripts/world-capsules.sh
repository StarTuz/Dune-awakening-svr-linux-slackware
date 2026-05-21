#!/bin/bash
set -euo pipefail

BATTLEGROUP_PREFIX="${BATTLEGROUP_PREFIX:-funcom-seabass-}"
BACKUP_ROOT="${BACKUP_ROOT:-/srv/backups/dune}"
DUNE_HOME="${DUNE_HOME:-/home/dune/.dune}"
REPO_ROOT="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/.." && pwd)"

CAPSULE_ROOT="${CAPSULE_ROOT:-$DUNE_HOME/capsules}"
PACKAGE_ROOT_BASE="${PACKAGE_ROOT_BASE:-/home/dune/dune-packages}"
DEFAULT_LIVE_APP_ID="4754530"
DEFAULT_PTC_APP_ID="3104830"

usage() {
    cat <<EOF
Usage: $0 <command> [options]

Non-destructive inventory for Dune world capsules. A capsule is the metadata
needed to cold-swap a self-hosted world without mixing PTC and Live state:
package root, app/build, battlegroup spec, token identity, namespace, services,
PVCs, and backup environment.

Commands:
  inventory                         Print current host/package/world isolation state
  create [options]                  Render a capsule without applying it to Kubernetes
  refresh [options]                 Refresh an existing capsule from its package root
  package install [options]         Download a package with SteamCMD, then validate it
  package validate [options]        Validate an installed package root
  images load [options]             Import package images into k3s/containerd
  activate [options]                Dry-run or apply a rendered capsule
  -h, --help                        Show this help

Create options:
  --env ptc|live                    Capsule environment (default: live)
  --name NAME                       World title; prompts when omitted
  --sietch-name NAME                Sietch name (default: Sietch Abbir)
  --region REGION                   Farm region (live default: North America; PTC default: North America Test)
  --token JWT                       Self-hosting token; prompts when omitted
  --token-file PATH                 Read self-hosting token from a file
  --package-root PATH               Package root containing server/scripts/setup
  --world-id NAME                   Battlegroup id; generated from token when omitted
  --host-ip IP                      Public host IP advertised to FLS
  --force                           Overwrite an existing capsule directory

Refresh options:
  --env ptc|live                    Capsule environment (default: live)
  --world-id NAME                   Capsule battlegroup id
  --package-root PATH               Package root containing updated package images
  --allow-downgrade                 Allow refresh to render an older image tag

Package options:
  --env ptc|live                    Package environment (default: live)
  --app-id ID                       Steam app id (live: 4754530, ptc: 3104830)
  --package-root PATH               Install/validate root
  --steamcmd PATH                   SteamCMD script path

Image options:
  --env ptc|live                    Package environment (default: live)
  --package-root PATH               Package root to import from

Activate options:
  --env ptc|live                    Capsule environment (default: live)
  --world-id NAME                   Capsule battlegroup id
  --apply                           Apply namespace, secrets, and BattleGroup
  --force                           Allow apply while other battlegroups exist
EOF
}

section() {
    printf '\n== %s ==\n' "$1"
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

validate_env() {
    case "$1" in
        ptc|live) ;;
        *) die "environment must be ptc or live (got '$1')" ;;
    esac
}

default_app_id() {
    case "$1" in
        ptc) echo "$DEFAULT_PTC_APP_ID" ;;
        live) echo "$DEFAULT_LIVE_APP_ID" ;;
        *) die "environment must be ptc or live (got '$1')" ;;
    esac
}

default_package_root() {
    local env="$1"
    local app_id="${2:-$(default_app_id "$env")}"
    echo "$PACKAGE_ROOT_BASE/$env/app-$app_id/server"
}

default_region() {
    case "$1" in
        ptc) echo "North America Test" ;;
        live) echo "North America" ;;
        *) die "environment must be ptc or live (got '$1')" ;;
    esac
}

prompt_if_empty() {
    local var_name="$1"
    local prompt="$2"
    local default="${3:-}"
    local current="${!var_name:-}"
    if [ -n "$current" ]; then
        return
    fi
    if [ -n "$default" ]; then
        read -r -p "$prompt [$default]: " current
        current="${current:-$default}"
    else
        read -r -p "$prompt: " current
    fi
    printf -v "$var_name" '%s' "$current"
}

prompt_secret_if_empty() {
    local var_name="$1"
    local prompt="$2"
    local current="${!var_name:-}"
    if [ -n "$current" ]; then
        return
    fi
    read -r -s -p "$prompt: " current
    printf '\n'
    printf -v "$var_name" '%s' "$current"
}

read_secret_file() {
    local file="$1"
    [ -f "$file" ] || die "token file does not exist: $file"
    local value
    value="$(tr -d '\r\n' < "$file")"
    [ -n "$value" ] || die "token file is empty: $file"
    printf '%s' "$value"
}

json_escape() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

sed_escape() {
    printf '%s' "$1" | sed 's/[\/&]/\\&/g'
}

validate_world_title() {
    local value="$1"
    [ -n "$value" ] || die "world name cannot be empty"
    [ "${#value}" -le 50 ] || die "world name must be 50 characters or fewer"
}

validate_sietch_name() {
    local value="$1"
    [ -n "$value" ] || die "sietch name cannot be empty"
    case "$value" in
        *"'"*|*"|"*) die "sietch name cannot contain single quote or pipe" ;;
    esac
}

base64url_decode() {
    local input="$1"
    local len pad
    input="${input//-/+}"
    input="${input//_//}"
    len=$((${#input} % 4))
    case "$len" in
        0) pad="" ;;
        2) pad="==" ;;
        3) pad="=" ;;
        *) return 1 ;;
    esac
    printf '%s%s' "$input" "$pad" | base64 -d 2>/dev/null
}

token_payload_json() {
    local token="$1"
    local payload
    IFS='.' read -r _ payload _ <<< "$token"
    [ -n "${payload:-}" ] || return 1
    base64url_decode "$payload"
}

token_host_id() {
    local token="$1"
    token_payload_json "$token" | jq -r '.HostId // empty' | tr '[:upper:]' '[:lower:]'
}

generate_world_id() {
    local token="$1"
    local host_id suffix
    host_id="$(token_host_id "$token")"
    [ -n "$host_id" ] || die "token does not contain HostId"
    while [ "${#suffix}" -lt 6 ]; do
        suffix="${suffix}$(openssl rand -base64 32 | tr -dc 'a-z' | head -c 6 || true)"
    done
    suffix="${suffix:0:6}"
    echo "sh-$host_id-$suffix"
}

acf_value() {
    local file="$1"
    local key="$2"
    awk -v key="$key" '
        $1 == "\"" key "\"" {
            gsub(/"/, "", $2)
            print $2
            exit
        }
    ' "$file" 2>/dev/null || true
}

acf_name() {
    local file="$1"
    awk '
        $1 == "\"name\"" {
            $1=""
            sub(/^[[:space:]]+"/, "")
            sub(/"$/, "")
            print
            exit
        }
    ' "$file" 2>/dev/null || true
}

print_package_roots() {
    section "Package Roots"
    local found=0
    while IFS= read -r manifest; do
        found=1
        local root appid build target name
        root="$(dirname "$(dirname "$manifest")")"
        appid="$(acf_value "$manifest" appid)"
        build="$(acf_value "$manifest" buildid)"
        target="$(acf_value "$manifest" TargetBuildID)"
        name="$(acf_name "$manifest")"
        printf '%-48s app=%-8s build=%-10s target=%-10s %s\n' \
            "$root" "${appid:-?}" "${build:-?}" "${target:-?}" "${name:-?}"
    done < <(find /home/dune -maxdepth 8 -path '*/steamapps/appmanifest_*.acf' -type f 2>/dev/null | sort)
    if [ "$found" -eq 0 ]; then
        echo "No Steam app manifests found under /home/dune."
    fi
    echo "Expected official self-host app id: 4754530"
    echo "Known PTC app id: 3104830"
}

capsule_value() {
    local file="$1"
    local key="$2"
    awk -F= -v key="$key" '$1 == key {sub(/^[^=]*=/, ""); print; exit}' "$file"
}

resolve_capsule_dir() {
    local env="$1"
    local world_id="$2"
    validate_env "$env"
    [ -n "$world_id" ] || die "--world-id is required"
    local dir="$CAPSULE_ROOT/$env/$world_id"
    [ -d "$dir" ] || die "capsule does not exist: $dir"
    [ -f "$dir/capsule.env" ] || die "capsule metadata missing: $dir/capsule.env"
    [ -f "$dir/battlegroup.yaml" ] || die "capsule battlegroup.yaml missing: $dir"
    [ -f "$dir/fls-secret.yaml" ] || die "capsule fls-secret.yaml missing: $dir"
    [ -f "$dir/rmq-secret.yaml" ] || die "capsule rmq-secret.yaml missing: $dir"
    echo "$dir"
}

package_manifest() {
    local root="$1"
    local app_id="$2"
    echo "$root/steamapps/appmanifest_$app_id.acf"
}

validate_package_root() {
    local env="$1"
    local app_id="$2"
    local root="$3"
    validate_env "$env"

    local manifest
    manifest="$(package_manifest "$root" "$app_id")"
    [ -d "$root" ] || die "package root does not exist: $root"
    [ -f "$manifest" ] || die "missing Steam app manifest: $manifest"
    [ -f "$root/scripts/setup/templates/world-template.yaml" ] || die "missing world template in $root"
    [ -f "$root/scripts/setup/templates/fls-secret.yaml" ] || die "missing fls-secret template in $root"
    [ -f "$root/scripts/setup/templates/rmq-secret.yaml" ] || die "missing rmq-secret template in $root"
    [ -f "$root/images/battlegroup/version.txt" ] || die "missing battlegroup version.txt in $root"
    [ -f "$root/images/operators/version.txt" ] || die "missing operators version.txt in $root"

    for image in \
        images/battlegroup/server.tar \
        images/battlegroup/server-bg-director.tar \
        images/battlegroup/server-db-utils.tar \
        images/battlegroup/server-gateway.tar \
        images/battlegroup/server-rabbitmq.tar \
        images/battlegroup/server-text-router.tar \
        images/operators/battlegroup-operator.tar \
        images/operators/database-operator.tar \
        images/operators/server-operator.tar \
        images/operators/utilities-operator.tar; do
        [ -f "$root/$image" ] || die "missing image tarball: $root/$image"
    done

    local found_app_id build name bg_version op_version
    found_app_id="$(acf_value "$manifest" appid)"
    build="$(acf_value "$manifest" buildid)"
    name="$(acf_name "$manifest")"
    bg_version="$(cat "$root/images/battlegroup/version.txt")"
    op_version="$(cat "$root/images/operators/version.txt")"

    [ "$found_app_id" = "$app_id" ] || die "manifest app id $found_app_id does not match expected $app_id"
    if [ "$env" = "live" ] && [ "$app_id" = "$DEFAULT_PTC_APP_ID" ]; then
        die "live package cannot use PTC app id $DEFAULT_PTC_APP_ID"
    fi

    echo "Package valid:"
    echo "  env=$env"
    echo "  app_id=$app_id"
    echo "  root=$root"
    echo "  steam_name=${name:-?}"
    echo "  steam_build=${build:-?}"
    echo "  battlegroup_image_tag=$bg_version"
    echo "  operator_image_tag=$op_version"
}

resolve_steamcmd() {
    local explicit="$1"
    if [ -n "$explicit" ]; then
        [ -x "$explicit" ] || die "SteamCMD is not executable: $explicit"
        echo "$explicit"
        return
    fi
    for candidate in \
        "/home/dune/steamcmd/steamcmd.sh" \
        "$HOME/steamcmd/steamcmd.sh"; do
        if [ -x "$candidate" ]; then
            echo "$candidate"
            return
        fi
    done
    die "steamcmd.sh not found; pass --steamcmd PATH"
}

package_command() {
    local subcommand="${1:-}"
    shift || true
    local env="live"
    local app_id=""
    local package_root=""
    local steamcmd=""

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --env)
                env="${2:-}"
                shift 2
                ;;
            --app-id)
                app_id="${2:-}"
                shift 2
                ;;
            --package-root)
                package_root="${2:-}"
                shift 2
                ;;
            --steamcmd)
                steamcmd="${2:-}"
                shift 2
                ;;
            *)
                die "unknown package option: $1"
                ;;
        esac
    done

    validate_env "$env"
    app_id="${app_id:-$(default_app_id "$env")}"
    package_root="${package_root:-$(default_package_root "$env" "$app_id")}"

    case "$subcommand" in
        validate)
            validate_package_root "$env" "$app_id" "$package_root"
            ;;
        install)
            steamcmd="$(resolve_steamcmd "$steamcmd")"
            mkdir -p "$package_root"
            echo "Installing Dune self-host package:"
            echo "  env=$env"
            echo "  app_id=$app_id"
            echo "  package_root=$package_root"
            "$steamcmd" +force_install_dir "$package_root" +login anonymous +app_update "$app_id" validate +quit
            validate_package_root "$env" "$app_id" "$package_root"
            ;;
        *)
            die "package command must be install or validate"
            ;;
    esac
}

image_tars() {
    cat <<EOF
images/operators/battlegroup-operator.tar
images/operators/database-operator.tar
images/operators/server-operator.tar
images/operators/utilities-operator.tar
images/battlegroup/server.tar
images/battlegroup/server-bg-director.tar
images/battlegroup/server-db-utils.tar
images/battlegroup/server-gateway.tar
images/battlegroup/server-rabbitmq.tar
images/battlegroup/server-text-router.tar
EOF
}

images_command() {
    local subcommand="${1:-}"
    shift || true
    local env="live"
    local package_root=""
    local app_id=""

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --env)
                env="${2:-}"
                shift 2
                ;;
            --package-root)
                package_root="${2:-}"
                shift 2
                ;;
            --app-id)
                app_id="${2:-}"
                shift 2
                ;;
            *)
                die "unknown images option: $1"
                ;;
        esac
    done

    validate_env "$env"
    app_id="${app_id:-$(default_app_id "$env")}"
    package_root="${package_root:-$(default_package_root "$env" "$app_id")}"
    validate_package_root "$env" "$app_id" "$package_root" >/dev/null

    case "$subcommand" in
        load)
            need_cmd sudo
            echo "Importing Dune package images:"
            echo "  env=$env"
            echo "  package_root=$package_root"
            while IFS= read -r rel; do
                [ -n "$rel" ] || continue
                echo "  import $rel"
                sudo ctr -n k8s.io images import "$package_root/$rel"
            done < <(image_tars)
            echo "Image import complete."
            ;;
        *)
            die "images command must be load"
            ;;
    esac
}

active_battlegroups() {
    sudo kubectl get battlegroups -A --no-headers 2>/dev/null | awk '{print $2}'
}

set_capsule_value() {
    local file="$1"
    local key="$2"
    local value="$3"
    if grep -q "^$key=" "$file"; then
        sed -i "s|^$key=.*|$key=$(sed_escape "$value")|" "$file"
    else
        printf '%s=%s\n' "$key" "$value" >> "$file"
    fi
}

refresh_capsule_manifest() {
    local file="$1"
    local image_tag="$2"
    local host_id="$3"

    [ -f "$file" ] || return 0
    sed -i -E \
        "s#(registry\\.funcom\\.com/funcom/self-hosting/seabass-server(-[a-z-]+)?):[^[:space:]]+#\\1:$(sed_escape "$image_tag")#g" \
        "$file"
    sed -i "/name: HOST_DATACENTER_ID/{n;s/value: .*/value: $(sed_escape "$host_id")/;}" "$file"
}

image_tag_revision() {
    printf '%s\n' "$1" | sed -n 's/^\([0-9][0-9]*\).*/\1/p'
}

current_capsule_image_tag() {
    local file="$1"
    [ -f "$file" ] || return 0
    grep -m1 -Eo 'registry\.funcom\.com/funcom/self-hosting/seabass-server:[^[:space:]]+' "$file" \
        | sed 's/^.*://'
}

refresh_capsule() {
    local env="live"
    local world_id=""
    local package_root=""
    local app_id=""
    local allow_downgrade="false"

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --env)
                env="${2:-}"
                shift 2
                ;;
            --world-id)
                world_id="${2:-}"
                shift 2
                ;;
            --package-root)
                package_root="${2:-}"
                shift 2
                ;;
            --app-id)
                app_id="${2:-}"
                shift 2
                ;;
            --allow-downgrade)
                allow_downgrade="true"
                shift
                ;;
            *)
                die "unknown refresh option: $1"
                ;;
        esac
    done

    validate_env "$env"
    app_id="${app_id:-$(default_app_id "$env")}"

    local dir meta token host_id image_tag manifest steam_build steam_name
    dir="$(resolve_capsule_dir "$env" "$world_id")"
    meta="$dir/capsule.env"
    package_root="${package_root:-$(capsule_value "$meta" package_root)}"
    [ -n "$package_root" ] || die "capsule package_root missing"
    validate_package_root "$env" "$app_id" "$package_root" >/dev/null

    token="$(awk '/ServiceAuthToken=/ {sub(/^.*ServiceAuthToken=/, ""); print; exit}' "$dir/battlegroup.yaml")"
    if [ -z "$token" ]; then
        token="$(awk '/value: eyJ/ {print $2; exit}' "$dir/fls-secret.yaml")"
    fi
    host_id="$(token_host_id "$token")"
    [ -n "$host_id" ] || die "unable to derive token HostId from capsule"

    image_tag="$(cat "$package_root/images/battlegroup/version.txt")"
    local current_tag current_revision image_revision
    current_tag="$(current_capsule_image_tag "$dir/battlegroup.yaml")"
    current_revision="$(image_tag_revision "$current_tag")"
    image_revision="$(image_tag_revision "$image_tag")"
    if [ "$allow_downgrade" != "true" ] \
        && [ -n "$current_revision" ] \
        && [ -n "$image_revision" ] \
        && [ "$current_revision" -gt "$image_revision" ]; then
        die "refusing to refresh $world_id from newer image $current_tag to older package image $image_tag; run package install first or pass --allow-downgrade"
    fi
    manifest="$(package_manifest "$package_root" "$app_id")"
    steam_build="$(acf_value "$manifest" buildid)"
    steam_name="$(acf_name "$manifest")"

    refresh_capsule_manifest "$dir/battlegroup.yaml" "$image_tag" "$host_id"
    refresh_capsule_manifest "$DUNE_HOME/$world_id.yaml" "$image_tag" "$host_id"
    set_capsule_value "$meta" package_root "$package_root"
    set_capsule_value "$meta" steam_app_id "$app_id"
    set_capsule_value "$meta" steam_build "${steam_build:-unknown}"
    set_capsule_value "$meta" steam_name "${steam_name:-unknown}"
    set_capsule_value "$meta" battlegroup_image_tag "$image_tag"
    set_capsule_value "$meta" token_host_id "$host_id"

    echo "Capsule refreshed:"
    echo "  env=$env"
    echo "  world_id=$world_id"
    echo "  package_root=$package_root"
    echo "  steam_build=${steam_build:-unknown}"
    echo "  battlegroup_image_tag=$image_tag"
    echo "  host_datacenter_id=$host_id"
}

activate_capsule() {
    local env="live"
    local world_id=""
    local apply=0
    local force=0

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --env)
                env="${2:-}"
                shift 2
                ;;
            --world-id)
                world_id="${2:-}"
                shift 2
                ;;
            --apply)
                apply=1
                shift
                ;;
            --force)
                force=1
                shift
                ;;
            *)
                die "unknown activate option: $1"
                ;;
        esac
    done

    local dir meta ns title package_root backup_root active_count active_list
    dir="$(resolve_capsule_dir "$env" "$world_id")"
    meta="$dir/capsule.env"
    ns="$(capsule_value "$meta" namespace)"
    title="$(capsule_value "$meta" world_title)"
    package_root="$(capsule_value "$meta" package_root)"
    backup_root="$(capsule_value "$meta" backup_root)"

    [ -n "$ns" ] || die "capsule namespace missing"
    [ -n "$package_root" ] || die "capsule package_root missing"
    [ -d "$package_root" ] || die "capsule package_root does not exist: $package_root"

    active_list="$(active_battlegroups || true)"
    active_count="$(printf '%s\n' "$active_list" | awk 'NF {count++} END {print count+0}')"

    echo "Activation plan:"
    echo "  env=$env"
    echo "  world_id=$world_id"
    echo "  namespace=$ns"
    echo "  world_title=$title"
    echo "  package_root=$package_root"
    echo "  backup_root=$backup_root"
    echo "  capsule=$dir"
    if [ "$active_count" -gt 0 ]; then
        echo "  existing_battlegroups=$(printf '%s' "$active_list" | tr '\n' ' ')"
    else
        echo "  existing_battlegroups=none"
    fi

    if [ "$apply" -ne 1 ]; then
        echo
        echo "Dry run only. Re-run with --apply after final backup/park of the active world."
        return
    fi

    if [ "$active_count" -gt 0 ] && [ "$force" -ne 1 ]; then
        die "refusing to apply while battlegroups exist; stop/park active world first or pass --force"
    fi

    ln -sfn "$package_root" "$DUNE_HOME/download"
    cp "$dir/battlegroup.yaml" "$DUNE_HOME/$world_id.yaml"
    cp "$dir/fls-secret.yaml" "$DUNE_HOME/$world_id-fls-secret.yaml"
    cp "$dir/rmq-secret.yaml" "$DUNE_HOME/$world_id-rmq-secret.yaml"
    chmod 600 "$DUNE_HOME/$world_id-fls-secret.yaml" "$DUNE_HOME/$world_id-rmq-secret.yaml"

    sudo kubectl get ns "$ns" >/dev/null 2>&1 || sudo kubectl create ns "$ns"
    sudo kubectl apply -n "$ns" -f "$dir/fls-secret.yaml"
    sudo kubectl apply -n "$ns" -f "$dir/rmq-secret.yaml"
    sudo kubectl apply -n "$ns" -f "$dir/battlegroup.yaml"
    echo "Capsule applied. Watch with: sudo kubectl get battlegroups -A"
}

copy_user_settings() {
    local source_root="$1"
    local dest_dir="$2"
    local sietch_name="$3"
    mkdir -p "$dest_dir"
    cp "$source_root/scripts/setup/config/UserEngine.ini" "$dest_dir/UserEngine.ini"
    cp "$source_root/scripts/setup/config/UserGame.ini" "$dest_dir/UserGame.ini"

    local escaped
    escaped="$(json_escape "$sietch_name")"
    if grep -q '^;*Bgd\.ServerDisplayName=' "$dest_dir/UserEngine.ini"; then
        sed -i "s/^;*Bgd\\.ServerDisplayName=.*/Bgd.ServerDisplayName=\"$escaped\"/" "$dest_dir/UserEngine.ini"
    else
        printf '\nBgd.ServerDisplayName="%s"\n' "$escaped" >> "$dest_dir/UserEngine.ini"
    fi
}

render_template_file() {
    local src="$1"
    local dst="$2"
    local world_name="$3"
    local world_id="$4"
    local region="$5"
    local image_tag="$6"
    local token="$7"
    local rmq_secret="$8"
    local postgres_pass="$9"
    local dune_pass="${10}"
    local host_ip="${11}"
    local host_id="${12}"

    cp "$src" "$dst"
    sed -i \
        -e "s/{WORLD_NAME}/$(sed_escape "$world_name")/g" \
        -e "s/{WORLD_UNIQUE_NAME}/$(sed_escape "$world_id")/g" \
        -e "s/{WORLD_REGION}/$(sed_escape "$region")/g" \
        -e "s/{WORLD_IMAGE_TAG}/$(sed_escape "$image_tag")/g" \
        -e "s/{WORLD_POSTGRES_PASS}/$(sed_escape "$postgres_pass")/g" \
        -e "s/{WORLD_DUNE_PASS}/$(sed_escape "$dune_pass")/g" \
        -e "s/{FLS_SECRET}/$(sed_escape "$token")/g" \
        -e "s|{RMQ_SECRET}|$(printf '%s' "$rmq_secret" | sed 's/[|&]/\\&/g')|g" \
        "$dst"
    if [ -n "$host_ip" ]; then
        sed -i -e "s/value: 127\\.0\\.0\\.1/value: $(sed_escape "$host_ip")/g" "$dst"
    fi
    if [ -n "$host_id" ]; then
        sed -i "/name: HOST_DATACENTER_ID/{n;s/value: .*/value: $(sed_escape "$host_id")/;}" "$dst"
    fi
}

create_capsule() {
    need_cmd jq
    need_cmd openssl

    local env="live"
    local world_name=""
    local sietch_name="Sietch Abbir"
    local region=""
    local token=""
    local token_file=""
    local package_root=""
    local world_id=""
    local host_ip="${HOST_DATACENTER_IP_ADDRESS:-}"
    local force=0

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --env)
                env="${2:-}"
                shift 2
                ;;
            --name)
                world_name="${2:-}"
                shift 2
                ;;
            --sietch-name)
                sietch_name="${2:-}"
                shift 2
                ;;
            --region)
                region="${2:-}"
                shift 2
                ;;
            --token)
                token="${2:-}"
                shift 2
                ;;
            --token-file)
                token_file="${2:-}"
                shift 2
                ;;
            --package-root)
                package_root="${2:-}"
                shift 2
                ;;
            --world-id)
                world_id="${2:-}"
                shift 2
                ;;
            --host-ip)
                host_ip="${2:-}"
                shift 2
                ;;
            --force)
                force=1
                shift
                ;;
            *)
                die "unknown create option: $1"
                ;;
        esac
    done

    validate_env "$env"
    if [ -n "$token_file" ]; then
        [ -z "$token" ] || die "use either --token or --token-file, not both"
        token="$(read_secret_file "$token_file")"
    fi
    token="${token:-${DUNE_FLS_TOKEN:-}}"
    prompt_if_empty world_name "World name"
    prompt_if_empty sietch_name "Sietch name" "Sietch Abbir"
    prompt_if_empty region "Region" "$(default_region "$env")"
    prompt_secret_if_empty token "Self-host token"

    validate_world_title "$world_name"
    validate_sietch_name "$sietch_name"
    host_ip="${host_ip:-127.0.0.1}"

    package_root="${package_root:-$(default_package_root "$env")}"
    [ -d "$package_root" ] || die "package root does not exist: $package_root"
    [ -f "$package_root/scripts/setup/templates/world-template.yaml" ] || die "missing world template in $package_root"
    [ -f "$package_root/images/battlegroup/version.txt" ] || die "missing battlegroup version.txt in $package_root"

    local host_id image_tag capsule_dir ns rmq_secret postgres_pass dune_pass app_id manifest steam_build steam_name created
    host_id="$(token_host_id "$token")"
    [ -n "$host_id" ] || die "token does not contain HostId"
    world_id="${world_id:-$(generate_world_id "$token")}"
    ns="$BATTLEGROUP_PREFIX$world_id"
    image_tag="$(cat "$package_root/images/battlegroup/version.txt")"
    app_id="$(default_app_id "$env")"
    manifest="$(package_manifest "$package_root" "$app_id")"
    if [ -f "$manifest" ]; then
        steam_build="$(acf_value "$manifest" buildid)"
        steam_name="$(acf_name "$manifest")"
    else
        steam_build=""
        steam_name=""
    fi
    if [ "$env" = "live" ] && [ -f "$package_root/steamapps/appmanifest_$DEFAULT_PTC_APP_ID.acf" ]; then
        die "refusing to create live capsule from PTC package root: $package_root"
    fi

    capsule_dir="$CAPSULE_ROOT/$env/$world_id"
    if [ -e "$capsule_dir" ] && [ "$force" -ne 1 ]; then
        die "capsule already exists: $capsule_dir (use --force to overwrite)"
    fi
    rm -rf "$capsule_dir"
    mkdir -p "$capsule_dir"

    rmq_secret="$(openssl rand 64 | base64 -w 0)"
    postgres_pass="$(openssl rand -base64 32 | tr -d '=+/' | cut -c1-24)"
    dune_pass="$(openssl rand -base64 32 | tr -d '=+/' | cut -c1-24)"
    created="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

    render_template_file \
        "$package_root/scripts/setup/templates/world-template.yaml" \
        "$capsule_dir/battlegroup.yaml" \
        "$world_name" "$world_id" "$region" "$image_tag" "$token" "$rmq_secret" "$postgres_pass" "$dune_pass" "$host_ip" "$host_id"
    render_template_file \
        "$package_root/scripts/setup/templates/fls-secret.yaml" \
        "$capsule_dir/fls-secret.yaml" \
        "$world_name" "$world_id" "$region" "$image_tag" "$token" "$rmq_secret" "$postgres_pass" "$dune_pass" "$host_ip" "$host_id"
    render_template_file \
        "$package_root/scripts/setup/templates/rmq-secret.yaml" \
        "$capsule_dir/rmq-secret.yaml" \
        "$world_name" "$world_id" "$region" "$image_tag" "$token" "$rmq_secret" "$postgres_pass" "$dune_pass" "$host_ip" "$host_id"

    copy_user_settings "$package_root" "$capsule_dir/UserSettings" "$sietch_name"
    ln -sfn "$package_root" "$capsule_dir/package-root"

    cat > "$capsule_dir/capsule.env" <<EOF
environment=$env
world_id=$world_id
namespace=$ns
world_title=$world_name
sietch_name=$sietch_name
region=$region
token_host_id=$host_id
package_root=$package_root
steam_app_id=$app_id
steam_build=${steam_build:-unknown}
steam_name=${steam_name:-unknown}
battlegroup_image_tag=$image_tag
host_ip=$host_ip
backup_root=$BACKUP_ROOT/$env/$world_id
created_utc=$created
EOF

    chmod 700 "$capsule_dir"
    chmod 600 "$capsule_dir/fls-secret.yaml" "$capsule_dir/rmq-secret.yaml" "$capsule_dir/capsule.env"

    echo "Capsule rendered:"
    echo "  env=$env"
    echo "  world_id=$world_id"
    echo "  namespace=$ns"
    echo "  world_title=$world_name"
    echo "  sietch_name=$sietch_name"
    echo "  package_root=$package_root"
    echo "  path=$capsule_dir"
    echo
    echo "No Kubernetes resources were applied."
}

print_world_specs() {
    section "World Specs"
    local found=0
    while IFS= read -r spec; do
        found=1
        local bg title env ns token_hint
        bg="$(basename "$spec" .yaml)"
        title="$(awk -F': *' '$1 == "  title" || $1 == "title" {print $2; exit}' "$spec" | tr -d '"')"
        env="$(awk -F': *' '
            $1 == "backup_environment" || $1 == "backupEnvironment" || $1 == "dune-ctl.algieba.org/backup-environment" {
                gsub(/"/, "", $2)
                print tolower($2)
                exit
            }
        ' "$spec")"
        if [ -z "$env" ]; then
            if [ "$bg" = "sh-db3533a2d5a25fb-xyyxbx" ]; then
                env="ptc(default)"
            else
                env="live(default)"
            fi
        fi
        ns="$BATTLEGROUP_PREFIX$bg"
        token_hint="$(awk '
            /ServiceAuthToken=/ {
                sub(/^.*ServiceAuthToken=/, "")
                print substr($0, 1, 24) "..."
                exit
            }
        ' "$spec")"
        printf '%-34s env=%-13s ns=%-48s title=%s token=%s\n' \
            "$bg" "$env" "$ns" "${title:-?}" "${token_hint:-?}"
    done < <(find "$DUNE_HOME" -maxdepth 1 -name '*.yaml' \
        ! -name '*-secret.yaml' ! -name '*-rmq-secret.yaml' ! -name '*-fls-secret.yaml' ! -name '*-dump-*.yaml' \
        -type f 2>/dev/null | sort)
    if [ "$found" -eq 0 ]; then
        echo "No world specs found in $DUNE_HOME."
    fi
}

print_capsules() {
    section "Capsules"
    local found=0
    while IFS= read -r meta; do
        found=1
        local dir env world_id ns title package_root backup_root image_tag
        dir="$(dirname "$meta")"
        env="$(capsule_value "$meta" environment)"
        world_id="$(capsule_value "$meta" world_id)"
        ns="$(capsule_value "$meta" namespace)"
        title="$(capsule_value "$meta" world_title)"
        package_root="$(capsule_value "$meta" package_root)"
        backup_root="$(capsule_value "$meta" backup_root)"
        image_tag="$(capsule_value "$meta" battlegroup_image_tag)"
        printf '%-6s %-34s ns=%-48s image=%-20s title=%s\n' \
            "${env:-?}" "${world_id:-?}" "${ns:-?}" "${image_tag:-?}" "${title:-?}"
        printf '       package=%s backup=%s path=%s\n' "${package_root:-?}" "${backup_root:-?}" "$dir"
    done < <(find "$CAPSULE_ROOT" -mindepth 3 -maxdepth 3 -name capsule.env -type f 2>/dev/null | sort)
    if [ "$found" -eq 0 ]; then
        echo "No capsules found in $CAPSULE_ROOT."
    fi
}

print_kubernetes_state() {
    section "Kubernetes Battlegroups"
    if ! sudo kubectl get battlegroups -A -o wide 2>/dev/null; then
        echo "kubectl battlegroup inventory unavailable."
    fi

    section "Kubernetes Services / NodePorts"
    if ! sudo kubectl get svc -A -o wide 2>/dev/null | awk '
        NR == 1 || $1 ~ /^funcom-seabass-/ || $1 == "kube-system" && $2 == "traefik" {print}
    '; then
        echo "kubectl service inventory unavailable."
    fi

    section "Kubernetes PVCs"
    if ! sudo kubectl get pvc -A 2>/dev/null | awk 'NR == 1 || $1 ~ /^funcom-seabass-/ {print}'; then
        echo "kubectl pvc inventory unavailable."
    fi
}

print_loaded_images() {
    section "Loaded Dune Images"
    if ! sudo ctr -n k8s.io images ls -q 2>/dev/null \
        | grep -E 'seabass|igw-k8s|igw-postgres' \
        | sort; then
        echo "No Dune images found or containerd inventory unavailable."
    fi
}

print_backup_buckets() {
    section "Backup Buckets"
    if [ ! -d "$BACKUP_ROOT" ]; then
        echo "No backup root at $BACKUP_ROOT."
        return
    fi
    (find "$BACKUP_ROOT" -maxdepth 3 -type d 2>/dev/null || true) \
        | sort \
        | awk -v root="$BACKUP_ROOT" '
            $0 == root {next}
            {
                rel=$0
                sub("^" root "/", "", rel)
                depth=gsub("/", "/", rel)
                if (depth <= 2) print $0
            }
        '
}

print_assessment() {
    section "Assessment"
    cat <<EOF
- Namespaces isolate battlegroup resources, secrets, DB PVCs, and server PVCs.
- CRDs and Funcom operators are cluster-global; different PTC/Live operator or CRD versions cannot be fully isolated inside one k3s cluster.
- The stock world template pins the public game RabbitMQ NodePort to 31982. Multiple running worlds need distinct NodePorts or cold swapping.
- Current safe model: one active world, other worlds parked as capsules: package root + world spec + secrets + backups + optional exported namespace evidence.
- PTC and Live DB data must remain separate. Use backup environment markers and restore guards; do not import PTC bundles into Live.
EOF
}

inventory() {
    print_package_roots
    print_capsules
    print_world_specs
    print_kubernetes_state
    print_loaded_images
    print_backup_buckets
    print_assessment
}

case "${1:-}" in
    inventory)
        inventory
        ;;
    create)
        shift
        create_capsule "$@"
        ;;
    refresh)
        shift
        refresh_capsule "$@"
        ;;
    package)
        shift
        package_command "$@"
        ;;
    images)
        shift
        images_command "$@"
        ;;
    activate)
        shift
        activate_capsule "$@"
        ;;
    -h|--help|"")
        usage
        ;;
    *)
        echo "Unknown command: $1" >&2
        usage >&2
        exit 1
        ;;
esac
