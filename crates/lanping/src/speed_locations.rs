//! Places the speed test can look for M-Lab servers in.

use std::fmt;

use lanping_core::ndt7::Location;

/// Every country, by two letter code. M-Lab has servers in about fifty of them; for the
/// others it offers the nearest servers, and each server shows the city it is in.
const COUNTRIES: [(&str, &str); 249] = [
    ("AF", "Afghanistan"),
    ("AL", "Albania"),
    ("DZ", "Algeria"),
    ("AD", "Andorra"),
    ("AO", "Angola"),
    ("AI", "Anguilla"),
    ("AQ", "Antarctica"),
    ("AG", "Antigua & Barbuda"),
    ("AR", "Argentina"),
    ("AM", "Armenia"),
    ("AW", "Aruba"),
    ("AU", "Australia"),
    ("AT", "Austria"),
    ("AZ", "Azerbaijan"),
    ("BS", "Bahamas"),
    ("BH", "Bahrain"),
    ("BD", "Bangladesh"),
    ("BB", "Barbados"),
    ("BY", "Belarus"),
    ("BE", "Belgium"),
    ("BZ", "Belize"),
    ("BJ", "Benin"),
    ("BM", "Bermuda"),
    ("BT", "Bhutan"),
    ("BO", "Bolivia"),
    ("BA", "Bosnia & Herzegovina"),
    ("BW", "Botswana"),
    ("BV", "Bouvet Island"),
    ("BR", "Brazil"),
    ("IO", "British Indian Ocean Territory"),
    ("BN", "Brunei"),
    ("BG", "Bulgaria"),
    ("BF", "Burkina Faso"),
    ("BI", "Burundi"),
    ("KH", "Cambodia"),
    ("CM", "Cameroon"),
    ("CA", "Canada"),
    ("CV", "Cape Verde"),
    ("BQ", "Caribbean NL"),
    ("KY", "Cayman Islands"),
    ("CF", "Central African Rep."),
    ("TD", "Chad"),
    ("CL", "Chile"),
    ("CN", "China"),
    ("CX", "Christmas Island"),
    ("CC", "Cocos (Keeling) Islands"),
    ("CO", "Colombia"),
    ("KM", "Comoros"),
    ("CD", "Congo (Dem. Rep.)"),
    ("CG", "Congo (Rep.)"),
    ("CK", "Cook Islands"),
    ("CR", "Costa Rica"),
    ("HR", "Croatia"),
    ("CU", "Cuba"),
    ("CW", "Curaçao"),
    ("CY", "Cyprus"),
    ("CZ", "Czechia"),
    ("CI", "Côte d’Ivoire"),
    ("DK", "Denmark"),
    ("DJ", "Djibouti"),
    ("DM", "Dominica"),
    ("DO", "Dominican Republic"),
    ("TL", "East Timor"),
    ("EC", "Ecuador"),
    ("EG", "Egypt"),
    ("SV", "El Salvador"),
    ("GQ", "Equatorial Guinea"),
    ("ER", "Eritrea"),
    ("EE", "Estonia"),
    ("SZ", "Eswatini (Swaziland)"),
    ("ET", "Ethiopia"),
    ("FK", "Falkland Islands"),
    ("FO", "Faroe Islands"),
    ("FJ", "Fiji"),
    ("FI", "Finland"),
    ("FR", "France"),
    ("GF", "French Guiana"),
    ("PF", "French Polynesia"),
    ("TF", "French S. Terr."),
    ("GA", "Gabon"),
    ("GM", "Gambia"),
    ("GE", "Georgia"),
    ("DE", "Germany"),
    ("GH", "Ghana"),
    ("GI", "Gibraltar"),
    ("GR", "Greece"),
    ("GL", "Greenland"),
    ("GD", "Grenada"),
    ("GP", "Guadeloupe"),
    ("GU", "Guam"),
    ("GT", "Guatemala"),
    ("GG", "Guernsey"),
    ("GN", "Guinea"),
    ("GW", "Guinea-Bissau"),
    ("GY", "Guyana"),
    ("HT", "Haiti"),
    ("HM", "Heard Island & McDonald Islands"),
    ("HN", "Honduras"),
    ("HK", "Hong Kong"),
    ("HU", "Hungary"),
    ("IS", "Iceland"),
    ("IN", "India"),
    ("ID", "Indonesia"),
    ("IR", "Iran"),
    ("IQ", "Iraq"),
    ("IE", "Ireland"),
    ("IM", "Isle of Man"),
    ("IL", "Israel"),
    ("IT", "Italy"),
    ("JM", "Jamaica"),
    ("JP", "Japan"),
    ("JE", "Jersey"),
    ("JO", "Jordan"),
    ("KZ", "Kazakhstan"),
    ("KE", "Kenya"),
    ("KI", "Kiribati"),
    ("KW", "Kuwait"),
    ("KG", "Kyrgyzstan"),
    ("LA", "Laos"),
    ("LV", "Latvia"),
    ("LB", "Lebanon"),
    ("LS", "Lesotho"),
    ("LR", "Liberia"),
    ("LY", "Libya"),
    ("LI", "Liechtenstein"),
    ("LT", "Lithuania"),
    ("LU", "Luxembourg"),
    ("MO", "Macau"),
    ("MG", "Madagascar"),
    ("MW", "Malawi"),
    ("MY", "Malaysia"),
    ("MV", "Maldives"),
    ("ML", "Mali"),
    ("MT", "Malta"),
    ("MH", "Marshall Islands"),
    ("MQ", "Martinique"),
    ("MR", "Mauritania"),
    ("MU", "Mauritius"),
    ("YT", "Mayotte"),
    ("MX", "Mexico"),
    ("FM", "Micronesia"),
    ("MD", "Moldova"),
    ("MC", "Monaco"),
    ("MN", "Mongolia"),
    ("ME", "Montenegro"),
    ("MS", "Montserrat"),
    ("MA", "Morocco"),
    ("MZ", "Mozambique"),
    ("MM", "Myanmar (Burma)"),
    ("NA", "Namibia"),
    ("NR", "Nauru"),
    ("NP", "Nepal"),
    ("NL", "Netherlands"),
    ("NC", "New Caledonia"),
    ("NZ", "New Zealand"),
    ("NI", "Nicaragua"),
    ("NE", "Niger"),
    ("NG", "Nigeria"),
    ("NU", "Niue"),
    ("NF", "Norfolk Island"),
    ("KP", "North Korea"),
    ("MK", "North Macedonia"),
    ("MP", "Northern Mariana Islands"),
    ("NO", "Norway"),
    ("OM", "Oman"),
    ("PK", "Pakistan"),
    ("PW", "Palau"),
    ("PS", "Palestine"),
    ("PA", "Panama"),
    ("PG", "Papua New Guinea"),
    ("PY", "Paraguay"),
    ("PE", "Peru"),
    ("PH", "Philippines"),
    ("PN", "Pitcairn"),
    ("PL", "Poland"),
    ("PT", "Portugal"),
    ("PR", "Puerto Rico"),
    ("QA", "Qatar"),
    ("RO", "Romania"),
    ("RU", "Russia"),
    ("RW", "Rwanda"),
    ("RE", "Réunion"),
    ("AS", "Samoa (American)"),
    ("WS", "Samoa (western)"),
    ("SM", "San Marino"),
    ("ST", "Sao Tome & Principe"),
    ("SA", "Saudi Arabia"),
    ("SN", "Senegal"),
    ("RS", "Serbia"),
    ("SC", "Seychelles"),
    ("SL", "Sierra Leone"),
    ("SG", "Singapore"),
    ("SK", "Slovakia"),
    ("SI", "Slovenia"),
    ("SB", "Solomon Islands"),
    ("SO", "Somalia"),
    ("ZA", "South Africa"),
    ("GS", "South Georgia & the South Sandwich Islands"),
    ("KR", "South Korea"),
    ("SS", "South Sudan"),
    ("ES", "Spain"),
    ("LK", "Sri Lanka"),
    ("BL", "St Barthelemy"),
    ("SH", "St Helena"),
    ("KN", "St Kitts & Nevis"),
    ("LC", "St Lucia"),
    ("SX", "St Maarten (Dutch)"),
    ("MF", "St Martin (French)"),
    ("PM", "St Pierre & Miquelon"),
    ("VC", "St Vincent"),
    ("SD", "Sudan"),
    ("SR", "Suriname"),
    ("SJ", "Svalbard & Jan Mayen"),
    ("SE", "Sweden"),
    ("CH", "Switzerland"),
    ("SY", "Syria"),
    ("TW", "Taiwan"),
    ("TJ", "Tajikistan"),
    ("TZ", "Tanzania"),
    ("TH", "Thailand"),
    ("TG", "Togo"),
    ("TK", "Tokelau"),
    ("TO", "Tonga"),
    ("TT", "Trinidad & Tobago"),
    ("TN", "Tunisia"),
    ("TR", "Turkey"),
    ("TM", "Turkmenistan"),
    ("TC", "Turks & Caicos Is"),
    ("TV", "Tuvalu"),
    ("UG", "Uganda"),
    ("UA", "Ukraine"),
    ("AE", "United Arab Emirates"),
    ("GB", "United Kingdom"),
    ("US", "United States"),
    ("UY", "Uruguay"),
    ("UM", "US minor outlying islands"),
    ("UZ", "Uzbekistan"),
    ("VU", "Vanuatu"),
    ("VA", "Vatican City"),
    ("VE", "Venezuela"),
    ("VN", "Vietnam"),
    ("VG", "Virgin Islands (UK)"),
    ("VI", "Virgin Islands (US)"),
    ("WF", "Wallis & Futuna"),
    ("EH", "Western Sahara"),
    ("YE", "Yemen"),
    ("ZM", "Zambia"),
    ("ZW", "Zimbabwe"),
    ("AX", "Åland Islands"),
];

/// Countries known to have a working M-Lab test server. Used until M-Lab's own list arrives,
/// and when it cannot be fetched.
const COUNTRIES_WITH_SERVERS: [&str; 48] = [
    "AR", "AU", "BE", "BG", "BR", "CA", "CH", "CL", "CO", "CZ", "DE", "ES", "FI", "FR", "GB", "GR",
    "HK", "ID", "IE", "IL", "IN", "IT", "JP", "KE", "KR", "ME", "MG", "MN", "MX", "MZ", "NG", "NL",
    "NO", "NP", "NZ", "PE", "PH", "PK", "PL", "PT", "QA", "RS", "SE", "SG", "TN", "TW", "US", "ZA",
];

/// The name of a country, when it is known.
fn country_name(code: &str) -> Option<&'static str> {
    COUNTRIES
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, name)| *name)
}

/// One entry of the location list: nearest, or a country that has a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationChoice {
    code: Option<String>,
    name: String,
}

impl LocationChoice {
    pub fn nearest() -> Self {
        Self {
            code: None,
            name: "Nearest to me".to_owned(),
        }
    }

    /// Nearest first, then the countries by name.
    pub fn from_codes(codes: &[String]) -> Vec<Self> {
        let mut countries: Vec<Self> = codes
            .iter()
            .map(|code| Self {
                code: Some(code.clone()),
                name: country_name(code).unwrap_or(code).to_owned(),
            })
            .collect();
        countries.sort_by(|a, b| a.name.cmp(&b.name));
        std::iter::once(Self::nearest()).chain(countries).collect()
    }

    /// The list to show before M-Lab's own arrives.
    pub fn known() -> Vec<Self> {
        let codes: Vec<String> = COUNTRIES_WITH_SERVERS
            .iter()
            .map(|code| (*code).to_owned())
            .collect();
        Self::from_codes(&codes)
    }

    pub fn location(&self) -> Location {
        match &self.code {
            Some(code) => Location::Country(code.clone()),
            None => Location::Nearest,
        }
    }
}

impl Default for LocationChoice {
    fn default() -> Self {
        Self::nearest()
    }
}

impl fmt::Display for LocationChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.code {
            Some(code) if *code != self.name => write!(f, "{} ({code})", self.name),
            _ => f.write_str(&self.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(choices: &[LocationChoice]) -> Vec<String> {
        choices.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn nearest_comes_first_and_maps_to_the_nearest_location() {
        let all = LocationChoice::known();
        assert_eq!(all[0], LocationChoice::nearest());
        assert_eq!(all[0].location(), Location::Nearest);
        assert_eq!(all.len(), COUNTRIES_WITH_SERVERS.len() + 1);
    }

    #[test]
    fn only_countries_with_servers_are_offered() {
        let shown = names(&LocationChoice::known());
        assert!(shown.contains(&"Japan (JP)".to_owned()));
        for without in ["Estonia (EE)", "Dominican Republic (DO)", "Latvia (LV)"] {
            assert!(!shown.contains(&without.to_owned()), "{without}");
        }
    }

    #[test]
    fn a_country_maps_to_its_code_and_the_list_is_sorted_by_name() {
        let all = LocationChoice::known();
        let japan = all.iter().find(|choice| choice.name == "Japan").unwrap();
        assert_eq!(japan.location(), Location::Country("JP".to_owned()));
        let countries = &all[1..];
        assert!(
            countries
                .windows(2)
                .all(|pair| pair[0].name <= pair[1].name)
        );
    }

    #[test]
    fn a_new_country_from_m_lab_is_offered_by_its_name_or_else_its_code() {
        let codes = vec!["EE".to_owned(), "ZZ".to_owned()];
        let shown = names(&LocationChoice::from_codes(&codes));
        assert_eq!(shown, ["Nearest to me", "Estonia (EE)", "ZZ"]);
    }

    #[test]
    fn the_known_codes_are_two_capital_letters_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for code in COUNTRIES_WITH_SERVERS {
            assert!(
                code.len() == 2 && code.chars().all(|c| c.is_ascii_uppercase()),
                "{code}"
            );
            assert!(seen.insert(code), "{code} listed twice");
        }
    }

    #[test]
    fn every_known_code_has_a_name() {
        for code in COUNTRIES_WITH_SERVERS {
            assert!(country_name(code).is_some(), "{code}");
        }
    }
}
