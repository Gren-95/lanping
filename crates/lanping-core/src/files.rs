//! File helpers shared by the saved scans and the saved measurements.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Names tried for one stem before giving up: `stem.json`, `stem-2.json`, and so on.
const MAX_NAME_ATTEMPTS: u32 = 1000;

/// Every `.json` file in `directory`. Empty when it cannot be read.
pub(crate) fn json_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect()
}

/// Writes `json` to a new file named after `stem` in `directory`, and returns its path. A name
/// that is taken gets a number added. The file is created only if it does not exist, so two
/// writers never share one and nothing is overwritten.
pub(crate) fn write_new_json(directory: &Path, stem: &str, json: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let name = if attempt == 1 {
            format!("{stem}.json")
        } else {
            format!("{stem}-{attempt}.json")
        };
        let path = directory.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(json.as_bytes())?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many files with the same name",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("lanping-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn a_taken_name_gets_a_number_and_nothing_is_overwritten() {
        let directory = scratch("names");
        let first = write_new_json(&directory, "scan-5", "one").unwrap();
        let second = write_new_json(&directory, "scan-5", "two").unwrap();
        let third = write_new_json(&directory, "scan-5", "three").unwrap();
        assert_eq!(first.file_name().unwrap(), "scan-5.json");
        assert_eq!(second.file_name().unwrap(), "scan-5-2.json");
        assert_eq!(third.file_name().unwrap(), "scan-5-3.json");
        assert_eq!(fs::read_to_string(&first).unwrap(), "one");
        assert_eq!(fs::read_to_string(&second).unwrap(), "two");
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn lists_only_json_files_and_nothing_for_a_missing_directory() {
        let directory = scratch("list");
        write_new_json(&directory, "a", "{}").unwrap();
        fs::write(directory.join("notes.txt"), "x").unwrap();
        let files = json_files(&directory);
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("a.json"));
        fs::remove_dir_all(&directory).unwrap();
        assert!(json_files(&directory).is_empty());
    }
}
