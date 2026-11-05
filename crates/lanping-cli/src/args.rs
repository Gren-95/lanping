//! A small argument reader: positional words, `--switch` flags and `--option value` pairs.
//! It is written by hand, so the command line needs no extra dependency.

use std::collections::HashMap;

/// The flags a command accepts.
pub struct Spec {
    /// Flags with no value, such as `--json`.
    pub switches: &'static [&'static str],
    /// Flags that take a value, such as `--ports 22,80`.
    pub options: &'static [&'static str],
}

#[derive(Debug, PartialEq, Eq)]
pub enum ArgError {
    UnknownFlag(String),
    MissingValue(String),
    UnexpectedValue(String),
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFlag(flag) => write!(f, "unknown option {flag}"),
            Self::MissingValue(flag) => write!(f, "{flag} needs a value"),
            Self::UnexpectedValue(flag) => write!(f, "{flag} does not take a value"),
        }
    }
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub positionals: Vec<String>,
    switches: Vec<String>,
    options: HashMap<String, String>,
}

impl Parsed {
    pub fn switch(&self, name: &str) -> bool {
        self.switches.iter().any(|known| known == name)
    }

    pub fn option(&self, name: &str) -> Option<&str> {
        self.options.get(name).map(String::as_str)
    }

    /// The only positional word, or the first of them.
    pub fn first(&self) -> Option<&str> {
        self.positionals.first().map(String::as_str)
    }
}

/// Reads `args` against `spec`. After a bare `--` everything is a positional word.
pub fn parse(args: &[String], spec: &Spec) -> Result<Parsed, ArgError> {
    let mut parsed = Parsed::default();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            parsed.positionals.extend(words.by_ref().cloned());
            break;
        }
        let Some(flag) = word.strip_prefix("--") else {
            parsed.positionals.push(word.clone());
            continue;
        };
        let (name, inline) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (flag, None),
        };
        if spec.switches.contains(&name) {
            if inline.is_some() {
                return Err(ArgError::UnexpectedValue(format!("--{name}")));
            }
            parsed.switches.push(name.to_owned());
        } else if spec.options.contains(&name) {
            let value = match inline {
                Some(value) => value,
                None => words
                    .next()
                    .cloned()
                    .ok_or_else(|| ArgError::MissingValue(format!("--{name}")))?,
            };
            parsed.options.insert(name.to_owned(), value);
        } else {
            return Err(ArgError::UnknownFlag(word.clone()));
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec = Spec {
        switches: &["json", "banners"],
        options: &["ports", "type"],
    };

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn words_switches_and_options_are_told_apart_in_any_order() {
        let parsed = parse(
            &words("192.168.0.0/24 --json --ports 22,80 --type=MX extra"),
            &SPEC,
        )
        .unwrap();
        assert_eq!(parsed.positionals, ["192.168.0.0/24", "extra"]);
        assert!(parsed.switch("json") && !parsed.switch("banners"));
        assert_eq!(parsed.option("ports"), Some("22,80"));
        assert_eq!(parsed.option("type"), Some("MX"));
        assert_eq!(parsed.first(), Some("192.168.0.0/24"));
    }

    #[test]
    fn everything_after_a_double_dash_is_a_word() {
        let parsed = parse(&words("-- --json -x"), &SPEC).unwrap();
        assert_eq!(parsed.positionals, ["--json", "-x"]);
        assert!(!parsed.switch("json"));
    }

    #[test]
    fn mistakes_are_named() {
        assert_eq!(
            parse(&words("--nope"), &SPEC).unwrap_err(),
            ArgError::UnknownFlag("--nope".to_owned())
        );
        assert_eq!(
            parse(&words("--ports"), &SPEC).unwrap_err(),
            ArgError::MissingValue("--ports".to_owned())
        );
        assert_eq!(
            parse(&words("--json=yes"), &SPEC).unwrap_err(),
            ArgError::UnexpectedValue("--json".to_owned())
        );
        assert!(
            ArgError::UnknownFlag("--x".to_owned())
                .to_string()
                .contains("--x")
        );
    }

    #[test]
    fn a_lone_dash_is_a_word() {
        assert_eq!(parse(&words("-"), &SPEC).unwrap().positionals, ["-"]);
    }
}
