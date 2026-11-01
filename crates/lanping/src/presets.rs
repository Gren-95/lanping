//! Named filter combinations: a few built in, the rest saved by the user.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::host_filter::{HostFilter, LatencyBand, NameFilter, RowStatus};

pub const MAX_PRESET_NAME_CHARS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterPreset {
    pub name: String,
    pub filter: HostFilter,
}

/// One entry of the presets dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetChoice {
    pub preset: FilterPreset,
    pub built_in: bool,
}

impl fmt::Display for PresetChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.built_in {
            write!(f, "{}", self.preset.name)
        } else {
            write!(f, "{} (saved)", self.preset.name)
        }
    }
}

fn built_in(name: &str, filter: HostFilter) -> PresetChoice {
    PresetChoice {
        preset: FilterPreset {
            name: name.to_owned(),
            filter,
        },
        built_in: true,
    }
}

pub fn built_in_presets() -> Vec<PresetChoice> {
    vec![
        built_in(
            "New since last scan",
            HostFilter {
                status: Some(RowStatus::New),
                ..HostFilter::default()
            },
        ),
        built_in(
            "Gone since last scan",
            HostFilter {
                status: Some(RowStatus::Gone),
                ..HostFilter::default()
            },
        ),
        built_in(
            "Unlabeled devices",
            HostFilter {
                name: NameFilter::NoLabel,
                ..HostFilter::default()
            },
        ),
        built_in(
            "Web servers",
            HostFilter {
                port: Some(80),
                ..HostFilter::default()
            },
        ),
        built_in(
            "Slow or silent",
            HostFilter {
                latency: LatencyBand::Slow,
                ..HostFilter::default()
            },
        ),
    ]
}

/// Built-in presets first, then the user's saved ones.
pub fn all_presets(saved: &[FilterPreset]) -> Vec<PresetChoice> {
    let mut choices = built_in_presets();
    choices.extend(saved.iter().cloned().map(|preset| PresetChoice {
        preset,
        built_in: false,
    }));
    choices
}

/// Adds or replaces the preset called `name`. Blank names are rejected.
pub fn upsert_preset(
    saved: &mut Vec<FilterPreset>,
    name: &str,
    filter: HostFilter,
) -> Result<(), String> {
    let name: String = name.trim().chars().take(MAX_PRESET_NAME_CHARS).collect();
    if name.is_empty() {
        return Err("Give the preset a name".to_owned());
    }
    if built_in_presets()
        .iter()
        .any(|choice| choice.preset.name.eq_ignore_ascii_case(&name))
    {
        return Err("That name is used by a built-in preset".to_owned());
    }
    match saved
        .iter_mut()
        .find(|preset| preset.name.eq_ignore_ascii_case(&name))
    {
        Some(existing) => *existing = FilterPreset { name, filter },
        None => saved.push(FilterPreset { name, filter }),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_come_first_and_are_marked() {
        let saved = [FilterPreset {
            name: "Mine".into(),
            filter: HostFilter::default(),
        }];
        let all = all_presets(&saved);
        assert!(all[0].built_in);
        assert_eq!(all.last().unwrap().to_string(), "Mine (saved)");
    }

    #[test]
    fn upsert_adds_then_replaces_case_insensitively() {
        let mut saved = Vec::new();
        upsert_preset(
            &mut saved,
            "  Printers ",
            HostFilter {
                port: Some(9100),
                ..HostFilter::default()
            },
        )
        .unwrap();
        upsert_preset(
            &mut saved,
            "printers",
            HostFilter {
                port: Some(631),
                ..HostFilter::default()
            },
        )
        .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].filter.port, Some(631));
    }

    #[test]
    fn rejects_blank_and_built_in_names() {
        let mut saved = Vec::new();
        assert!(upsert_preset(&mut saved, "   ", HostFilter::default()).is_err());
        assert!(upsert_preset(&mut saved, "web servers", HostFilter::default()).is_err());
        assert!(saved.is_empty());
    }
}
