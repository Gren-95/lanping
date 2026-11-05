# A throwaway SNMP device for trying LanPing's Traffic tab: a real net-snmp agent whose
# network interface carries traffic. Not for production: the community is public knowledge.
FROM alpine:3.20

RUN apk add --no-cache net-snmp iperf3

COPY snmpd.conf /etc/snmp/snmpd.conf
COPY entrypoint.sh traffic.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/entrypoint.sh /usr/local/bin/traffic.sh

EXPOSE 161/udp 5201/tcp
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
