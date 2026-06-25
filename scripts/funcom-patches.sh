#!/bin/bash
# Re-apply Slackware patches to Funcom-shipped scripts/config.
#
# Background: SteamCMD overwrites everything under server/scripts/ on every
# update. Our Slackware-specific edits to those files need to be re-applied
# after each update. Wired into update.sh.
#
# Layout under scripts/funcom-patches/:
#   <name>.sh           — our patched version (full file)
#   <name>.sh.upstream  — pristine upstream baseline this patch was built against
#
# For each patched file:
#   - If target already matches our patched version → "already patched", skip
#   - If target matches the recorded upstream baseline → cp our version, "patched"
#   - Otherwise upstream drifted → warn loudly, refuse to overwrite
#
# The third case is the safety net: if Funcom changes the script underneath us,
# we don't silently clobber their changes. Operator updates the .upstream
# baseline + patched version together and re-runs.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PATCH_DIR="$SCRIPT_DIR/funcom-patches"
TARGET_DIR="$SCRIPT_DIR/../server/scripts/setup"
CONFIG_DIR="$TARGET_DIR/config"

patch_userengine_ports() {
    local file="$CONFIG_DIR/UserEngine.ini"
    if [[ ! -f "$file" ]]; then
        echo "SKIP UserEngine.ini — target $file does not exist"
        return 0
    fi

    if grep -q '^Port=7782$' "$file" && grep -q '^IGWPort=7893$' "$file"; then
        echo "OK   UserEngine.ini — Dune port base already 7782/7893"
        return 0
    fi

    cp "$file" "$file.bak-port-$(date -u +%Y%m%d%H%M%S)"
    perl -0pi -e '
        s/^Port=\d+$/Port=7782/m;
        s/^IGWPort=\d+$/IGWPort=7893/m;
        s/7777, 7778 etc\./7782, 7783 etc./g;
        s/7888, 7889 etc\./7893, 7894 etc./g;
    ' "$file"
    echo "PATCH UserEngine.ini — set Dune port base to 7782/7893"
}

if [[ ! -d "$PATCH_DIR" ]]; then
    echo "No funcom-patches/ dir; nothing to do."
    exit 0
fi

shopt -s nullglob
status=0
for patched in "$PATCH_DIR"/*.sh; do
    name=$(basename "$patched")
    upstream="$PATCH_DIR/$name.upstream"
    target="$TARGET_DIR/$name"

    if [[ ! -f "$target" ]]; then
        echo "SKIP $name — target $target does not exist"
        continue
    fi
    if [[ ! -f "$upstream" ]]; then
        echo "SKIP $name — no upstream baseline $upstream"
        continue
    fi

    if cmp -s "$patched" "$target"; then
        echo "OK   $name — already patched"
        continue
    fi
    if cmp -s "$upstream" "$target"; then
        cp "$patched" "$target"
        chmod --reference="$upstream" "$target" 2>/dev/null || true
        echo "PATCH $name — applied"
        continue
    fi

    echo "WARN $name — upstream drift detected"
    echo "     target hash:   $(sha256sum "$target" | awk '{print $1}')"
    echo "     baseline hash: $(sha256sum "$upstream" | awk '{print $1}')"
    echo "     patched hash:  $(sha256sum "$patched" | awk '{print $1}')"
    echo "     review with: diff $upstream $target"
    echo "     to accept new upstream: cp $target $upstream  (and update $patched)"
    status=1
done

patch_userengine_ports

exit $status
