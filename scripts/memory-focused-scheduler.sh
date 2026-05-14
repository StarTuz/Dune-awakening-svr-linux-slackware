#!/bin/bash
# Passthrough scheduler for pods requesting memory-focused-scheduler.
# Runs on the host; binds pending pods to the single k3s node.
SCHED_NAME="memory-focused-scheduler"
NODE=$(sudo kubectl get node -o jsonpath='{.items[0].metadata.name}')
echo "$(date -Iseconds) memory-focused-scheduler started (node=$NODE)"

while true; do
    sudo kubectl get pods -A -o json 2>/dev/null | python3 -c "
import sys, json, subprocess, tempfile, os

data = json.load(sys.stdin)
node = '$NODE'
sched = '$SCHED_NAME'

for pod in data['items']:
    spec = pod.get('spec', {})
    if spec.get('schedulerName') != sched:
        continue
    if spec.get('nodeName'):
        continue
    phase = pod.get('status', {}).get('phase', '')
    if phase not in ('', 'Pending'):
        continue
    ns   = pod['metadata']['namespace']
    name = pod['metadata']['name']
    binding = {
        'apiVersion': 'v1',
        'kind': 'Binding',
        'metadata': {'name': name},
        'target': {'apiVersion': 'v1', 'kind': 'Node', 'name': node}
    }
    with tempfile.NamedTemporaryFile(mode='w', suffix='.json', delete=False) as f:
        json.dump(binding, f)
        fname = f.name
    r = subprocess.run(['sudo','kubectl','create','-n',ns,'-f',fname],
                       capture_output=True, text=True)
    os.unlink(fname)
    if r.returncode == 0:
        print(f'Bound {ns}/{name} to {node}', flush=True)
    elif 'AlreadyExists' not in r.stderr:
        print(f'Error binding {ns}/{name}: {r.stderr.strip()}', flush=True)
"
    sleep 5
done
