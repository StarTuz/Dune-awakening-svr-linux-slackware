#!/usr/bin/env python3
"""Historical extra guard for the old implicit Dune port-skip setup.

Dune now explicitly sets Port=7782 / IGWPort=7893 in UserEngine.ini, so this
should not be the primary mechanism that keeps game ports in the router-forwarded
range. It still holds 7779-7781 because Path of Titans owns that range on the
router and this makes regressions more obvious.
"""
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
