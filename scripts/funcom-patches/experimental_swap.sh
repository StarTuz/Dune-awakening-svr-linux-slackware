#!/bin/bash

G_SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BATTLEGROUP_PREFIX="funcom-seabass-"

# Map settings
declare -A map_to_requests
map_to_requests["Survival_1"]='{"limits": { "memory": "12Gi" }, "requests": { "memory": "5Gi" }}'
map_to_requests["Overmap"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["SH_Arrakeen"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["SH_HarkoVillage"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["DeepDesert_1"]='{"limits": { "memory": "10Gi" }, "requests": { "memory": "3Gi" }}'
map_to_requests["CB_Ecolab_Bronze_Green_089"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Ecolab_Bronze_Green_152"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Ecolab_Bronze_Green_024"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Ecolab_Bronze_Green_195"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Ecolab_Bronze_Green_136"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Overland_M_01"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Overland_S_04"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Overland_S_06"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Overland_S_07"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Overland_S_08"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Dungeon_ThePit"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Story_BanditFortress01"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["Story_HeighlinerDungeon"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["Story_Faction_Outpost_Hark"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["Story_Faction_Outpost_Atre"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Dungeon_OldCarthag"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Dungeon_Hephaestus"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["Story_ArtOfKanly"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["DLC_Story_LostHarvest_ForgottenLab"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["DLC_Story_LostHarvest_EcolabB"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["DLC_Story_LostHarvest_EcolabA"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["Story_ProcesVerbal"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Story_WaterFatManor"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Story_Ecolab_Carthag"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'
map_to_requests["CB_Story_Hephaestus"]='{"limits": { "memory": "1Gi" }, "requests": { "memory": "200Mi" }}'


# Methods
configure_battlegroup() {
        # Load battlegroup info
        declare -A map_to_index
        while IFS=': ' read -r index value; do
          map_to_index["$value"]=$index
        done < <(sudo kubectl get battlegroups -n "funcom-seabass-$bgname" "$bgname" -o json | jq -r '.spec.serverGroup.template.spec.sets | to_entries[] | "\(.key): \(.value.map)"')

        for key in "${!map_to_index[@]}"; do
                if [[ ${map_to_requests[$key]} ]]; then
                        echo "Applying resources for $key, with index: ${map_to_index[$key]} and requests: ${map_to_requests[$key]}"
                        payload='[{"op": "replace", "path": "/spec/serverGroup/template/spec/sets/'"${map_to_index[$key]}"'/resources", "value": '"${map_to_requests[$key]}"'}]'
                        sudo kubectl patch battlegroup -n "funcom-seabass-$bgname" "$bgname" --type='json' -p="$payload"
                fi
        done
}

enable_swap() {
        # Experimental SWAP setup
        # Slackware note: host already has swap (zram + dune-vg + sdc1); skip swapfile creation if swap is present
        if /sbin/swapon --show | grep -q .; then
          echo "Swap already active on this host, skipping swapfile creation."
        elif [ -f "/swapfile" ]; then
          echo "Swap file exists but not active — activating."
          sudo swapon /swapfile
        else
          # Setting up the swap file
          sudo dd if=/dev/zero of=/swapfile bs=1024 count=31457280
          sudo chmod 600 /swapfile
          sudo mkswap /swapfile
          sudo swapon /swapfile
          # Adding swapfile to fstab
          echo '/swapfile swap swap defaults 0 0' | sudo tee -a /etc/fstab
          # rc-update is a stub on Slackware; swap persistence is handled by fstab + rc.local
          sudo rc-update add swap boot
        fi

        # Stop k3s before continuing on updates
        sudo rc-service k3s stop
        sudo k3s-killall.sh

        # Update k3s to use the swap (skip if already correct — sudo tee not in sudoers whitelist)
        if ! grep -q 'kubelet-config.yaml' /etc/rancher/k3s/config.yaml 2>/dev/null; then
          echo -e "kubelet-arg:\n- config=/etc/rancher/k3s/kubelet-config.yaml" | sudo tee /etc/rancher/k3s/config.yaml
        else
          echo "k3s config.yaml already correct, skipping"
        fi

        # Copy new kubelet config (skip if already matches)
        if ! diff -q "$G_SCRIPT_PATH/templates/kubelet-config.yaml" /etc/rancher/k3s/kubelet-config.yaml &>/dev/null; then
          sudo cp "$G_SCRIPT_PATH/templates/kubelet-config.yaml" "/etc/rancher/k3s/kubelet-config.yaml"
        else
          echo "kubelet-config.yaml already correct, skipping"
        fi

        # Prevent k3s server process itself from swapping.
        # Upstream uses Alpine's OpenRC cgroup path (/sys/fs/cgroup/openrc.k3s/memory.swap.max)
        # which does not exist on Slackware. Locate the actual k3s cgroup dynamically instead.
        K3S_PID=$(pgrep -x k3s-server || pgrep -f 'k3s server' | head -1)
        if [ -n "$K3S_PID" ]; then
          K3S_CGROUP=$(cat /proc/$K3S_PID/cgroup 2>/dev/null | grep -oP '(?<=:memory:).*' | head -1)
          SWAP_MAX=/sys/fs/cgroup/memory${K3S_CGROUP}/memory.memsw.limit_in_bytes
          if [ -f "$SWAP_MAX" ]; then
            # cgroup v1: set memsw limit == memory limit to disallow swap for k3s
            MEM_LIMIT=$(cat /sys/fs/cgroup/memory${K3S_CGROUP}/memory.limit_in_bytes)
            echo "$MEM_LIMIT" | sudo tee "$SWAP_MAX"
            echo "k3s swap disabled via $SWAP_MAX"
          else
            echo "WARNING: could not find k3s cgroup memory interface — k3s process may use swap"
          fi
        else
          echo "k3s not running yet; cgroup swap lock will apply after restart"
        fi

        # Restart k3s with the new changes
        sudo rc-service k3s restart
        sleep 2m # Give k3s a chance to get back up
}

# Battlegroup name must be passed as the first argument
bgname="$1"
if [ -z "$bgname" ]; then
        echo "Usage: $0 <battlegroup-name>"
        exit 1
fi

# First lets try to enable swap:
enable_swap

# Lets configure the battlegroups
configure_battlegroup
