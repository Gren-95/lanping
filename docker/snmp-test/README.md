# SNMP test device

A throwaway SNMP agent to try LanPing's **Traffic** tab against, without a real router.
It is a real net-snmp `snmpd` in a container whose `eth0` carries changing traffic, so the
rates and graphs move.

```sh
docker compose up --build
```

In LanPing, open the **Traffic** tab and enter:

| Field     | Value            |
|-----------|------------------|
| Address   | `127.0.0.1:1161` |
| Community | `lanping-test`   |

You should see `eth0` with a link speed and rates that step between about 1 and 100 Mbit/s
(5, 40, 100, 20 and 1 Mbit/s in turn, ten seconds each, in both directions). Tick
*Show all interfaces* to add `lo`.

Stop it with `docker compose down`.

## SNMP v3

The agent also has a v3 user for trying a v3 client later:

| Field            | Value               |
|------------------|---------------------|
| User             | `lanping`           |
| Authentication   | SHA, `lanping-auth-pass` |
| Privacy          | AES, `lanping-priv-pass` |

## Notes

- The ports are published on `127.0.0.1` only, so nothing outside this computer can reach the
  agent. The community and the v3 passwords are public knowledge: use this for testing only.
- With Podman, `podman build` and `podman run` work the same way (publish `1161:161/udp`).
- A command line check, if `snmpwalk` is installed:
  `snmpwalk -v2c -c lanping-test 127.0.0.1:1161 ifDescr`
