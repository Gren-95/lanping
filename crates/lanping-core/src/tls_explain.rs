//! Plain sentences for the ways a TLS check can fail, saying what happened and what to try.

use std::io::{Error, ErrorKind};
use std::net::IpAddr;

use rustls::{AlertDescription, Error as TlsFailure};

use crate::plain_service::{PlainService, identify, usual_use_of_port};

const NAME_HINT: &str = "Servers that host several sites, such as Caddy, need a server name: enter one in the second field.";

/// Why connecting to `host:port` failed.
pub(crate) fn connect_failure(error: &Error, host: &str, port: u16) -> String {
    match error.kind() {
        ErrorKind::ConnectionRefused => {
            format!("Connection refused: nothing is listening on port {port}.")
        }
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => {
            "There is no route to the host. Check the address and the network.".to_owned()
        }
        ErrorKind::TimedOut => {
            "The host did not answer. A firewall may be dropping packets to this port.".to_owned()
        }
        _ if is_name_lookup_failure(error) => format!("Could not find the host name \"{host}\"."),
        _ => format!("Could not connect: {error}"),
    }
}

fn is_name_lookup_failure(error: &Error) -> bool {
    let text = error.to_string().to_lowercase();
    [
        "lookup address",
        "name or service not known",
        "no address associated",
        "nodename nor servname",
        "name resolution",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

/// Why the TLS handshake with `host:port` failed. When the server does not seem to speak
/// TLS, it is asked what it does speak.
pub(crate) async fn handshake_failure(error: &Error, host: &str, port: u16, name: &str) -> String {
    let cause = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<TlsFailure>());
    match cause {
        Some(TlsFailure::AlertReceived(alert)) => alert_message(*alert, name),
        Some(TlsFailure::PeerIncompatible(reason)) => format!(
            "The server's TLS settings do not fit this checker ({reason:?}). It may only offer TLS older than 1.2, or weak ciphers."
        ),
        Some(TlsFailure::InvalidMessage(_)) => not_tls(host, port, name).await,
        _ if closed_early(error.kind()) => closed_without_tls(host, port, name).await,
        _ => error.to_string(),
    }
}

fn closed_early(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::UnexpectedEof
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::BrokenPipe
    )
}

fn alert_message(alert: AlertDescription, name: &str) -> String {
    let named_by_ip = name.parse::<IpAddr>().is_ok();
    match alert {
        AlertDescription::ProtocolVersion => {
            "The server only offers TLS versions older than 1.2, such as 1.0 or 1.1, which this checker does not use.".to_owned()
        }
        AlertDescription::CertificateRequired | AlertDescription::BadCertificate => {
            format!("The server wants a client certificate (mutual TLS), which this checker does not send ({alert:?}).")
        }
        AlertDescription::HandshakeFailure | AlertDescription::InsufficientSecurity => {
            let hint = if named_by_ip {
                format!(" It may also need a server name. {NAME_HINT}")
            } else {
                String::new()
            };
            format!("The server refused the TLS handshake ({alert:?}). It may share no TLS version or cipher with this checker.{hint}")
        }
        AlertDescription::InternalError
        | AlertDescription::UnrecognisedName
        | AlertDescription::AccessDenied
            if named_by_ip =>
        {
            format!("The server refused the TLS handshake ({alert:?}). {NAME_HINT}")
        }
        _ => format!("The server refused the TLS handshake ({alert:?})."),
    }
}

/// " Port 1883 is usually used for MQTT.", or nothing for a port with no usual use.
fn port_hint(port: u16) -> String {
    usual_use_of_port(port)
        .map(|usage| format!(" Port {port} is usually used for {usage}."))
        .unwrap_or_default()
}

/// The server answered the TLS greeting with something that is not TLS.
async fn not_tls(host: &str, port: u16, name: &str) -> String {
    match identify(host, port, name).await {
        PlainService::Silent => format!(
            "This port does not speak TLS. It answered with something that is not a TLS message.{}",
            port_hint(port)
        ),
        service @ PlainService::Http { .. } => format!(
            "This port does not speak TLS. It is {service}. Try http:// instead, or check the HTTPS port."
        ),
        service => format!("This port does not speak TLS. It is {service}."),
    }
}

/// The server hung up on the TLS greeting.
async fn closed_without_tls(host: &str, port: u16, name: &str) -> String {
    match identify(host, port, name).await {
        PlainService::Silent => format!(
            "The server closed the connection without a TLS answer. This port may not speak TLS, or a firewall or proxy may be rejecting the connection.{}",
            port_hint(port)
        ),
        service => format!(
            "The server closed the connection without a TLS answer. This port does not speak TLS: it is {service}."
        ),
    }
}

/// The server accepted the connection and then said nothing to the TLS greeting.
pub(crate) async fn silent_after_connect(host: &str, port: u16, name: &str) -> String {
    match identify(host, port, name).await {
        PlainService::Silent => format!(
            "The server accepted the connection but did not answer the TLS greeting. This port may not speak TLS.{}",
            port_hint(port)
        ),
        service => format!("This port does not speak TLS. It is {service}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[test]
    fn a_refused_connection_names_the_port() {
        let error = Error::new(ErrorKind::ConnectionRefused, "refused");
        assert_eq!(
            connect_failure(&error, "x", 8443),
            "Connection refused: nothing is listening on port 8443."
        );
    }

    #[test]
    fn a_failed_name_lookup_names_the_host() {
        let error = Error::other("failed to lookup address information: Name or service not known");
        assert_eq!(
            connect_failure(&error, "nope.invalid", 443),
            "Could not find the host name \"nope.invalid\"."
        );
    }

    #[test]
    fn unreachable_and_silent_hosts_are_explained() {
        let unreachable = Error::new(ErrorKind::HostUnreachable, "x");
        assert!(connect_failure(&unreachable, "x", 1).contains("no route"));
        let timed_out = Error::new(ErrorKind::TimedOut, "x");
        assert!(connect_failure(&timed_out, "x", 1).contains("firewall"));
    }

    #[test]
    fn alerts_suggest_a_server_name_only_for_ip_targets() {
        let by_ip = alert_message(AlertDescription::InternalError, "192.168.0.1");
        assert!(by_ip.contains("server name"), "{by_ip}");
        let by_name = alert_message(AlertDescription::InternalError, "nas.lan");
        assert!(!by_name.contains("server name"), "{by_name}");
        assert!(by_name.contains("InternalError"));
    }

    #[test]
    fn explains_old_protocols_client_certificates_and_missing_ciphers() {
        assert!(alert_message(AlertDescription::ProtocolVersion, "x").contains("older than 1.2"));
        assert!(
            alert_message(AlertDescription::CertificateRequired, "x")
                .contains("client certificate")
        );
        let handshake = alert_message(AlertDescription::HandshakeFailure, "10.0.0.1");
        assert!(handshake.contains("cipher") && handshake.contains("server name"));
    }

    #[tokio::test]
    async fn confirms_plain_http_with_the_server_and_redirect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream
                    .write_all(b"HTTP/1.1 308 Permanent Redirect\r\nLocation: https://x/\r\nServer: Caddy\r\n\r\n")
                    .await;
            }
        });
        let message = not_tls("127.0.0.1", port, "x").await;
        assert!(message.contains("plain HTTP server"), "{message}");
        assert!(message.contains("Server: Caddy"), "{message}");
        assert!(message.contains("redirects to https://x/"), "{message}");
        assert!(message.contains("Try http://"), "{message}");
    }

    #[tokio::test]
    async fn names_other_services_that_are_not_tls() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream.write_all(b"SSH-2.0-OpenSSH_9.6\r\n").await;
            }
        });
        let message = not_tls("127.0.0.1", port, "x").await;
        assert_eq!(
            message,
            "This port does not speak TLS. It is an SSH server (SSH-2.0-OpenSSH_9.6)."
        );
    }
}
