#!/bin/bash
# Create VPA objects in Off mode for all Deployments and StatefulSets in the
# active battlegroup namespace. Off mode = recommendations only, no auto-apply.
# Re-running is safe (kubectl apply is idempotent).

set -e

BATTLEGROUP_PREFIX="funcom-seabass-"
namespaces=( $(sudo kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name | grep "^$BATTLEGROUP_PREFIX") )

if [ ${#namespaces[@]} -eq 0 ]; then
    echo "No battlegroup namespace found."
    exit 1
fi

for NS in "${namespaces[@]}"; do
    echo "=== Namespace: $NS ==="

    for kind in Deployment StatefulSet; do
        names=( $(sudo kubectl get "$kind" -n "$NS" --no-headers -o custom-columns=NAME:.metadata.name 2>/dev/null) )
        for name in "${names[@]}"; do
            vpa_name="vpa-${name}"
            echo "  Applying VPA ($kind/$name) → $vpa_name"
            sudo kubectl apply -f - <<EOF
apiVersion: autoscaling.k8s.io/v1
kind: VerticalPodAutoscaler
metadata:
  name: ${vpa_name}
  namespace: ${NS}
spec:
  targetRef:
    apiVersion: apps/v1
    kind: ${kind}
    name: ${name}
  updatePolicy:
    updateMode: "Off"
  resourcePolicy:
    containerPolicies:
    - containerName: "*"
      controlledResources: ["memory"]
      controlledValues: RequestsAndLimits
EOF
        done
    done
done

echo ""
echo "Done. View recommendations with:"
echo "  sudo kubectl get vpa -n <namespace>"
echo "  sudo kubectl describe vpa <name> -n <namespace>"
