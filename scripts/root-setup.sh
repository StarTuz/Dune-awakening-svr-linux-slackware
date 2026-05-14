#!/bin/bash
# Run as root. Idempotent — safe to re-run.
set -e

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
ok()   { echo -e "${GREEN}  [ok]${NC} $*"; }
skip() { echo -e "${YELLOW}[skip]${NC} $*"; }
fail() { echo -e "${RED}[FAIL]${NC} $*"; exit 1; }
info() { echo -e "  ...  $*"; }

[ "$(id -u)" -eq 0 ] || fail "Must be run as root"

K3S_VERSION="v1.36.0+k3s1"
K3S_BIN=/usr/local/bin/k3s
DUNE_LOG_DIR=/home/dune/dune-server/logs

echo ""
echo "=== Step 1: k3s binary ==="
if [ -x "$K3S_BIN" ] && "$K3S_BIN" --version 2>/dev/null | grep -q "$K3S_VERSION"; then
    skip "k3s $K3S_VERSION already installed"
else
    info "Downloading k3s $K3S_VERSION..."
    curl -fL "https://github.com/k3s-io/k3s/releases/download/${K3S_VERSION}/k3s" \
        -o /tmp/k3s
    install -o root -g root -m 0755 /tmp/k3s "$K3S_BIN"
    rm -f /tmp/k3s
    ok "k3s installed to $K3S_BIN"
fi

echo ""
echo "=== Step 2: kubectl / crictl symlinks ==="
for link in kubectl crictl; do
    if [ -L "/usr/local/bin/$link" ] && [ "$(readlink /usr/local/bin/$link)" = "$K3S_BIN" ]; then
        skip "/usr/local/bin/$link already symlinked"
    else
        ln -sf "$K3S_BIN" "/usr/local/bin/$link"
        ok "/usr/local/bin/$link -> $K3S_BIN"
    fi
done

echo ""
echo "=== Step 3: ctr wrapper ==="
if [ -x /usr/local/bin/ctr ] && grep -q 'k3s ctr' /usr/local/bin/ctr 2>/dev/null; then
    skip "ctr wrapper already present"
else
    cat > /usr/local/bin/ctr << 'EOF'
#!/bin/sh
exec /usr/local/bin/k3s ctr "$@"
EOF
    chmod 0755 /usr/local/bin/ctr
    ok "ctr wrapper written"
fi

echo ""
echo "=== Step 4: OpenRC shims (rc-service / rc-update) ==="
if [ -x /usr/local/bin/rc-service ] && grep -q 'rc.d' /usr/local/bin/rc-service 2>/dev/null; then
    skip "rc-service shim already present"
else
    cat > /usr/local/bin/rc-service << 'EOF'
#!/bin/sh
exec /etc/rc.d/rc.${1} ${2}
EOF
    chmod 0755 /usr/local/bin/rc-service
    ok "rc-service shim written"
fi

if [ -x /usr/local/bin/rc-update ] && grep -q 'stub' /usr/local/bin/rc-update 2>/dev/null; then
    skip "rc-update stub already present"
else
    cat > /usr/local/bin/rc-update << 'EOF'
#!/bin/sh
# stub — k3s boot integration handled by Slackware rc.local
echo "rc-update: $*  (stubbed on Slackware)"
EOF
    chmod 0755 /usr/local/bin/rc-update
    ok "rc-update stub written"
fi

echo ""
echo "=== Step 5: k3s config and kubelet config ==="
mkdir -p /etc/rancher/k3s

if [ -f /etc/rancher/k3s/config.yaml ]; then
    skip "/etc/rancher/k3s/config.yaml already exists"
else
    cat > /etc/rancher/k3s/config.yaml << 'EOF'
kubelet-arg:
  - config=/etc/rancher/k3s/kubelet-config.yaml
EOF
    ok "k3s config.yaml written"
fi

if [ -f /etc/rancher/k3s/kubelet-config.yaml ]; then
    skip "/etc/rancher/k3s/kubelet-config.yaml already exists"
else
    cat > /etc/rancher/k3s/kubelet-config.yaml << 'EOF'
apiVersion: kubelet.config.k8s.io/v1beta1
kind: KubeletConfiguration
imageGCHighThresholdPercent: 99
imageGCLowThresholdPercent: 98
failSwapOn: false
memorySwap:
  swapBehavior: LimitedSwap
evictionHard:
  memory.available: "100Mi"
  nodefs.available: "1%"
  nodefs.inodesFree: "1%"
  imagefs.available: "1%"
  imagefs.inodesFree: "1%"
containerLogMaxSize: "50Mi"
containerLogMaxFiles: 2
systemReserved:
  memory: "2Gi"
EOF
    ok "kubelet-config.yaml written"
fi

echo ""
echo "=== Step 6: Slackware rc.d service script ==="
if [ -x /etc/rc.d/rc.k3s ]; then
    skip "/etc/rc.d/rc.k3s already exists"
else
    cat > /etc/rc.d/rc.k3s << RCEOF
#!/bin/sh
K3S_BIN=/usr/local/bin/k3s
K3S_LOG=${DUNE_LOG_DIR}/k3s.log
K3S_PIDFILE=/var/run/k3s.pid

k3s_start() {
    if [ -f "\$K3S_PIDFILE" ] && kill -0 "\$(cat \$K3S_PIDFILE)" 2>/dev/null; then
        echo "k3s is already running (PID \$(cat \$K3S_PIDFILE))"
        return
    fi
    echo "Starting k3s..."
    \$K3S_BIN server >> "\$K3S_LOG" 2>&1 &
    echo \$! > "\$K3S_PIDFILE"
    echo "k3s started (PID \$(cat \$K3S_PIDFILE))"
}

k3s_stop() {
    if [ -f "\$K3S_PIDFILE" ]; then
        echo "Stopping k3s..."
        kill "\$(cat \$K3S_PIDFILE)" 2>/dev/null
        rm -f "\$K3S_PIDFILE"
    fi
    /usr/local/bin/k3s-killall.sh 2>/dev/null || true
    echo "k3s stopped"
}

k3s_status() {
    if [ -f "\$K3S_PIDFILE" ] && kill -0 "\$(cat \$K3S_PIDFILE)" 2>/dev/null; then
        echo "k3s is running (PID \$(cat \$K3S_PIDFILE))"
    else
        echo "k3s is not running"
    fi
}

case "\$1" in
    start)   k3s_start ;;
    stop)    k3s_stop ;;
    restart) k3s_stop; sleep 3; k3s_start ;;
    status)  k3s_status ;;
    *)       echo "Usage: \$0 {start|stop|restart|status}" ;;
esac
RCEOF
    chmod 0755 /etc/rc.d/rc.k3s
    ok "/etc/rc.d/rc.k3s written"
fi

echo ""
echo "=== Step 7: sudoers entry for dune ==="
SUDOERS_FILE=/etc/sudoers.d/dune-k3s
if [ -f "$SUDOERS_FILE" ]; then
    skip "$SUDOERS_FILE already exists"
else
    mkdir -p /etc/sudoers.d
    cat > "$SUDOERS_FILE" << 'EOF'
dune ALL=(ALL) NOPASSWD: /usr/local/bin/kubectl, /usr/local/bin/ctr, /usr/local/bin/k3s, /usr/local/bin/rc-service, /usr/local/bin/k3s-killall.sh
EOF
    chmod 0440 "$SUDOERS_FILE"
    # Validate before leaving it in place
    if visudo -cf "$SUDOERS_FILE" 2>/dev/null; then
        ok "sudoers entry written and validated"
    else
        rm -f "$SUDOERS_FILE"
        fail "sudoers validation failed — entry removed, check manually"
    fi
fi

echo ""
echo "=== Step 8: LVM on sdb2 — swap + backup volume ==="
# sdb2 is a 182.9 GB SSD partition. 32 GB → swap LV (priority -1, between zram and sdc1).
# Remainder → btrfs backups LV mounted at /srv/backups with dune/ and conan/ subdirs.

if ! command -v pvcreate &>/dev/null; then
    fail "LVM tools not found — install the lvm2 package first"
fi

if pvdisplay /dev/sdb2 &>/dev/null 2>&1; then
    skip "/dev/sdb2 already an LVM PV"
else
    pvcreate /dev/sdb2
    ok "/dev/sdb2 initialised as LVM PV"
fi

if vgdisplay dune-vg &>/dev/null 2>&1; then
    skip "VG dune-vg already exists"
else
    vgcreate dune-vg /dev/sdb2
    ok "VG dune-vg created"
fi

if lvdisplay /dev/dune-vg/swap &>/dev/null 2>&1; then
    skip "LV dune-vg/swap already exists"
else
    lvcreate -L 32G -n swap dune-vg
    mkswap -L dune-swap /dev/dune-vg/swap
    ok "LV dune-vg/swap created (32 GB)"
fi

if lvdisplay /dev/dune-vg/backups &>/dev/null 2>&1; then
    skip "LV dune-vg/backups already exists"
else
    lvcreate -l 100%FREE -n backups dune-vg
    mkfs.btrfs -L dune-backups /dev/dune-vg/backups
    ok "LV dune-vg/backups created (remaining ~150 GB, btrfs)"
fi

BACKUPS_MOUNT=/srv/backups
if [ -d "$BACKUPS_MOUNT" ]; then
    skip "$BACKUPS_MOUNT already exists"
else
    mkdir -p "$BACKUPS_MOUNT"
    ok "$BACKUPS_MOUNT created"
fi

if grep -q 'dune-vg/swap' /etc/fstab; then
    skip "swap fstab entry already present"
else
    echo '/dev/dune-vg/swap      swap             swap   defaults,pri=-1          0  0' >> /etc/fstab
    ok "swap fstab entry added (priority -1)"
fi

if grep -q 'dune-vg/backups' /etc/fstab; then
    skip "backups fstab entry already present"
else
    echo "/dev/dune-vg/backups   $BACKUPS_MOUNT   btrfs  defaults,compress=zstd   0  0" >> /etc/fstab
    ok "backups fstab entry added"
fi

if swapon --show | grep -q 'dune-vg'; then
    skip "dune-vg swap already active"
else
    swapon -p -1 /dev/dune-vg/swap
    ok "dune-vg swap activated at priority -1"
fi

if mountpoint -q "$BACKUPS_MOUNT"; then
    skip "$BACKUPS_MOUNT already mounted"
else
    mount "$BACKUPS_MOUNT"
    ok "$BACKUPS_MOUNT mounted"
fi

for subdir in dune conan; do
    if [ -d "$BACKUPS_MOUNT/$subdir" ]; then
        skip "$BACKUPS_MOUNT/$subdir already exists"
    else
        mkdir -p "$BACKUPS_MOUNT/$subdir"
        ok "$BACKUPS_MOUNT/$subdir created"
    fi
done

chown dune:users  "$BACKUPS_MOUNT/dune"
chown conan:users "$BACKUPS_MOUNT/conan"
ok "backup dir ownership set"

echo ""
echo "=== Verification summary ==="
echo -n "k3s binary:       "; "$K3S_BIN" --version 2>/dev/null || echo "MISSING"
echo -n "kubectl symlink:  "; readlink /usr/local/bin/kubectl 2>/dev/null || echo "MISSING"
echo -n "ctr wrapper:      "; [ -x /usr/local/bin/ctr ]       && echo "ok" || echo "MISSING"
echo -n "rc-service shim:  "; [ -x /usr/local/bin/rc-service ] && echo "ok" || echo "MISSING"
echo -n "rc-update stub:   "; [ -x /usr/local/bin/rc-update ]  && echo "ok" || echo "MISSING"
echo -n "k3s config:       "; [ -f /etc/rancher/k3s/config.yaml ]         && echo "ok" || echo "MISSING"
echo -n "kubelet config:   "; [ -f /etc/rancher/k3s/kubelet-config.yaml ] && echo "ok" || echo "MISSING"
echo -n "rc.k3s script:    "; [ -x /etc/rc.d/rc.k3s ]         && echo "ok" || echo "MISSING"
echo -n "sudoers entry:    "; [ -f /etc/sudoers.d/dune-k3s ]  && echo "ok" || echo "MISSING"
echo -n "dune-vg swap:     "; swapon --show | grep -q 'dune-vg' && echo "ok" || echo "MISSING"
echo -n "backups mounted:  "; mountpoint -q "$BACKUPS_MOUNT"  && echo "ok" || echo "MISSING"

echo ""
ok "Root setup complete. Hand back to the dune user."
