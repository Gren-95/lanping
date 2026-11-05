//! File helpers shared by the saved scans, measurements, settings, names and notes.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Names tried for one stem before giving up: `stem.json`, `stem-2.json`, and so on.
const MAX_NAME_ATTEMPTS: u32 = 1000;

/// Every `.json` file in `directory`. Empty when it cannot be read.
pub fn json_files(directory: &Path) -> Vec<PathBuf> {
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
pub fn write_new_json(directory: &Path, stem: &str, json: &str) -> io::Result<PathBuf> {
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

/// Replaces the file at `path` with `contents` so that it is never left half written: the text
/// goes to a file beside it first, which then takes its place. A crash or a full disk leaves
/// the old file as it was.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    write_through_temp(path, contents, false)
}

/// Like [`write_atomic`], and on Unix only the owner can read the file. For files that hold a
/// secret such as an SNMP community.
pub fn write_atomic_private(path: &Path, contents: &str) -> io::Result<()> {
    write_through_temp(path, contents, true)
}

fn write_through_temp(path: &Path, contents: &str, private: bool) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let temp = path.with_file_name(name);
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let result = options.open(&temp).and_then(|mut file| {
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    });
    match result.and_then(|()| fs::rename(&temp, path)) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// Moves a file that could not be read out of the way, to `name.unreadable`, so the next save
/// does not overwrite what was in it. Returns where it went.
pub fn set_aside_unreadable(path: &Path) -> io::Result<PathBuf> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".unreadable");
    let target = path.with_file_name(name);
    fs::rename(path, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_no_temp_file() {
        let directory = scratch("atomic");
        let path = directory.join("settings.json");
        write_atomic(&path, "one").unwrap();
        write_atomic(&path, "two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["settings.json"]);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_private_write_is_readable_by_the_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let directory = scratch("private");
        let path = directory.join("secret.json");
        write_atomic_private(&path, "x").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_unreadable_file_is_moved_aside_with_its_content_kept() {
        let directory = scratch("aside");
        let path = directory.join("labels.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, "{ not json").unwrap();
        let kept = set_aside_unreadable(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(kept).unwrap(), "{ not json");
        fs::remove_dir_all(&directory).unwrap();
    }

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
