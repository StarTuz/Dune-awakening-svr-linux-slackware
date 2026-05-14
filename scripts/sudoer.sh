#!/bin/bash
! sed -i '/^dune ALL.*NOPASSWD.*kubectl/d;/rc-service.*k3s-killall/d' /etc/sudoers && echo 'dune ALL=(ALL)
NOPASSWD: /usr/local/bin/kubectl, /usr/local/bin/ctr, /usr/local/bin/k3s, /usr/local/bin/rc-service,
/usr/local/bin/k3s-killall.sh, /etc/rc.d/rc.k3s' >> /etc/sudoers && rm -f /var/run/k3s.pid &&
/etc/rc.d/rc.k3s start
