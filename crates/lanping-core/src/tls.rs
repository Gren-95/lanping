//! TLS certificate checker: connects to a host and port, reads the certificate chain the
//! server presents, and says whether the system would trust it.
//!
//! The handshake always goes on, so an expired, self-signed or mismatched certificate
//! can still be read and shown. The trust verdict comes from the normal verifier.

use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsFailure, RootCertStore,
    SignatureScheme,
};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::socket_budget::socket_permit;
use crate::tls_explain::{connect_failure, handshake_failure, silent_after_connect};
use crate::trace::validate_target;

/// The port used when a target names none: the one HTTPS uses.
pub const DEFAULT_PORT: u16 = 443;
const STEP_WAIT: Duration = Duration::from_secs(8);
const SECONDS_PER_DAY: i64 = 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsError {
    InvalidTarget,
    Timeout,
    /// The connection or the handshake failed before a certificate was seen.
    Failed(String),
}

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTarget => {
                f.write_str("Enter a host name or address, with a port if it is not 443")
            }
            Self::Timeout => f.write_str("The server did not answer in time. A firewall may be dropping packets to this port, or the host is down."),
            Self::Failed(reason) => f.write_str(reason),
        }
    }
}

/// Whether the system would trust the certificate chain, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    Trusted,
    /// The server signed its own certificate. Common on a LAN.
    SelfSigned,
    Problem(String),
}

/// What one certificate says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    pub subject: String,
    pub issuer: String,
    /// Unix time of the first moment it is valid.
    pub not_before: i64,
    /// Unix time of the last moment it is valid.
    pub not_after: i64,
    pub serial: String,
    /// Host names and addresses it is valid for.
    pub names: Vec<String>,
    pub self_signed: bool,
}

impl CertInfo {
    /// Whole days from `now` until it expires. Negative once it has expired.
    pub fn days_left(&self, now: i64) -> i64 {
        (self.not_after - now).div_euclid(SECONDS_PER_DAY)
    }

    fn parse(der: &[u8]) -> Option<Self> {
        let (_, certificate) = X509Certificate::from_der(der).ok()?;
        let names = certificate
            .subject_alternative_name()
            .ok()
            .flatten()
            .map(|extension| {
                extension
                    .value
                    .general_names
                    .iter()
                    .filter_map(general_name_text)
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            subject: certificate.subject().to_string(),
            issuer: certificate.issuer().to_string(),
            not_before: certificate.validity().not_before.timestamp(),
            not_after: certificate.validity().not_after.timestamp(),
            serial: certificate.raw_serial_as_string(),
            names,
            self_signed: certificate.subject().as_raw() == certificate.issuer().as_raw(),
        })
    }
}

fn general_name_text(name: &GeneralName<'_>) -> Option<String> {
    match name {
        GeneralName::DNSName(name) => Some((*name).to_owned()),
        GeneralName::IPAddress(bytes) => match bytes.len() {
            4 => <[u8; 4]>::try_from(*bytes)
                .ok()
                .map(|octets| std::net::Ipv4Addr::from(octets).to_string()),
            16 => <[u8; 16]>::try_from(*bytes)
                .ok()
                .map(|octets| std::net::Ipv6Addr::from(octets).to_string()),
            _ => None,
        },
        GeneralName::RFC822Name(name) | GeneralName::URI(name) => Some((*name).to_owned()),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsReport {
    pub host: String,
    pub port: u16,
    pub address: Option<SocketAddr>,
    /// The protocol version and cipher suite, when the handshake finished.
    pub protocol: Option<String>,
    pub cipher: Option<String>,
    /// The server's certificate first, then the ones that vouch for it.
    pub chain: Vec<CertInfo>,
    pub trust: Trust,
    /// Unix time of the check, for counting days left.
    pub checked_at: i64,
}

/// Reads `host`, `host:port`, `[v6]:port` or a bare IPv6 address. A missing port is 443.
pub fn parse_target(target: &str) -> Result<(String, u16), TlsError> {
    let target = target.trim();
    let invalid = || TlsError::InvalidTarget;
    let (host, port) = if let Some(rest) = target.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(invalid)?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port),
            None if after.is_empty() => None,
            None => return Err(invalid()),
        };
        (host, port)
    } else if target.matches(':').count() == 1 {
        let (host, port) = target.split_once(':').ok_or_else(invalid)?;
        (host, Some(port))
    } else {
        (target, None)
    };
    let host = validate_target(host).map_err(|_| invalid())?;
    let port = match port {
        Some(port) => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(invalid)?,
        None => DEFAULT_PORT,
    };
    Ok((host.to_owned(), port))
}

/// What the verifier saw during the handshake.
#[derive(Debug)]
struct Seen {
    chain: Vec<CertificateDer<'static>>,
    verdict: Result<(), TlsFailure>,
}

/// Asks the normal verifier, keeps its verdict and the chain, and lets the handshake go on.
#[derive(Debug)]
struct Inspecting {
    inner: Arc<WebPkiServerVerifier>,
    seen: Mutex<Option<Seen>>,
}

impl ServerCertVerifier for Inspecting {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsFailure> {
        let verdict = self
            .inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            .map(|_| ());
        let chain = std::iter::once(end_entity)
            .chain(intermediates)
            .map(|certificate| certificate.clone().into_owned())
            .collect();
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(Seen { chain, verdict });
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsFailure> {
        self.inner
            .verify_tls12_signature(message, certificate, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsFailure> {
        self.inner
            .verify_tls13_signature(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// The certificates the system trusts, read once.
pub(crate) async fn system_roots() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    if let Some(roots) = ROOTS.get() {
        return Arc::clone(roots);
    }
    let loaded = tokio::task::spawn_blocking(|| {
        let mut store = RootCertStore::empty();
        store.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
        Arc::new(store)
    })
    .await
    .unwrap_or_else(|_| Arc::new(RootCertStore::empty()));
    Arc::clone(ROOTS.get_or_init(|| loaded))
}

fn setup_error(error: impl fmt::Display) -> TlsError {
    TlsError::Failed(format!("Could not set up TLS: {error}"))
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// Connects to `target` (see [`parse_target`]) and reports the certificate it presents.
///
/// `server_name` is the host name to ask for, when it differs from the address connected
/// to. Servers that host several sites, such as Caddy, pick their certificate by that name
/// and refuse a connection that gives none. The certificate is checked against it too.
pub async fn check(target: &str, server_name: Option<&str>) -> Result<TlsReport, TlsError> {
    let (host, port) = parse_target(target)?;
    let name = match server_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => validate_target(name)
            .map_err(|_| TlsError::InvalidTarget)?
            .to_owned(),
        None => host.clone(),
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let inner =
        WebPkiServerVerifier::builder_with_provider(system_roots().await, Arc::clone(&provider))
            .build()
            .map_err(setup_error)?;
    let verifier = Arc::new(Inspecting {
        inner,
        seen: Mutex::new(None),
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(setup_error)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::clone(&verifier) as Arc<dyn ServerCertVerifier>)
        .with_no_client_auth();
    let server_name = ServerName::try_from(name.clone()).map_err(|_| TlsError::InvalidTarget)?;

    let _permit = socket_permit().await;
    let stream = timeout(STEP_WAIT, TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| TlsError::Timeout)?
        .map_err(|error| TlsError::Failed(connect_failure(&error, &host, port)))?;
    let address = stream.peer_addr().ok();
    let connector = TlsConnector::from(Arc::new(config));
    let Ok(handshake) = timeout(STEP_WAIT, connector.connect(server_name, stream)).await else {
        return Err(TlsError::Failed(
            silent_after_connect(&host, port, &name).await,
        ));
    };
    let (protocol, cipher, failure) = match handshake {
        Ok(stream) => {
            let (_, connection) = stream.get_ref();
            (
                connection
                    .protocol_version()
                    .map(|version| format!("{version:?}")),
                connection
                    .negotiated_cipher_suite()
                    .map(|suite| format!("{:?}", suite.suite())),
                None,
            )
        }
        Err(error) => (
            None,
            None,
            Some(handshake_failure(&error, &host, port, &name).await),
        ),
    };
    let seen = verifier
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let Some(seen) = seen else {
        return Err(TlsError::Failed(
            failure.unwrap_or_else(|| "The server sent no certificate".to_owned()),
        ));
    };
    let chain: Vec<CertInfo> = seen
        .chain
        .iter()
        .filter_map(|der| CertInfo::parse(der))
        .collect();
    let trust = trust_of(&seen.verdict, &chain);
    Ok(TlsReport {
        host,
        port,
        address,
        protocol,
        cipher,
        chain,
        trust,
        checked_at: unix_now(),
    })
}

/// A lone self-signed certificate is refused either as coming from an unknown issuer or,
/// when it is marked as a certificate authority (the default of `openssl req -x509`),
/// as an authority used as a server certificate.
fn is_untrusted_root(error: &TlsFailure) -> bool {
    match error {
        TlsFailure::InvalidCertificate(CertificateError::UnknownIssuer) => true,
        TlsFailure::InvalidCertificate(CertificateError::Other(other)) => {
            format!("{other:?}").contains("CaUsedAsEndEntity")
        }
        _ => false,
    }
}

fn trust_of(verdict: &Result<(), TlsFailure>, chain: &[CertInfo]) -> Trust {
    let problem = |text: &str| Trust::Problem(text.to_owned());
    match verdict {
        Ok(()) => Trust::Trusted,
        Err(error) if chain.len() == 1 && chain[0].self_signed && is_untrusted_root(error) => {
            Trust::SelfSigned
        }
        Err(TlsFailure::InvalidCertificate(CertificateError::UnknownIssuer)) => {
            problem("The issuer is not trusted by this system")
        }
        Err(TlsFailure::InvalidCertificate(
            CertificateError::Expired | CertificateError::ExpiredContext { .. },
        )) => problem("The certificate has expired"),
        Err(TlsFailure::InvalidCertificate(
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. },
        )) => problem("The certificate is not valid yet"),
        Err(TlsFailure::InvalidCertificate(
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
        )) => problem("The certificate is not valid for this name"),
        Err(TlsFailure::InvalidCertificate(CertificateError::Revoked)) => {
            problem("The certificate has been revoked")
        }
        Err(other) => Trust::Problem(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::ServerConfig;
    use rustls::pki_types::PrivateKeyDer;
    use rustls::server::{ClientHello, ResolvesServerCert};
    use rustls::sign::CertifiedKey;
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    /// A throwaway certificate made for these tests: CN lanping.test, valid for 100 years.
    const TEST_CERT: &[u8] = include_bytes!("../data/test/self-signed-cert.der");
    /// Its private key. It protects nothing.
    const TEST_KEY: &[u8] = include_bytes!("../data/test/self-signed-key.der");

    #[test]
    fn reads_the_parts_of_a_certificate() {
        let info = CertInfo::parse(TEST_CERT).unwrap();
        assert!(info.subject.contains("lanping.test"));
        assert_eq!(info.subject, info.issuer);
        assert!(info.self_signed);
        assert_eq!(info.names, ["lanping.test", "www.lanping.test", "10.0.0.1"]);
        assert!(info.not_after > info.not_before);
        assert!(!info.serial.is_empty());
    }

    #[test]
    fn counts_whole_days_and_goes_negative_after_expiry() {
        let mut info = CertInfo::parse(TEST_CERT).unwrap();
        info.not_after = 10 * SECONDS_PER_DAY + 5;
        assert_eq!(info.days_left(0), 10);
        assert_eq!(info.days_left(10 * SECONDS_PER_DAY + 6), -1);
    }

    #[test]
    fn reads_targets_with_and_without_ports() {
        let ok = |text: &str| parse_target(text).unwrap();
        assert_eq!(ok("example.com"), ("example.com".to_owned(), 443));
        assert_eq!(ok(" example.com:8443 "), ("example.com".to_owned(), 8443));
        assert_eq!(ok("192.168.0.1:993"), ("192.168.0.1".to_owned(), 993));
        assert_eq!(ok("[::1]:8443"), ("::1".to_owned(), 8443));
        assert_eq!(ok("[::1]"), ("::1".to_owned(), 443));
        assert_eq!(ok("fe80::1"), ("fe80::1".to_owned(), 443));
    }

    #[test]
    fn refuses_bad_targets() {
        for target in [
            "",
            "-x.com",
            "a b",
            "example.com:0",
            "example.com:70000",
            "example.com:x",
            "[::1",
            "[::1]x",
        ] {
            assert_eq!(
                parse_target(target),
                Err(TlsError::InvalidTarget),
                "{target}"
            );
        }
    }

    #[test]
    fn explains_each_kind_of_failure() {
        let invalid = |error| Err(TlsFailure::InvalidCertificate(error));
        let chain = [CertInfo::parse(TEST_CERT).unwrap()];
        assert_eq!(trust_of(&Ok(()), &chain), Trust::Trusted);
        assert_eq!(
            trust_of(&invalid(CertificateError::UnknownIssuer), &chain),
            Trust::SelfSigned
        );
        let mut signed_by_other = chain.clone();
        signed_by_other[0].self_signed = false;
        assert!(matches!(
            trust_of(&invalid(CertificateError::UnknownIssuer), &signed_by_other),
            Trust::Problem(text) if text.contains("issuer")
        ));
        assert!(matches!(
            trust_of(&invalid(CertificateError::Expired), &chain),
            Trust::Problem(text) if text.contains("expired")
        ));
        assert!(matches!(
            trust_of(&invalid(CertificateError::NotValidForName), &chain),
            Trust::Problem(text) if text.contains("name")
        ));
    }

    fn provider() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    /// Serves `config` on a free local port and returns that port.
    async fn serve(config: ServerConfig) -> u16 {
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let _ = acceptor.accept(stream).await;
                });
            }
        });
        port
    }

    /// Serves the test certificate to everyone.
    async fn serve_test_certificate() -> u16 {
        let key = PrivateKeyDer::try_from(TEST_KEY).unwrap().clone_key();
        let config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(TEST_CERT)], key)
            .unwrap();
        serve(config).await
    }

    /// Gives the test certificate only to clients that ask for `lanping.test`, as Caddy does.
    #[derive(Debug)]
    struct NamedOnly(Arc<CertifiedKey>);

    impl ResolvesServerCert for NamedOnly {
        fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            (hello.server_name() == Some("lanping.test")).then(|| Arc::clone(&self.0))
        }
    }

    async fn serve_for_one_name() -> u16 {
        let key = PrivateKeyDer::try_from(TEST_KEY).unwrap();
        let signing_key = rustls::crypto::ring::sign::any_supported_type(&key).unwrap();
        let certified = CertifiedKey::new(vec![CertificateDer::from(TEST_CERT)], signing_key);
        let config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(NamedOnly(Arc::new(certified))));
        serve(config).await
    }

    #[tokio::test]
    async fn checking_a_self_signed_server_reports_it_instead_of_failing() {
        let port = serve_test_certificate().await;
        let report = check(&format!("127.0.0.1:{port}"), None).await.unwrap();
        assert_eq!(report.trust, Trust::SelfSigned);
        assert_eq!(report.chain.len(), 1);
        assert!(report.chain[0].subject.contains("lanping.test"));
        assert_eq!(report.port, port);
        assert!(report.protocol.is_some());
        assert!(report.cipher.is_some());
        assert!(report.chain[0].days_left(report.checked_at) > 0);
    }

    #[tokio::test]
    async fn a_closed_port_is_an_error() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let result = check(&format!("127.0.0.1:{port}"), None).await;
        assert!(matches!(result, Err(TlsError::Failed(_))));
    }

    #[tokio::test]
    async fn a_server_that_needs_a_name_says_so_and_works_when_given_one() {
        let port = serve_for_one_name().await;
        let target = format!("127.0.0.1:{port}");
        let refused = check(&target, None).await.unwrap_err();
        let TlsError::Failed(message) = refused else {
            panic!("expected a handshake failure, got {refused:?}");
        };
        assert!(message.contains("refused the TLS handshake"), "{message}");
        assert!(message.contains("server name"), "{message}");
        let report = check(&target, Some("lanping.test")).await.unwrap();
        assert_eq!(report.trust, Trust::SelfSigned);
        assert!(report.chain[0].names.contains(&"lanping.test".to_owned()));
        assert_eq!(report.host, "127.0.0.1");
    }

    #[tokio::test]
    async fn a_port_that_does_not_speak_tls_is_explained() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream
                    .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
                    .await;
            }
        });
        let result = check(&format!("127.0.0.1:{port}"), None).await;
        let Err(TlsError::Failed(message)) = result else {
            panic!("expected a handshake failure, got {result:?}");
        };
        assert!(message.contains("a plain HTTP server"), "{message}");
    }

    #[test]
    fn a_bad_server_name_is_refused() {
        let result = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(check("127.0.0.1:1", Some("-x")));
        assert_eq!(result, Err(TlsError::InvalidTarget));
    }
}
