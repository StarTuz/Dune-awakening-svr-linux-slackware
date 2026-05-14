#!/usr/bin/env python3
"""Hold UDP 7779-7781 on the host so Dune game servers bind to 7782+ instead.
Path of Titans (192.168.254.100) owns 7777-7781 on the router; this prevents
Dune from claiming conflicting ports when its pods start."""
import socket, signal, sys, time

PORTS = [7779, 7780, 7781]
socks = []

for p in PORTS:
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(('0.0.0.0', p))
    socks.append(s)
    print(f'port-preempt: holding UDP {p}', flush=True)

print('port-preempt: all ports held', flush=True)

def cleanup(*_):
    for s in socks:
        s.close()
    sys.exit(0)

signal.signal(signal.SIGTERM, cleanup)
signal.signal(signal.SIGINT, cleanup)

while True:
    time.sleep(3600)
