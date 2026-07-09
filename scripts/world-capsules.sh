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
  images verify [options]           Verify package images are registered in k3s/containerd
  activate [options]                Dry-run or apply a rendered capsule
  park [options]                    Dry-run or apply a park of the active world
  swap [options]                    Hot-swap the active Live world for another capsule
  restore [options]                 Sudo-safe restore of a backup bundle into a stopped world
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
  --app-id ID                       Steam app id

Activate options:
  --env ptc|live                    Capsule environment (default: live)
  --world-id NAME                   Capsule battlegroup id
  --apply                           Apply namespace, secrets, and BattleGroup
  --force                           Allow apply while other battlegroups exist

Park options:
  --env ptc|live                    Capsule environment (default: live)
  --world-id NAME                   Battlegroup id of the world to park
  --apply                           Stop, back up, and delete the namespace
  --skip-backup                     Skip the final backup (not recommended)

Swap options:
  --env ptc|live                    Capsule environment (default: live)
  --to NAME                         Target capsule battlegroup id to activate
  --apply                           Park the active world and activate the target
  --skip-backup                     Skip the parked world's final backup
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

sudo_capsule() {
    if [ "${DUNE_CAPSULE_SUDO_INTERACTIVE:-0}" = "1" ]; then
        sudo "$@"
    else
        sudo -n "$@"
    fi
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
    local host_id suffix=""
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
    local package_root="$1"
    (cd "$package_root" && find images -type f -name '*.tar' | sort)
}

expected_package_images() {
    local package_root="$1"
    local bg_version op_version
    bg_version="$(cat "$package_root/images/battlegroup/version.txt")"
    op_version="$(cat "$package_root/images/operators/version.txt")"
    cat <<EOF
registry.funcom.com/funcom/self-hosting/seabass-server:$bg_version
registry.funcom.com/funcom/self-hosting/seabass-server-bg-director:$bg_version
registry.funcom.com/funcom/self-hosting/seabass-server-db-utils:$bg_version
registry.funcom.com/funcom/self-hosting/seabass-server-gateway:$bg_version
registry.funcom.com/funcom/self-hosting/seabass-server-rabbitmq:$bg_version
registry.funcom.com/funcom/self-hosting/seabass-server-text-router:$bg_version
registry.funcom.com/funcom/self-hosting/igw-k8s-battlegroup-operator:$op_version
registry.funcom.com/funcom/self-hosting/igw-k8s-database-operator:$op_version
registry.funcom.com/funcom/self-hosting/igw-k8s-server-operator:$op_version
registry.funcom.com/funcom/self-hosting/igw-k8s-utilities-operator:$op_version
EOF
}

verify_package_images_loaded() {
    local package_root="$1"
    local missing=0
    local image
    local loaded_images

    echo "Verifying package images in k3s/containerd:"
    loaded_images="$(sudo_capsule ctr -n k8s.io images ls -q)" \
        || die "unable to list k3s/containerd images with sudo ctr"
    while IFS= read -r image; do
        [ -n "$image" ] || continue
        if grep -Fxq "$image" <<< "$loaded_images"; then
            echo "  ok $image"
        else
            echo "  missing $image"
            missing=1
        fi
    done < <(expected_package_images "$package_root")

    [ "$missing" = 0 ] || die "one or more package images are not registered in k3s/containerd"
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
                sudo_capsule ctr -n k8s.io images import "$package_root/$rel"
            done < <(image_tars "$package_root")
            echo "Image import complete."
            verify_package_images_loaded "$package_root"
            ;;
        verify)
            need_cmd sudo
            echo "  env=$env"
            echo "  package_root=$package_root"
            verify_package_images_loaded "$package_root"
            ;;
        *)
            die "images command must be load or verify"
            ;;
    esac
}

active_battlegroups() {
    sudo_capsule kubectl get battlegroups -A --no-headers 2>/dev/null | awk '{print $2}'
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

    sudo_capsule kubectl get ns "$ns" >/dev/null 2>&1 || sudo_capsule kubectl create ns "$ns"
    sudo_capsule kubectl apply -n "$ns" -f "$dir/fls-secret.yaml"
    sudo_capsule kubectl apply -n "$ns" -f "$dir/rmq-secret.yaml"
    sudo_capsule kubectl apply -n "$ns" -f "$dir/battlegroup.yaml"
    echo "Capsule applied. Watch with: sudo kubectl get battlegroups -A"

    # A brand-new world comes up on a fresh Postgres volume where only the
    # superuser exists — the game role/database are never created, and the
    # operator's schema-init only waits for them (world hangs with
    # "database \"dune\" does not exist"). Ensure them now; idempotent, so a
    # swap-in of an already-initialized world is a no-op.
    provision_database_for "$ns" "$world_id"

    # A brand-new world also has no UserSettings on its shared volume, so the
    # game servers fall back to package defaults (Port=7777/IGWPort=7888 — the
    # Conan-colliding UE defaults, outside our forwarded 7782-7790 range). Seed
    # the capsule's UserSettings before the servers start. Skips an already-
    # initialized world so live settings are never clobbered.
    deploy_user_settings_for "$ns" "$dir"
}

# Seed the capsule's UserSettings onto the world's shared volume via the
# filebrowser pod (which mounts the same PVC the game servers read from at
# DuneSandbox/Saved). Only seeds a fresh world — an existing UserEngine.ini is
# left untouched so live-edited settings survive a swap-in.
deploy_user_settings_for() {
    local ns="$1" dir="$2"
    section "Deploying UserSettings"
    local src="$dir/UserSettings"
    if [ ! -f "$src/UserEngine.ini" ]; then
        echo "  capsule has no UserSettings; skipping"
        return 0
    fi
    local fbpod="" waited=0
    while [ "$waited" -lt 180 ]; do
        fbpod="$(sudo_capsule kubectl get pods -n "$ns" --no-headers 2>/dev/null \
            | awk '/-fb-deploy-/{print $1; exit}')"
        if [ -n "$fbpod" ] \
            && sudo_capsule kubectl wait --for=condition=Ready -n "$ns" "pod/$fbpod" --timeout=10s >/dev/null 2>&1; then
            break
        fi
        fbpod=""
        sleep 10
        waited=$((waited + 10))
        echo "  waiting for filebrowser pod in $ns... (${waited}s / 180s)"
    done
    [ -n "$fbpod" ] || die "filebrowser pod did not become ready in $ns; cannot deploy UserSettings"

    if sudo_capsule kubectl exec -n "$ns" "$fbpod" -- test -f /srv/UserSettings/UserEngine.ini >/dev/null 2>&1; then
        echo "  UserSettings already present on the volume; leaving them untouched"
        return 0
    fi

    sudo_capsule kubectl exec -n "$ns" "$fbpod" -- mkdir -p /srv/UserSettings
    local f
    for f in UserEngine.ini UserGame.ini; do
        [ -f "$src/$f" ] || continue
        sudo_capsule kubectl cp "$src/$f" "$ns/$fbpod:/srv/UserSettings/$f" \
            || die "failed to deploy $f to $ns"
        echo "  deployed $f"
    done
    echo "UserSettings deployed; game servers read them from the shared volume on start."
}

# Restore a database dump into a STOPPED world via a Funcom import
# DatabaseOperation, staging the dump through the filebrowser pod (which mounts
# the game PVC at /srv, so /srv/DatabaseDumps is what the operator reads). This
# is the sudo-whitelist-safe path (only `sudo -n kubectl`); the legacy
# `dune-ctl backup restore` stages via `sudo cp/mkdir` to /funcom/artifacts and
# fails under non-interactive NOPASSWD-only sudo. Caller MUST ensure the
# battlegroup is stopped first — import is destructive.
restore_database_for() {
    local ns="$1" bg="$2" dump_file="$3"
    [ -f "$dump_file" ] || die "dump file not found: $dump_file"
    local backup_name
    backup_name="$(basename "$dump_file")"

    section "Restoring database ($backup_name)"
    local fbpod="" waited=0
    while [ "$waited" -lt 180 ]; do
        fbpod="$(sudo_capsule kubectl get pods -n "$ns" --no-headers 2>/dev/null \
            | awk '/-fb-deploy-/{print $1; exit}')"
        if [ -n "$fbpod" ] \
            && sudo_capsule kubectl wait --for=condition=Ready -n "$ns" "pod/$fbpod" --timeout=10s >/dev/null 2>&1; then
            break
        fi
        fbpod=""
        sleep 10
        waited=$((waited + 10))
        echo "  waiting for filebrowser pod in $ns... (${waited}s / 180s)"
    done
    [ -n "$fbpod" ] || die "filebrowser pod did not become ready in $ns; cannot stage restore"

    # Stage the dump into the game PVC's DatabaseDumps dir (kubectl cp — no sudo cp).
    sudo_capsule kubectl exec -n "$ns" "$fbpod" -- mkdir -p /srv/DatabaseDumps
    sudo_capsule kubectl cp "$dump_file" "$ns/$fbpod:/srv/DatabaseDumps/$backup_name" \
        || die "failed to stage dump into $ns"
    echo "  staged $backup_name into the game volume"

    # Apply an import DatabaseOperation and wait for it (mirrors dune-backup.sh's
    # dump-side pattern, action=import).
    local op_name="$bg-import-$(date +%Y%m%d-%H%M%S)"
    printf '%s\n' \
        'apiVersion: igw.funcom.com/v1' \
        'kind: DatabaseOperation' \
        'metadata:' \
        "  name: $op_name" \
        "  namespace: $ns" \
        'spec:' \
        "  battleGroup: $bg" \
        '  action: import' \
        "  backup: $backup_name" \
        | sudo_capsule kubectl apply -f - || die "failed to apply import operation"
    echo "  import operation $op_name applied; waiting..."

    local elapsed=0 interval=5 timeout=600 phase
    while [ "$elapsed" -lt "$timeout" ]; do
        phase="$(sudo_capsule kubectl get databaseoperation "$op_name" -n "$ns" -o jsonpath='{.status.phase}' 2>/dev/null || true)"
        case "$phase" in
            Succeeded)
                echo "  database import succeeded."
                return 0
                ;;
            Failed)
                sudo_capsule kubectl describe databaseoperation "$op_name" -n "$ns" >&2 || true
                die "import operation $op_name failed"
                ;;
        esac
        sleep "$interval"
        elapsed=$((elapsed + interval))
        echo "  still waiting... (${elapsed}s / ${timeout}s, phase=${phase:-Pending})"
    done
    die "timed out waiting for import operation $op_name"
}

# Standalone sudo-safe restore of a backup bundle into a stopped world. Used to
# validate B0 independently of swap; B1 (swap --restore) reuses
# restore_database_for. Dry-run by default.
restore_capsule() {
    local env="live" world_id="" bundle="" apply=0
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --env) env="${2:-}"; shift 2 ;;
            --world|--world-id|--to) world_id="${2:-}"; shift 2 ;;
            --bundle) bundle="${2:-}"; shift 2 ;;
            --apply) apply=1; shift ;;
            *) die "unknown restore option: $1" ;;
        esac
    done
    validate_env "$env"
    [ -n "$world_id" ] || die "--world-id <battlegroup> is required"
    [ -n "$bundle" ] || die "--bundle <timestamp> is required"

    local ns="$BATTLEGROUP_PREFIX$world_id"
    local bundle_dir="$BACKUP_ROOT/$env/$world_id/$bundle"
    [ -d "$bundle_dir" ] || die "bundle not found: $bundle_dir"
    local dump_file
    dump_file="$(ls -1 "$bundle_dir/database/"*.backup 2>/dev/null | grep -v '\.yaml$' | head -1)"
    [ -n "$dump_file" ] || die "no .backup dump in $bundle_dir/database/"

    echo "Restore plan:"
    echo "  env=$env"
    echo "  world_id=$world_id"
    echo "  namespace=$ns"
    echo "  bundle=$bundle"
    echo "  dump=$dump_file"
    echo "  steps: verify stopped -> stage dump (kubectl) -> import DatabaseOperation"

    if [ "$apply" -ne 1 ]; then
        echo
        echo "Dry run only. Re-run with --apply to perform the restore (world must be stopped)."
        return 0
    fi

    # Import is destructive; require the battlegroup stopped.
    local stop
    stop="$(sudo_capsule kubectl get battlegroup "$world_id" -n "$ns" -o jsonpath='{.spec.stop}' 2>/dev/null || true)"
    [ "$stop" = "true" ] || die "battlegroup $world_id is not stopped (spec.stop=${stop:-unknown}); run 'dune-ctl --world $world_id sietches stop' first"

    restore_database_for "$ns" "$world_id" "$dump_file"
    echo "Restore complete. Start with: dune-ctl --world $world_id sietches start"
}

# Wait for the operator to bring up the Postgres pod, then ensure the game
# role/database exist via db-credentials.sh.
provision_database_for() {
    local ns="$1" world_id="$2"
    section "Provisioning game database"
    local dbpod="" waited=0
    while [ "$waited" -lt 300 ]; do
        dbpod="$(sudo_capsule kubectl get pods -n "$ns" --no-headers 2>/dev/null \
            | awk '/-db-dbdepl-sts-/{print $1; exit}')"
        if [ -n "$dbpod" ] \
            && sudo_capsule kubectl wait --for=condition=Ready -n "$ns" "pod/$dbpod" --timeout=10s >/dev/null 2>&1; then
            break
        fi
        dbpod=""
        sleep 10
        waited=$((waited + 10))
        echo "  waiting for Postgres pod in $ns... (${waited}s / 300s)"
    done
    [ -n "$dbpod" ] || die "Postgres pod did not become ready in $ns; cannot provision game database"
    "$REPO_ROOT/scripts/db-credentials.sh" provision --bg "$world_id" \
        || die "game database provisioning failed for $world_id"
}

# Wait for all game server pods (role=igw-server) in a namespace to terminate.
# These are the map pods owned by the ServerSet CR; waiting for them to drain
# before backup gives a consistent DB dump (the servers have flushed state).
wait_game_pods_gone() {
    local ns="$1"
    local timeout="${2:-300}"
    local waited=0
    while true; do
        local count
        count="$(sudo_capsule kubectl get pods -n "$ns" -l role=igw-server \
            --no-headers 2>/dev/null | awk 'NF {c++} END {print c+0}')"
        if [ "$count" -eq 0 ]; then
            echo "  game server pods drained"
            return 0
        fi
        if [ "$waited" -ge "$timeout" ]; then
            echo "  WARNING: $count game server pod(s) still present after ${timeout}s" >&2
            return 1
        fi
        echo "  waiting for $count game server pod(s) to terminate (${waited}s)..."
        sleep 10
        waited=$((waited + 10))
    done
}

# Park the active world: stop the battlegroup, wait for game pods to drain,
# take a final env+bg-stamped backup, export namespace evidence, and only then
# delete the namespace. Capsule files and backups remain on disk. Dry-run by
# default; pass --apply to mutate the cluster.
park_capsule() {
    local env="live"
    local world_id=""
    local apply=0
    local skip_backup=0

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --env)
                env="${2:-}"
                shift 2
                ;;
            --world-id|--world)
                world_id="${2:-}"
                shift 2
                ;;
            --apply)
                apply=1
                shift
                ;;
            --skip-backup)
                skip_backup=1
                shift
                ;;
            *)
                die "unknown park option: $1"
                ;;
        esac
    done

    validate_env "$env"
    [ -n "$world_id" ] || die "--world-id is required"
    local ns="$BATTLEGROUP_PREFIX$world_id"

    echo "Park plan:"
    echo "  env=$env"
    echo "  world_id=$world_id"
    echo "  namespace=$ns"
    echo "  backup=$([ "$skip_backup" -eq 1 ] && echo skip || echo yes)"
    echo "  steps: stop -> drain game pods -> backup -> export -> delete namespace"

    if ! sudo_capsule kubectl get ns "$ns" >/dev/null 2>&1; then
        echo
        echo "Namespace $ns does not exist; nothing to park."
        return 0
    fi

    if [ "$apply" -ne 1 ]; then
        echo
        echo "Dry run only. Re-run with --apply to stop, back up, and delete the namespace."
        return 0
    fi

    section "Stopping battlegroup $world_id"
    sudo_capsule kubectl patch battlegroup "$world_id" -n "$ns" \
        --type=merge -p '{"spec":{"stop":true}}'

    section "Draining game server pods"
    wait_game_pods_gone "$ns" || die "game server pods did not drain; refusing to park"

    if [ "$skip_backup" -ne 1 ]; then
        section "Final backup ($env/$world_id)"
        "$REPO_ROOT/scripts/dune-backup.sh" --env "$env" --bg "$world_id" \
            || die "backup failed; refusing to delete namespace $ns"
    fi

    section "Exporting namespace evidence"
    local export_dir="$CAPSULE_ROOT/$env/$world_id/exports/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$export_dir"
    sudo_capsule kubectl get all,pvc,secret,battlegroup,serverset,messagequeue \
        -n "$ns" -o yaml > "$export_dir/namespace.yaml" 2>/dev/null || true
    echo "  wrote $export_dir/namespace.yaml"

    section "Deleting namespace $ns"
    sudo_capsule kubectl delete ns "$ns" --wait=true
    echo "Parked $world_id: namespace removed; capsule files and backups retained."
}

# Hot-swap the active Live world for another capsule. Parks whichever world is
# currently online (backup + namespace teardown), then activates the target.
# Enforces the single-active invariant: exactly one Live world online at a time.
swap_capsule() {
    local env="live"
    local target=""
    local apply=0
    local skip_backup=0

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --env)
                env="${2:-}"
                shift 2
                ;;
            --to|--world|--world-id)
                target="${2:-}"
                shift 2
                ;;
            --apply)
                apply=1
                shift
                ;;
            --skip-backup)
                skip_backup=1
                shift
                ;;
            *)
                die "unknown swap option: $1"
                ;;
        esac
    done

    validate_env "$env"
    [ -n "$target" ] || die "--to <battlegroup> is required"

    # Validate the target capsule exists before touching the live world.
    local target_dir
    target_dir="$(resolve_capsule_dir "$env" "$target")"

    local active_list active_count
    active_list="$(active_battlegroups || true)"
    active_count="$(printf '%s\n' "$active_list" | awk 'NF {count++} END {print count+0}')"

    # Refuse if the target is already the active world.
    if printf '%s\n' "$active_list" | grep -qx "$target"; then
        die "target $target is already active; nothing to swap"
    fi

    echo "Swap plan:"
    echo "  env=$env"
    echo "  target=$target ($target_dir)"
    if [ "$active_count" -gt 0 ]; then
        echo "  park_active=$(printf '%s' "$active_list" | tr '\n' ' ')"
    else
        echo "  park_active=none (no world currently online)"
    fi
    echo "  steps: park active world(s) -> activate target -> FLS re-declare -> preflight"

    if [ "$apply" -ne 1 ]; then
        echo
        echo "Dry run only. Re-run with --apply to perform the swap."
        return 0
    fi

    # Park every currently-active world so the target activates into a clean host.
    local bg
    while IFS= read -r bg; do
        [ -n "$bg" ] || continue
        section "Parking active world $bg"
        local park_args=(--env "$env" --world-id "$bg" --apply)
        [ "$skip_backup" -eq 1 ] && park_args+=(--skip-backup)
        park_capsule "${park_args[@]}"
    done <<EOF
$active_list
EOF

    section "Activating target $target"
    activate_capsule --env "$env" --world-id "$target" --apply

    # Re-point the nightly backup schedule at the newly active world. The parked
    # world's namespace is gone, so leaving the cron on it would silently break
    # backups. Best-effort: a swap that already activated the target must not be
    # failed by a crontab hiccup. No-op if no schedule is installed.
    section "Retargeting backup schedule"
    local dune_ctl="$REPO_ROOT/dune-ctl/target/release/dune-ctl"
    if [ -x "$dune_ctl" ]; then
        "$dune_ctl" --world "$target" backup schedule --retarget \
            || echo "WARNING: backup schedule retarget failed; run 'dune-ctl --world $target backup schedule --retarget' manually" >&2
    else
        echo "WARNING: dune-ctl binary not found at $dune_ctl; retarget the nightly backup schedule to $target manually" >&2
    fi

    section "Swap complete"
    cat <<EOF
Target $target activated. Next:
  - Wait ~5-10 min for FLS re-declaration before the world is browser-visible.
  - Verify: dune-ctl --world $target preflight
            dune-ctl --world $target status
EOF
}

copy_user_settings() {
    local source_root="$1"
    local dest_dir="$2"
    local sietch_name="$3"
    mkdir -p "$dest_dir"
    cp "$source_root/scripts/setup/config/UserEngine.ini" "$dest_dir/UserEngine.ini"
    cp "$source_root/scripts/setup/config/UserGame.ini" "$dest_dir/UserGame.ini"
    perl -0pi -e '
        s/^Port=\d+$/Port=7782/m;
        s/^IGWPort=\d+$/IGWPort=7893/m;
        s/7777, 7778 etc\./7782, 7783 etc./g;
        s/7888, 7889 etc\./7893, 7894 etc./g;
    ' "$dest_dir/UserEngine.ini"

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
    # Refuse only a genuinely PTC-only root: a PTC manifest with no live manifest
    # alongside it. A valid live root may also carry a stray PTC manifest (steamapps
    # accumulates appmanifests) — Ixware's own live root does — so the live manifest
    # being present is the authoritative signal that live content is installed here.
    if [ "$env" = "live" ] \
        && [ -f "$package_root/steamapps/appmanifest_$DEFAULT_PTC_APP_ID.acf" ] \
        && [ ! -f "$package_root/steamapps/appmanifest_$DEFAULT_LIVE_APP_ID.acf" ]; then
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
    if ! sudo_capsule kubectl get battlegroups -A -o wide 2>/dev/null; then
        echo "kubectl battlegroup inventory unavailable."
    fi

    section "Kubernetes Services / NodePorts"
    if ! sudo_capsule kubectl get svc -A -o wide 2>/dev/null | awk '
        NR == 1 || $1 ~ /^funcom-seabass-/ || $1 == "kube-system" && $2 == "traefik" {print}
    '; then
        echo "kubectl service inventory unavailable."
    fi

    section "Kubernetes PVCs"
    if ! sudo_capsule kubectl get pvc -A 2>/dev/null | awk 'NR == 1 || $1 ~ /^funcom-seabass-/ {print}'; then
        echo "kubectl pvc inventory unavailable."
    fi
}

print_loaded_images() {
    section "Loaded Dune Images"
    if ! sudo_capsule ctr -n k8s.io images ls -q 2>/dev/null \
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
    park)
        shift
        park_capsule "$@"
        ;;
    swap)
        shift
        swap_capsule "$@"
        ;;
    restore)
        shift
        restore_capsule "$@"
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
