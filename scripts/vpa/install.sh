#!/bin/bash
# Install VPA recommender (Off mode, recommendations only) on the k3s cluster.
#
# What this does:
#   1. Downloads and applies the VPA CRDs (v1.6.0)
#   2. Applies RBAC for the recommender service account
#   3. Deploys the recommender pod into kube-system
#   4. Creates Off-mode VPA objects for all Deployments/StatefulSets in battlegroup namespaces
#
# Safe to re-run (all steps are idempotent).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VPA_VERSION="1.6.0"
CRD_URL="https://raw.githubusercontent.com/kubernetes/autoscaler/vertical-pod-autoscaler-${VPA_VERSION}/vertical-pod-autoscaler/deploy/vpa-v1-crd-gen.yaml"
CRD_FILE="$SCRIPT_DIR/vpa-v1-crd-gen.yaml"

echo "=== VPA install (v${VPA_VERSION}) ==="

# Step 1: CRDs
if [[ ! -f "$CRD_FILE" ]]; then
    echo "Downloading VPA CRDs..."
    curl -sSfL "$CRD_URL" -o "$CRD_FILE"
else
    echo "CRD file already present: $CRD_FILE"
fi
echo "Applying VPA CRDs..."
sudo kubectl apply -f "$CRD_FILE"

# Step 2: RBAC
echo "Applying recommender RBAC..."
sudo kubectl apply -f "$SCRIPT_DIR/recommender-rbac.yaml"

# Step 3: Recommender deployment
echo "Applying recommender deployment..."
sudo kubectl apply -f "$SCRIPT_DIR/recommender-deployment.yaml"

# Step 4: Wait for recommender to be ready (up to 2 minutes)
echo "Waiting for vpa-recommender rollout..."
sudo kubectl rollout status deployment/vpa-recommender -n kube-system --timeout=120s

# Step 5: VPA objects for standard workloads
echo "Creating VPA objects for battlegroup workloads..."
bash "$SCRIPT_DIR/vpa-objects.sh"

echo ""
echo "=== VPA install complete ==="
echo "Recommender pod:"
sudo kubectl get pod -n kube-system -l app=vpa-recommender
echo ""
echo "VPA objects (wait ~24h for initial recommendations):"
BATTLEGROUP_NS=$(sudo kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name | grep '^funcom-seabass-' | head -1)
if [[ -n "$BATTLEGROUP_NS" ]]; then
    sudo kubectl get vpa -n "$BATTLEGROUP_NS"
fi
echo ""
echo "Game server pods (Survival_1, Overmap) are owned by ServerSet CRD — VPA cannot"
echo "auto-target them. Run watch-gameservers.sh to track their memory usage:"
echo "  $SCRIPT_DIR/watch-gameservers.sh --once"
echo "  $SCRIPT_DIR/watch-gameservers.sh          # continuous (120s interval)"
echo "To adjust game server memory requests, edit experimental_swap.sh map_to_requests"
echo "and re-run, or kubectl patch the BattleGroup CR directly."
