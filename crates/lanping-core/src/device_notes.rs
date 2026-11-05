//! The user's own facts about a device, keyed by [`crate::Host::device_key`].

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use crate::text::contains_ignore_case;
use serde::{Deserialize, Serialize};

const MAX_FIELD_CHARS: usize = 200;
const MAX_NOTES_CHARS: usize = 2000;
const MAX_TAGS: usize = 20;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceNote {
    pub owner: String,
    pub location: String,
    pub tags: Vec<String>,
    pub notes: String,
}

fn limited(text: &str, max_chars: usize) -> String {
    text.trim().chars().take(max_chars).collect()
}

impl DeviceNote {
    /// Builds a note from what was typed. Tags are separated by commas.
    pub fn from_input(owner: &str, location: &str, tags: &str, notes: &str) -> Self {
        let mut unique_tags: Vec<String> = Vec::new();
        for tag in tags.split(',').map(|tag| limited(tag, MAX_FIELD_CHARS)) {
            if !tag.is_empty() && !unique_tags.contains(&tag) && unique_tags.len() < MAX_TAGS {
                unique_tags.push(tag);
            }
        }
        Self {
            owner: limited(owner, MAX_FIELD_CHARS),
            location: limited(location, MAX_FIELD_CHARS),
            tags: unique_tags,
            notes: limited(notes, MAX_NOTES_CHARS),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.owner.is_empty()
            && self.location.is_empty()
            && self.tags.is_empty()
            && self.notes.is_empty()
    }

    /// Tags joined back into the form they are typed in.
    pub fn tags_text(&self) -> String {
        self.tags.join(", ")
    }

    /// Whether any field contains `needle`, which must already be lowercase.
    pub fn contains(&self, needle: &str) -> bool {
        [&self.owner, &self.location, &self.notes]
            .into_iter()
            .chain(&self.tags)
            .any(|field| contains_ignore_case(field, needle))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceNotes(BTreeMap<String, DeviceNote>);

impl DeviceNotes {
    pub fn get(&self, key: &str) -> Option<&DeviceNote> {
        self.0.get(key)
    }

    /// Stores `note` for `key`. An empty note removes the entry.
    pub fn set(&mut self, key: &str, note: DeviceNote) {
        if note.is_empty() {
            self.0.remove(key);
        } else {
            self.0.insert(key.to_owned(), note);
        }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let json = fs::read_to_string(path)?;
        serde_json::from_str(&json).map_err(io::Error::other)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_splits_what_was_typed() {
        let note = DeviceNote::from_input(" Sam ", "Attic", "iot, , iot,  lab ", " x ");
        assert_eq!(note.owner, "Sam");
        assert_eq!(note.tags, ["iot", "lab"]);
        assert_eq!(note.tags_text(), "iot, lab");
        assert_eq!(note.notes, "x");
    }

    #[test]
    fn limits_field_lengths() {
        let long = "x".repeat(5000);
        let note = DeviceNote::from_input(&long, "", "", &long);
        assert_eq!(note.owner.len(), MAX_FIELD_CHARS);
        assert_eq!(note.notes.len(), MAX_NOTES_CHARS);
    }

    #[test]
    fn an_empty_note_removes_the_entry() {
        let mut notes = DeviceNotes::default();
        notes.set("k", DeviceNote::from_input("Sam", "", "", ""));
        assert!(notes.get("k").is_some());
        notes.set("k", DeviceNote::from_input(" ", " ", " , ", " "));
        assert!(notes.get("k").is_none());
    }

    #[test]
    fn searches_every_field_including_tags() {
        let note = DeviceNote::from_input("Sam", "Attic", "Lab", "old unit");
        for needle in ["sam", "attic", "lab", "old"] {
            assert!(note.contains(needle), "{needle}");
        }
        assert!(!note.contains("kitchen"));
    }

    #[test]
    fn saves_and_loads() {
        let directory = std::env::temp_dir().join(format!("lanping-notes-{}", std::process::id()));
        let path = directory.join("device_notes.json");
        let mut notes = DeviceNotes::default();
        notes.set("k", DeviceNote::from_input("Sam", "", "a", ""));
        notes.save(&path).unwrap();
        assert_eq!(DeviceNotes::load(&path).unwrap(), notes);
        let _ = fs::remove_dir_all(&directory);
    }
}
