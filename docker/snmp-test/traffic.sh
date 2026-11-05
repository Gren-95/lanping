#!/bin/sh
# Sends traffic to the agent in steps, so the rates on its interface go up and down.
# Each step runs both directions at once, so In and Out both move.
set -u
host="${TARGET:-snmp}"
until iperf3 --client "$host" --time 1 >/dev/null 2>&1; do
    sleep 1
done
while true; do
    for rate in 5M 40M 100M 20M 1M; do
        iperf3 --client "$host" --time 10 --bitrate "$rate" --bidir >/dev/null 2>&1 || sleep 2
    done
done
