//! The commands, and how they end.

mod dns;
mod email;
mod mtu;
mod ntp;
mod scan;
mod services;
mod tls;
mod upnp;
mod web;
mod wifi;

use crate::args::{ArgError, Parsed, Spec, parse};

/// How a command that ran ends.
#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    Fine,
    /// The check ran, and found something a script should act on.
    Problem,
}

/// Why a command could not run to the end.
#[derive(Debug)]
pub struct Failure {
    pub message: String,
    /// The command line was wrong, as against the check failing.
    pub usage: bool,
}

impl Failure {
    pub fn failed(message: impl ToString) -> Self {
        Self {
            message: message.to_string(),
            usage: false,
        }
    }

    pub fn usage(message: impl ToString) -> Self {
        Self {
            message: message.to_string(),
            usage: true,
        }
    }
}

impl From<ArgError> for Failure {
    fn from(error: ArgError) -> Self {
        Self::usage(error)
    }
}

pub type Outcome = Result<Exit, Failure>;

/// What `--help` says about a command.
pub struct Info {
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
}

/// Every command, in the order the help lists them.
pub const COMMANDS: [Info; 10] = [
    scan::INFO,
    wifi::INFO,
    dns::INFO,
    tls::INFO,
    ntp::INFO,
    mtu::INFO,
    web::INFO,
    email::INFO,
    upnp::INFO,
    services::INFO,
];

/// Switches every command accepts.
const JSON: &str = "json";

/// A spec with `--json` and the extra `switches` and `options`.
pub const fn spec(switches: &'static [&'static str], options: &'static [&'static str]) -> Spec {
    Spec { switches, options }
}

/// The one word a command needs, or a usage error saying what it is.
pub fn required<'a>(parsed: &'a Parsed, what: &str) -> Result<&'a str, Failure> {
    match parsed.positionals.as_slice() {
        [only] => Ok(only),
        [] => Err(Failure::usage(format!("missing {what}"))),
        _ => Err(Failure::usage("too many arguments")),
    }
}

pub fn wants_json(parsed: &Parsed) -> bool {
    parsed.switch(JSON)
}

/// Runs the command called `name` with `args`.
pub async fn run(name: &str, args: &[String]) -> Outcome {
    match name {
        "scan" => scan::run(parse(args, &scan::SPEC)?).await,
        "wifi" => wifi::run(parse(args, &wifi::SPEC)?).await,
        "dns" => dns::run(parse(args, &dns::SPEC)?).await,
        "tls" => tls::run(parse(args, &tls::SPEC)?).await,
        "ntp" => ntp::run(parse(args, &ntp::SPEC)?).await,
        "mtu" => mtu::run(parse(args, &mtu::SPEC)?).await,
        "web" => web::run(parse(args, &web::SPEC)?).await,
        "email" => email::run(parse(args, &email::SPEC)?).await,
        "upnp" => upnp::run(parse(args, &upnp::SPEC)?).await,
        "services" => services::run(parse(args, &services::SPEC)?).await,
        other => Err(Failure::usage(format!("unknown command {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_listed_can_be_run_by_name() {
        for info in &COMMANDS {
            // An unknown flag is refused by the command's own spec, which proves the name
            // is routed to a command and not to the unknown command error.
            let outcome = tokio_test_block(run(info.name, &["--no-such-flag".to_owned()]));
            let failure = outcome.unwrap_err();
            assert!(
                failure.usage && failure.message.contains("--no-such-flag"),
                "{}",
                info.name
            );
        }
    }

    fn tokio_test_block<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn an_unknown_command_is_a_usage_error() {
        let failure = tokio_test_block(run("nope", &[])).unwrap_err();
        assert!(failure.usage && failure.message.contains("nope"));
    }

    #[test]
    fn a_command_wants_exactly_one_word() {
        let none = Parsed::default();
        assert!(
            required(&none, "a host")
                .unwrap_err()
                .message
                .contains("missing a host")
        );
    }
}
