//! A domain's email records: where its mail goes (MX), who may send for it (SPF), what
//! receivers should do with forgeries (DMARC), and signing keys at common names (DKIM).
//!
//! Only ordinary DNS questions are asked. Each result comes with what it means in plain words.

use std::net::IpAddr;

use tokio::task::JoinSet;

use crate::dns::{DnsError, PUBLIC_RESOLVERS, RecordType, lookup, question_name, system_resolvers};
use crate::verdict::Verdict;

/// Names a DKIM key is often published under. A key under any other name is not found, so
/// finding none does not mean there is none.
pub const COMMON_DKIM_SELECTORS: [&str; 12] = [
    "default",
    "google",
    "selector1",
    "selector2",
    "k1",
    "k2",
    "mail",
    "dkim",
    "s1",
    "s2",
    "smtp",
    "mandrill",
];

/// Most DNS lookups an SPF record may cause before receivers give up on it.
const SPF_LOOKUP_LIMIT: usize = 10;
const DMARC_PREFIX: &str = "v=DMARC1";
const SPF_PREFIX: &str = "v=spf1";
const DKIM_MARKERS: [&str; 2] = ["v=DKIM1", "p="];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailServer {
    pub preference: u16,
    pub host: String,
}

/// One record checked, with what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordCheck {
    pub verdict: Verdict,
    pub record: Option<String>,
    pub meaning: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailReport {
    pub domain: String,
    pub mail_servers: Vec<MailServer>,
    pub spf: RecordCheck,
    pub dmarc: RecordCheck,
    /// The selectors under which a DKIM key was found.
    pub dkim_selectors: Vec<String>,
}

/// Looks up the email records of `domain`.
pub async fn check(domain: &str) -> Result<EmailReport, DnsError> {
    let domain = question_name(domain, RecordType::Mx)?;
    let server = resolver();
    let mail_servers = mail_servers(server, &domain).await?;
    let spf_records = texts(server, &domain).await?;
    let dmarc_records = texts(server, &format!("_dmarc.{domain}")).await?;
    Ok(EmailReport {
        mail_servers,
        spf: judge_spf(&spf_records),
        dmarc: judge_dmarc(&dmarc_records),
        dkim_selectors: dkim_selectors(server, &domain).await,
        domain,
    })
}

/// The resolver this computer uses, or Cloudflare's when none can be read.
fn resolver() -> IpAddr {
    system_resolvers()
        .first()
        .copied()
        .unwrap_or(PUBLIC_RESOLVERS[0].1)
}

/// A name that has no records of the kind is an empty list, not an error.
async fn answers(
    server: IpAddr,
    name: &str,
    record_type: RecordType,
) -> Result<Vec<String>, DnsError> {
    match lookup(server, name, record_type).await {
        Ok(answer) => Ok(answer
            .records
            .into_iter()
            .filter(|record| record.type_code == record_type.code())
            .map(|record| record.data)
            .collect()),
        Err(DnsError::NotFound) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

async fn texts(server: IpAddr, name: &str) -> Result<Vec<String>, DnsError> {
    answers(server, name, RecordType::Txt).await
}

async fn mail_servers(server: IpAddr, domain: &str) -> Result<Vec<MailServer>, DnsError> {
    let mut servers: Vec<MailServer> = answers(server, domain, RecordType::Mx)
        .await?
        .iter()
        .filter_map(|data| parse_mail_server(data))
        .collect();
    servers.sort_by_key(|server| server.preference);
    Ok(servers)
}

/// `10 mail.example.com` as a mail server.
fn parse_mail_server(data: &str) -> Option<MailServer> {
    let (preference, host) = data.split_once(' ')?;
    Some(MailServer {
        preference: preference.parse().ok()?,
        host: host.trim().to_owned(),
    })
}

async fn dkim_selectors(server: IpAddr, domain: &str) -> Vec<String> {
    let mut lookups = JoinSet::new();
    for selector in COMMON_DKIM_SELECTORS {
        let name = format!("{selector}._domainkey.{domain}");
        lookups.spawn(async move { (selector, texts(server, &name).await) });
    }
    let mut found = Vec::new();
    while let Some(Ok((selector, result))) = lookups.join_next().await {
        if result.is_ok_and(|records| records.iter().any(|record| is_dkim_key(record))) {
            found.push(selector.to_owned());
        }
    }
    found.sort_by_key(|selector| {
        COMMON_DKIM_SELECTORS
            .iter()
            .position(|known| known == selector)
    });
    found
}

fn is_dkim_key(record: &str) -> bool {
    DKIM_MARKERS.iter().any(|marker| record.contains(marker))
}

fn starts_with_ignoring_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

fn check_of(verdict: Verdict, record: Option<&str>, meaning: impl Into<String>) -> RecordCheck {
    RecordCheck {
        verdict,
        record: record.map(str::to_owned),
        meaning: meaning.into(),
    }
}

/// What the SPF record says about who may send mail for the domain.
fn judge_spf(texts: &[String]) -> RecordCheck {
    let records: Vec<&String> = texts
        .iter()
        .filter(|text| starts_with_ignoring_case(text.trim_start(), SPF_PREFIX))
        .collect();
    let record = match records.as_slice() {
        [] => {
            return check_of(
                Verdict::Missing,
                None,
                "No SPF record. Receivers cannot tell which servers may send mail for this \
                 domain, so forged mail is easier to pass off as its own.",
            );
        }
        [only] => only.trim(),
        _ => {
            return check_of(
                Verdict::Missing,
                Some(records[0].trim()),
                "More than one SPF record. Receivers treat that as an error and ignore SPF \
                 altogether. Merge them into one.",
            );
        }
    };
    let terms: Vec<&str> = record.split_whitespace().skip(1).collect();
    let lookups = terms.iter().filter(|term| causes_lookup(term)).count();
    if lookups > SPF_LOOKUP_LIMIT {
        return check_of(
            Verdict::Missing,
            Some(record),
            format!(
                "The record needs {lookups} DNS lookups and the limit is {SPF_LOOKUP_LIMIT}. \
                 Receivers treat SPF as an error. Remove includes or use plain addresses."
            ),
        );
    }
    let (verdict, meaning) = match all_term(&terms) {
        Some("-all") => (
            Verdict::Good,
            "Only the listed servers may send. Mail from any other is rejected.",
        ),
        Some("~all") => (
            Verdict::Weak,
            "Mail from servers not listed is marked suspicious but usually still delivered. \
             -all is stricter.",
        ),
        Some("?all") => (
            Verdict::Weak,
            "Neutral about servers not listed, so SPF protects nothing. Use -all or ~all.",
        ),
        Some("+all") | Some("all") => (
            Verdict::Missing,
            "Allows every server in the world to send for this domain. That cancels SPF out.",
        ),
        _ if terms.iter().any(|term| term.starts_with("redirect=")) => (
            Verdict::Good,
            "Uses the SPF record of another domain, which decides who may send.",
        ),
        _ => (
            Verdict::Weak,
            "Lists servers but has no rule for the rest. End the record with -all or ~all.",
        ),
    };
    check_of(verdict, Some(record), meaning)
}

/// Whether a term of an SPF record makes the receiver look something up in DNS.
fn causes_lookup(term: &str) -> bool {
    let term = term.trim_start_matches(['+', '-', '~', '?']);
    ["include:", "exists:", "redirect="]
        .iter()
        .any(|prefix| term.starts_with(prefix))
        || matches!(term.split([':', '/']).next(), Some("a" | "mx" | "ptr"))
}

/// The `all` term that ends the record, with its qualifier.
fn all_term<'a>(terms: &[&'a str]) -> Option<&'a str> {
    terms
        .iter()
        .copied()
        .find(|term| term.trim_start_matches(['+', '-', '~', '?']) == "all")
}

/// What the DMARC record asks receivers to do with mail that fails its checks.
fn judge_dmarc(texts: &[String]) -> RecordCheck {
    let Some(record) = texts
        .iter()
        .map(|text| text.trim())
        .find(|text| starts_with_ignoring_case(text, DMARC_PREFIX))
    else {
        return check_of(
            Verdict::Missing,
            None,
            "No DMARC record. Receivers have no instruction for mail that claims to be from \
             this domain but fails SPF and DKIM, and the owner gets no reports of it.",
        );
    };
    let tag = |name: &str| {
        record.split(';').find_map(|part| {
            let (key, value) = part.split_once('=')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
        })
    };
    let reports = if tag("rua").is_some() {
        ""
    } else {
        " It names no address for reports, so nobody hears about forgeries."
    };
    let partial = tag("pct")
        .and_then(|percent| percent.parse::<u8>().ok())
        .filter(|percent| *percent < 100)
        .map(|percent| format!(" It applies to only {percent} percent of mail."))
        .unwrap_or_default();
    let (verdict, meaning) = match tag("p").map(str::to_ascii_lowercase).as_deref() {
        Some("reject") => (
            Verdict::Good,
            "Mail that fails is rejected. This is the strongest policy.".to_owned(),
        ),
        Some("quarantine") => (
            Verdict::Good,
            "Mail that fails goes to spam. Reject is stricter once reports look clean.".to_owned(),
        ),
        Some("none") => (
            Verdict::Weak,
            "Monitoring only: mail that fails is still delivered. Move to quarantine or reject \
             when the reports look clean."
                .to_owned(),
        ),
        _ => (
            Verdict::Weak,
            "The record has no valid policy (p=), so receivers may ignore it.".to_owned(),
        ),
    };
    check_of(
        verdict,
        Some(record),
        format!("{meaning}{partial}{reports}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(list: &[&str]) -> Vec<String> {
        list.iter().map(|text| (*text).to_owned()).collect()
    }

    fn spf(record: &str) -> RecordCheck {
        judge_spf(&texts(&[record]))
    }

    #[test]
    fn mx_records_become_mail_servers_in_order_of_preference() {
        assert_eq!(
            parse_mail_server("10 mail.example.com"),
            Some(MailServer {
                preference: 10,
                host: "mail.example.com".to_owned()
            })
        );
        assert_eq!(parse_mail_server("x mail"), None);
        assert_eq!(
            parse_mail_server("0 ."),
            Some(MailServer {
                preference: 0,
                host: ".".to_owned()
            })
        );
    }

    #[test]
    fn a_domain_with_no_spf_record_or_two_has_a_problem() {
        let none = judge_spf(&texts(&["google-site-verification=abc"]));
        assert_eq!((none.verdict, none.record), (Verdict::Missing, None));
        let two = judge_spf(&texts(&["v=spf1 -all", "v=spf1 ~all"]));
        assert_eq!(two.verdict, Verdict::Missing);
        assert!(two.meaning.contains("More than one"));
    }

    #[test]
    fn the_end_of_an_spf_record_decides_how_strong_it_is() {
        assert_eq!(
            spf("v=spf1 include:_spf.google.com -all").verdict,
            Verdict::Good
        );
        assert_eq!(spf("v=spf1 mx ~all").verdict, Verdict::Weak);
        assert_eq!(spf("v=spf1 mx ?all").verdict, Verdict::Weak);
        assert_eq!(spf("v=spf1 mx +all").verdict, Verdict::Missing);
        assert_eq!(spf("v=spf1 mx").verdict, Verdict::Weak);
        assert_eq!(
            spf("v=spf1 redirect=_spf.example.net").verdict,
            Verdict::Good
        );
        assert_eq!(spf("V=SPF1 -all").verdict, Verdict::Good);
    }

    #[test]
    fn an_spf_record_with_too_many_lookups_is_an_error_to_receivers() {
        let many = format!("v=spf1 {} -all", "include:a.example ".repeat(11));
        let judged = spf(&many);
        assert_eq!(judged.verdict, Verdict::Missing);
        assert!(judged.meaning.contains("11"), "{}", judged.meaning);
        let enough = format!("v=spf1 {} -all", "include:a.example ".repeat(10));
        assert_eq!(spf(&enough).verdict, Verdict::Good);
    }

    #[test]
    fn only_some_terms_cost_a_lookup() {
        for costly in [
            "include:x",
            "a",
            "a:x.example",
            "mx",
            "mx/24",
            "ptr",
            "exists:%{i}.x",
            "redirect=x",
            "~a",
        ] {
            assert!(causes_lookup(costly), "{costly}");
        }
        for free in ["ip4:1.2.3.4", "ip6:::1", "-all", "all"] {
            assert!(!causes_lookup(free), "{free}");
        }
    }

    #[test]
    fn dmarc_is_graded_by_its_policy_and_notes_what_is_left_out() {
        let reject = judge_dmarc(&texts(&["v=DMARC1; p=reject; rua=mailto:r@example.com"]));
        assert_eq!(reject.verdict, Verdict::Good);
        assert!(!reject.meaning.contains("no address"));
        let monitor = judge_dmarc(&texts(&["v=DMARC1; p=none"]));
        assert_eq!(monitor.verdict, Verdict::Weak);
        assert!(monitor.meaning.contains("Monitoring") && monitor.meaning.contains("no address"));
        let partial = judge_dmarc(&texts(&[
            "v=DMARC1; p=quarantine; pct=25; rua=mailto:a@b.c",
        ]));
        assert_eq!(partial.verdict, Verdict::Good);
        assert!(partial.meaning.contains("25 percent"));
        assert_eq!(
            judge_dmarc(&texts(&["v=DMARC1; rua=mailto:a@b.c"])).verdict,
            Verdict::Weak
        );
        assert_eq!(judge_dmarc(&[]).verdict, Verdict::Missing);
    }

    #[test]
    fn a_dkim_key_is_told_by_its_version_or_its_public_key() {
        assert!(is_dkim_key("v=DKIM1; k=rsa; p=MIGf"));
        assert!(is_dkim_key("k=rsa; p=MIGf"));
        assert!(!is_dkim_key("v=spf1 -all"));
    }

    #[tokio::test]
    async fn a_bad_domain_is_refused_before_any_question_is_asked() {
        assert_eq!(check("").await.unwrap_err(), DnsError::InvalidName);
        assert_eq!(
            check("not a domain").await.unwrap_err(),
            DnsError::InvalidName
        );
    }
}
