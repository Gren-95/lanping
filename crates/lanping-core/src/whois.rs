//! WHOIS lookups for domain names and IP addresses over TCP port 43.
//!
//! IANA is asked first, since it knows which registry is responsible. If that
//! registry's answer points to a registrar's own server, that one is asked too.

use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::dns::{RecordType, question_name};
use crate::socket_budget::socket_permit;

const WHOIS_PORT: u16 = 43;
const IANA_SERVER: &str = "whois.iana.org";
const SERVER_WAIT: Duration = Duration::from_secs(8);
/// Longest answer read from a server; real answers are far shorter.
const MAX_RESPONSE_BYTES: u64 = 256 * 1024;
const MAX_NAME_SERVERS: usize = 6;
/// Keys whose value names the server to ask next, as written in the answers.
const IANA_REFERRAL_KEYS: [&str; 2] = ["refer", "whois"];
const REGISTRY_REFERRAL_KEYS: [&str; 2] = ["registrar whois server", "referralserver"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhoisError {
    InvalidQuery,
    /// IANA did not name a server for this query.
    NoServer,
    Timeout,
    Network(String),
}

impl fmt::Display for WhoisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuery => f.write_str("Enter a domain name or an IP address"),
            Self::NoServer => f.write_str("No WHOIS server is known for this name"),
            Self::Timeout => f.write_str(
                "The WHOIS server did not answer in time. Port 43 may be blocked on this network",
            ),
            Self::Network(reason) => write!(f, "Could not reach the WHOIS server: {reason}"),
        }
    }
}

/// One line of the short summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRow {
    pub label: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoisReport {
    /// The server that gave the answer in `text`.
    pub server: String,
    pub text: String,
    pub summary: Vec<SummaryRow>,
}

enum Target {
    Address(IpAddr),
    Domain(String),
}

impl Target {
    fn parse(input: &str) -> Result<Self, WhoisError> {
        let input = input.trim();
        if let Ok(address) = input.parse::<IpAddr>() {
            return Ok(Self::Address(address));
        }
        question_name(input, RecordType::A)
            .map(Self::Domain)
            .map_err(|_| WhoisError::InvalidQuery)
    }

    /// What the registry is asked.
    fn question(&self) -> String {
        match self {
            Self::Address(address) => address.to_string(),
            Self::Domain(name) => name.clone(),
        }
    }

    /// What IANA is asked: the address, or the last label of a name.
    fn iana_question(&self) -> String {
        match self {
            Self::Address(address) => address.to_string(),
            Self::Domain(name) => name.rsplit('.').next().unwrap_or(name).to_owned(),
        }
    }
}

/// Looks up `input`, a domain name or an IP address.
pub async fn whois(input: &str) -> Result<WhoisReport, WhoisError> {
    let target = Target::parse(input)?;
    let iana = exchange(IANA_SERVER, &target.iana_question()).await?;
    let mut server = referral(&iana, &IANA_REFERRAL_KEYS).ok_or(WhoisError::NoServer)?;
    let mut text = exchange(&server, &target.question()).await?;
    if let Some(next) = referral(&text, &REGISTRY_REFERRAL_KEYS)
        && next != server
        && let Ok(more) = exchange(&next, &target.question()).await
    {
        server = next;
        text = more;
    }
    Ok(WhoisReport {
        server,
        summary: summarize(&text),
        text,
    })
}

async fn exchange(server: &str, question: &str) -> Result<String, WhoisError> {
    let _permit = socket_permit().await;
    let run = async {
        let mut stream = TcpStream::connect((server, WHOIS_PORT)).await?;
        stream
            .write_all(format!("{question}\r\n").as_bytes())
            .await?;
        let mut answer = Vec::new();
        stream
            .take(MAX_RESPONSE_BYTES)
            .read_to_end(&mut answer)
            .await?;
        Ok::<_, std::io::Error>(answer)
    };
    match timeout(SERVER_WAIT, run).await {
        Err(_) => Err(WhoisError::Timeout),
        Ok(Err(error)) => Err(WhoisError::Network(error.to_string())),
        Ok(Ok(answer)) => Ok(String::from_utf8_lossy(&answer).into_owned()),
    }
}

/// Key and value of every `key: value` line, skipping comments and empty values.
/// Keys are lowercased.
fn fields(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.starts_with(['%', '#']) && !line.starts_with(">>>"))
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_lowercase(), value.trim().to_owned()))
        .filter(|(_, value)| !value.is_empty())
        .collect()
}

/// The server named by the first of `keys` found in `text`. The value may be a bare
/// host name, or a `whois://host:43` address. Addresses and odd names are refused, so
/// a server cannot send this client to an arbitrary machine.
fn referral(text: &str, keys: &[&str]) -> Option<String> {
    fields(text)
        .into_iter()
        .filter(|(key, _)| keys.contains(&key.as_str()))
        .find_map(|(_, value)| server_name(&value))
}

fn server_name(value: &str) -> Option<String> {
    let value = value.strip_prefix("whois://").unwrap_or(value);
    let host = value.split(['/', ':']).next()?.trim().to_lowercase();
    if host.parse::<IpAddr>().is_ok() {
        return None;
    }
    question_name(&host, RecordType::A).ok()
}

enum Pick {
    First,
    FirstWord,
    AllLowercase,
}

struct Field {
    label: &'static str,
    keys: &'static [&'static str],
    pick: Pick,
}

const FIELDS: [Field; 10] = [
    Field {
        label: "Registrar",
        keys: &["registrar"],
        pick: Pick::First,
    },
    Field {
        label: "Organization",
        keys: &[
            "registrant organization",
            "orgname",
            "org-name",
            "organization",
            "organisation",
            "owner",
        ],
        pick: Pick::First,
    },
    Field {
        label: "Created",
        keys: &["creation date", "created", "registered"],
        pick: Pick::First,
    },
    Field {
        label: "Updated",
        keys: &["updated date", "last updated", "last-modified", "changed"],
        pick: Pick::First,
    },
    Field {
        label: "Expires",
        keys: &[
            "registry expiry date",
            "registrar registration expiration date",
            "expiry date",
            "expiration date",
            "paid-till",
        ],
        pick: Pick::First,
    },
    Field {
        label: "Status",
        keys: &["domain status"],
        pick: Pick::FirstWord,
    },
    Field {
        label: "Name servers",
        keys: &["name server", "nserver"],
        pick: Pick::AllLowercase,
    },
    Field {
        label: "Network name",
        keys: &["netname"],
        pick: Pick::First,
    },
    Field {
        label: "Range",
        keys: &["cidr", "netrange", "inetnum"],
        pick: Pick::First,
    },
    Field {
        label: "Country",
        keys: &["country"],
        pick: Pick::First,
    },
];

/// The facts people usually look for, taken from the answer in whatever format that
/// registry uses. Facts that are not in the answer are left out.
pub fn summarize(text: &str) -> Vec<SummaryRow> {
    let pairs = fields(text);
    FIELDS
        .iter()
        .filter_map(|field| {
            let mut values = pairs
                .iter()
                .filter(|(key, _)| field.keys.contains(&key.as_str()))
                .map(|(_, value)| value.as_str());
            let value = match field.pick {
                Pick::First => values.next()?.to_owned(),
                Pick::FirstWord => values.next()?.split_whitespace().next()?.to_owned(),
                Pick::AllLowercase => {
                    let mut servers: Vec<String> = Vec::new();
                    for value in values {
                        let value = value.split_whitespace().next()?.to_lowercase();
                        if !servers.contains(&value) {
                            servers.push(value);
                        }
                    }
                    servers.truncate(MAX_NAME_SERVERS);
                    (!servers.is_empty()).then(|| servers.join(", "))?
                }
            };
            Some(SummaryRow {
                label: field.label,
                value,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN_ANSWER: &str = "\
   Domain Name: EXAMPLE.COM
   Registrar WHOIS Server: whois.markmonitor.com
   Updated Date: 2026-08-14T08:01:43Z
   Creation Date: 1995-08-14T04:00:00Z
   Registry Expiry Date: 2027-08-13T04:00:00Z
   Registrar: RESERVED-Internet Assigned Numbers Authority
   Registrar Abuse Contact Email:
   Domain Status: clientDeleteProhibited https://icann.org/epp#clientDeleteProhibited
   Domain Status: clientTransferProhibited https://icann.org/epp#clientTransferProhibited
   Name Server: ELLIOTT.NS.CLOUDFLARE.COM
   Name Server: HERA.NS.CLOUDFLARE.COM
   Name Server: hera.ns.cloudflare.com
>>> Last update of whois database: 2026-11-05T16:41:48Z <<<
NOTICE: this is not a field
";

    const NETWORK_ANSWER: &str = "\
% Information related to '1.1.1.0 - 1.1.1.255'
inetnum:        1.1.1.0 - 1.1.1.255
netname:        APNIC-LABS
country:        AU
descr:          APNIC and Cloudflare DNS Resolver project
";

    #[test]
    fn summarizes_a_domain_answer() {
        let rows = summarize(DOMAIN_ANSWER);
        let get = |label: &str| {
            rows.iter()
                .find(|row| row.label == label)
                .map(|row| row.value.as_str())
        };
        assert_eq!(
            get("Registrar"),
            Some("RESERVED-Internet Assigned Numbers Authority")
        );
        assert_eq!(get("Created"), Some("1995-08-14T04:00:00Z"));
        assert_eq!(get("Expires"), Some("2027-08-13T04:00:00Z"));
        assert_eq!(get("Status"), Some("clientDeleteProhibited"));
        assert_eq!(
            get("Name servers"),
            Some("elliott.ns.cloudflare.com, hera.ns.cloudflare.com")
        );
        assert_eq!(get("Network name"), None);
    }

    #[test]
    fn summarizes_a_network_answer() {
        let rows = summarize(NETWORK_ANSWER);
        let labels: Vec<&str> = rows.iter().map(|row| row.label).collect();
        assert_eq!(labels, ["Network name", "Range", "Country"]);
        assert_eq!(rows[1].value, "1.1.1.0 - 1.1.1.255");
    }

    #[test]
    fn finds_the_registrar_server_to_ask_next() {
        assert_eq!(
            referral(DOMAIN_ANSWER, &REGISTRY_REFERRAL_KEYS).as_deref(),
            Some("whois.markmonitor.com")
        );
    }

    #[test]
    fn finds_the_registry_in_the_iana_answer() {
        let iana = "% IANA WHOIS server\nrefer:        whois.apnic.net\n\ninetnum: 1.0.0.0 - 1.255.255.255\nwhois:        whois.apnic.net\n";
        assert_eq!(
            referral(iana, &IANA_REFERRAL_KEYS).as_deref(),
            Some("whois.apnic.net")
        );
        assert_eq!(referral("nothing useful", &IANA_REFERRAL_KEYS), None);
    }

    #[test]
    fn reads_server_names_in_url_form_and_refuses_addresses() {
        assert_eq!(
            server_name("whois://whois.ripe.net:43").as_deref(),
            Some("whois.ripe.net")
        );
        assert_eq!(
            server_name("WHOIS.Example.COM").as_deref(),
            Some("whois.example.com")
        );
        assert_eq!(server_name("192.168.0.1"), None);
        assert_eq!(server_name("bad host"), None);
        assert_eq!(server_name(""), None);
    }

    #[test]
    fn asks_iana_about_the_last_label_of_a_name() {
        let domain = Target::parse("www.Example.com.").unwrap();
        assert_eq!(domain.iana_question(), "com");
        assert_eq!(domain.question(), "www.Example.com");
        let address = Target::parse(" 1.1.1.1 ").unwrap();
        assert_eq!(address.iana_question(), "1.1.1.1");
    }

    #[test]
    fn refuses_bad_queries() {
        for input in ["", "  ", "-x.com", "a b", "x;y"] {
            assert!(
                matches!(Target::parse(input), Err(WhoisError::InvalidQuery)),
                "{input}"
            );
        }
    }
}
