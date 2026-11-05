//! Text matching without allocating.

use std::cmp::Ordering;

/// Whether `haystack` contains `needle`, ignoring case. `needle` must already be lowercase.
/// Plain ASCII text is compared in place; anything else is lowercased first.
pub fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle.is_ascii() {
        return haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()));
    }
    haystack.to_lowercase().contains(needle)
}

/// Orders two texts as their lowercase forms would be ordered, without building them.
pub fn compare_ignore_case(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_text_whatever_its_case() {
        assert!(contains_ignore_case("Hewlett Packard", "hewlett"));
        assert!(contains_ignore_case("FE80::5", "fe80::5"));
        assert!(contains_ignore_case("anything", ""));
        assert!(!contains_ignore_case("Cisco", "juniper"));
        assert!(!contains_ignore_case("ab", "abc"));
    }

    #[test]
    fn finds_text_that_is_not_plain_ascii() {
        assert!(contains_ignore_case("Müller Büro", "büro"));
        assert!(contains_ignore_case("ÄÖÜ", "äöü"));
        assert!(!contains_ignore_case("Müller", "mueller"));
    }

    #[test]
    fn orders_like_the_lowercase_text() {
        assert_eq!(compare_ignore_case("alpha", "Beta"), Ordering::Less);
        assert_eq!(compare_ignore_case("ALPHA", "alpha"), Ordering::Equal);
        assert_eq!(compare_ignore_case("b", "A"), Ordering::Greater);
        assert_eq!(compare_ignore_case("ab", "abc"), Ordering::Less);
    }
}
