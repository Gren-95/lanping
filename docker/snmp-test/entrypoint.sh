#!/bin/sh
# Starts the traffic sink (iperf3), so the interface has something to count, and the agent.
set -e
iperf3 --server --daemon
exec snmpd -f -Lo -C -c /etc/snmp/snmpd.conf
