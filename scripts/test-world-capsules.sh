#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WORLD_CAPSULES="$SCRIPT_DIR/world-capsules.sh"
PACKAGE_ROOT="$REPO_ROOT/server"
TMP_ROOT="$(mktemp -d /tmp/dune-world-capsules-test.XXXXXX)"

cleanup() {
    rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

b64url() {
    openssl base64 -A | tr '+/' '-_' | tr -d '='
}

fake_token() {
    local header payload
    header="$(printf '{"alg":"HS256","typ":"JWT"}' | b64url)"
    payload="$(printf '{"HostId":"TESTHOST","exp":1809813921}' | b64url)"
    printf '%s.%s.signature\n' "$header" "$payload"
}

TOKEN="$(fake_token)"
WORLD_ID="sh-testhost-abcdef"
CAPSULE_DIR="$TMP_ROOT/.dune/capsules/ptc/$WORLD_ID"

DUNE_HOME="$TMP_ROOT/.dune" "$WORLD_CAPSULES" package validate \
    --env ptc \
    --app-id 3104830 \
    --package-root "$PACKAGE_ROOT" >/tmp/dune-world-capsules-package.out

DUNE_HOME="$TMP_ROOT/.dune" "$WORLD_CAPSULES" create \
    --env ptc \
    --name "Harness Arrakis" \
    --sietch-name "Sietch Abbir" \
    --region "North America Test" \
    --token "$TOKEN" \
    --package-root "$PACKAGE_ROOT" \
    --world-id "$WORLD_ID" \
    --force >/tmp/dune-world-capsules-create.out

[ -f "$CAPSULE_DIR/capsule.env" ] || fail "capsule.env missing"
[ -f "$CAPSULE_DIR/battlegroup.yaml" ] || fail "battlegroup.yaml missing"
[ -f "$CAPSULE_DIR/fls-secret.yaml" ] || fail "fls-secret.yaml missing"
[ -f "$CAPSULE_DIR/rmq-secret.yaml" ] || fail "rmq-secret.yaml missing"
[ -f "$CAPSULE_DIR/UserSettings/UserEngine.ini" ] || fail "UserEngine.ini missing"
[ -L "$CAPSULE_DIR/package-root" ] || fail "package-root symlink missing"

grep -q '^environment=ptc$' "$CAPSULE_DIR/capsule.env" || fail "environment not recorded"
grep -q '^world_id=sh-testhost-abcdef$' "$CAPSULE_DIR/capsule.env" || fail "world id not recorded"
grep -q '^token_host_id=testhost$' "$CAPSULE_DIR/capsule.env" || fail "token host id not recorded"
grep -q '^backup_root=.*/ptc/sh-testhost-abcdef$' "$CAPSULE_DIR/capsule.env" || fail "backup root not environment scoped"
grep -q 'name: sh-testhost-abcdef' "$CAPSULE_DIR/battlegroup.yaml" || fail "world id not rendered"
grep -q 'title: Harness Arrakis' "$CAPSULE_DIR/battlegroup.yaml" || fail "world title not rendered"
grep -q -- '-FarmRegion=North America Test' "$CAPSULE_DIR/battlegroup.yaml" || fail "region not rendered"
grep -q 'Bgd.ServerDisplayName="Sietch Abbir"' "$CAPSULE_DIR/UserSettings/UserEngine.ini" || fail "sietch name not rendered"
if grep -q "$TOKEN" "$CAPSULE_DIR/capsule.env"; then
    fail "token leaked into capsule.env"
fi

echo "world-capsules harness passed"
