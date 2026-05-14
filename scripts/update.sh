#!/bin/bash
# Update Dune: Awakening to the latest Steam build.
#
# What this does:
#   1. SteamCMD pre-fetch with validate — works around Funcom revoking old
#                        depot manifests on PTC. Without `validate`, steamcmd
#                        tries to verify the currently-installed manifest
#                        before planning a delta, and the old manifest
#                        returns "Access Denied" on subsequent runs.
#   2. battlegroup.sh update — runs Funcom's update flow:
#                        - steamcmd (no-op now, already up to date)
#                        - operator image+CRD update
#                        - battlegroup CR patched to new image revision
#   3. Gateway patch    — re-applies --RMQGameHttpPort=30196 (lost when the
#                        server-operator regenerates the gateway Deployment)
#
# Usage:
#   ~/dune-server/scripts/update.sh

set -euo pipefail

SCRIPT_DIR="$(dirname "$(realpath "$0")")"
DOWNLOAD_PATH="$SCRIPT_DIR/../server"

echo "=== Pre-fetching with validate (works around revoked PTC manifests) ==="
~/steamcmd/steamcmd.sh +force_install_dir "$DOWNLOAD_PATH" \
    +login anonymous +app_update 3104830 validate +quit

echo ""
echo "=== Re-applying Slackware patches to Funcom scripts ==="
"$SCRIPT_DIR/funcom-patches.sh"

echo ""
echo "=== Pulling latest server build ==="
"$SCRIPT_DIR/../server/scripts/battlegroup.sh" update

echo ""
echo "=== Re-applying gateway RMQ HTTP port patch ==="
"$SCRIPT_DIR/gateway-patch.sh"

echo ""
echo "=== Update complete ==="
