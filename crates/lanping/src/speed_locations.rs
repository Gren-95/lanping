//! Places the speed test can look for M-Lab servers in.

use std::fmt;

use lanping_core::ndt7::Location;

/// Countries offered next to "nearest", by two letter code. M-Lab has no server in some of
/// them; asking then says so.
const COUNTRIES: [(&str, &str); 50] = [
    ("AR", "Argentina"),
    ("AU", "Australia"),
    ("AT", "Austria"),
    ("BE", "Belgium"),
    ("BR", "Brazil"),
    ("BG", "Bulgaria"),
    ("CA", "Canada"),
    ("CL", "Chile"),
    ("CO", "Colombia"),
    ("CZ", "Czechia"),
    ("DK", "Denmark"),
    ("EG", "Egypt"),
    ("FI", "Finland"),
    ("FR", "France"),
    ("DE", "Germany"),
    ("GR", "Greece"),
    ("HK", "Hong Kong"),
    ("HU", "Hungary"),
    ("IN", "India"),
    ("ID", "Indonesia"),
    ("IE", "Ireland"),
    ("IL", "Israel"),
    ("IT", "Italy"),
    ("JP", "Japan"),
    ("KE", "Kenya"),
    ("KR", "South Korea"),
    ("MY", "Malaysia"),
    ("MX", "Mexico"),
    ("NL", "Netherlands"),
    ("NZ", "New Zealand"),
    ("NG", "Nigeria"),
    ("NO", "Norway"),
    ("PK", "Pakistan"),
    ("PE", "Peru"),
    ("PH", "Philippines"),
    ("PL", "Poland"),
    ("PT", "Portugal"),
    ("RO", "Romania"),
    ("SA", "Saudi Arabia"),
    ("SG", "Singapore"),
    ("ZA", "South Africa"),
    ("ES", "Spain"),
    ("SE", "Sweden"),
    ("CH", "Switzerland"),
    ("TW", "Taiwan"),
    ("TH", "Thailand"),
    ("TR", "Turkey"),
    ("AE", "United Arab Emirates"),
    ("GB", "United Kingdom"),
    ("US", "United States"),
];

/// One entry of the location list: nearest, or a country.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocationChoice {
    code: Option<&'static str>,
    name: &'static str,
}

impl LocationChoice {
    pub const NEAREST: Self = Self {
        code: None,
        name: "Nearest to me",
    };

    /// Nearest first, then the countries by name.
    pub fn all() -> Vec<Self> {
        std::iter::once(Self::NEAREST)
            .chain(COUNTRIES.iter().map(|&(code, name)| Self {
                code: Some(code),
                name,
            }))
            .collect()
    }

    pub fn location(self) -> Location {
        match self.code {
            Some(code) => Location::Country(code.to_owned()),
            None => Location::Nearest,
        }
    }
}

impl Default for LocationChoice {
    fn default() -> Self {
        Self::NEAREST
    }
}

impl fmt::Display for LocationChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(code) => write!(f, "{} ({code})", self.name),
            None => f.write_str(self.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_comes_first_and_maps_to_the_nearest_location() {
        let all = LocationChoice::all();
        assert_eq!(all[0], LocationChoice::NEAREST);
        assert_eq!(all[0].location(), Location::Nearest);
        assert_eq!(all.len(), COUNTRIES.len() + 1);
    }

    #[test]
    fn a_country_maps_to_its_code() {
        let japan = LocationChoice::all()
            .into_iter()
            .find(|choice| choice.name == "Japan")
            .unwrap();
        assert_eq!(japan.location(), Location::Country("JP".to_owned()));
        assert_eq!(japan.to_string(), "Japan (JP)");
    }

    #[test]
    fn country_codes_are_two_capital_letters_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (code, _) in COUNTRIES {
            assert!(
                code.len() == 2 && code.chars().all(|c| c.is_ascii_uppercase()),
                "{code}"
            );
            assert!(seen.insert(code), "{code} listed twice");
        }
    }
}
