use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::{ProjectDirs, UserDirs};

pub fn history_directory() -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_NAME).map(|dirs| dirs.data_dir().join("scans"))
}

/// Saved latency runs and throughput tests, next to the saved scans.
pub fn measurements_directory() -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_NAME).map(|dirs| dirs.data_dir().join("measurements"))
}

pub fn export_directory() -> Option<PathBuf> {
    let dirs = UserDirs::new()?;
    dirs.download_dir()
        .map(PathBuf::from)
        .or_else(|| Some(dirs.home_dir().to_path_buf()))
}

pub fn settings_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_NAME).map(|dirs| dirs.config_dir().join("settings.json"))
}

pub fn device_notes_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_NAME).map(|dirs| dirs.config_dir().join("device_notes.json"))
}

pub fn labels_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_NAME).map(|dirs| dirs.config_dir().join("labels.json"))
}

/// Loads a file of the user's with `load`. A missing file is the first run and gives the
/// defaults. A file that cannot be read gives the defaults too, but is moved aside first, so
/// the next save does not overwrite what was in it, and the second value says what happened.
pub fn load_or_default<T: Default>(
    path: Option<PathBuf>,
    what: &str,
    load: impl FnOnce(&Path) -> io::Result<T>,
) -> (T, Option<String>) {
    let Some(path) = path else {
        return (T::default(), None);
    };
    match load(&path) {
        Ok(value) => (value, None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (T::default(), None),
        Err(error) => {
            let kept = match lanping_core::files::set_aside_unreadable(&path) {
                Ok(moved) => format!(
                    "The old file was kept as {}.",
                    moved.file_name().unwrap_or_default().to_string_lossy()
                ),
                Err(move_error) => format!("It could not be moved aside either: {move_error}."),
            };
            (
                T::default(),
                Some(format!(
                    "Could not read {what} ({error}), so it starts empty. {kept}"
                )),
            )
        }
    }
}

const APP_NAME: &str = "lanping";
const LEGACY_APP_NAME: &str = "lansweep";

/// Moves settings, labels and saved scans from the folders of the app's earlier
/// name. A folder is only moved when the new one does not exist yet.
/// Returns how many folders were moved.
pub fn migrate_legacy_data() -> io::Result<usize> {
    let (Some(old), Some(new)) = (
        ProjectDirs::from("", "", LEGACY_APP_NAME),
        ProjectDirs::from("", "", APP_NAME),
    ) else {
        return Ok(0);
    };
    let pairs = [
        (old.config_dir(), new.config_dir()),
        (old.data_dir(), new.data_dir()),
    ];
    let mut moved = 0;
    for (from, to) in pairs {
        if move_directory_if_free(from, to)? {
            moved += 1;
        }
    }
    Ok(moved)
}

/// Renames `from` to `to` when `from` exists and `to` does not.
fn move_directory_if_free(from: &Path, to: &Path) -> io::Result<bool> {
    if !from.is_dir() || to.exists() {
        return Ok(false);
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(from, to)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("lanping-storage-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_missing_file_gives_the_defaults_without_a_message() {
        let root = temp_root("load-missing");
        let (value, message) = load_or_default(Some(root.join("x.json")), "x", |path| {
            fs::read_to_string(path)
        });
        assert_eq!(value, "");
        assert_eq!(message, None);
        let (value, message) = load_or_default(None, "x", |_| Ok(String::from("unused")));
        assert_eq!((value.as_str(), message), ("", None));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_kept_and_the_message_says_so() {
        let root = temp_root("load-bad");
        let path = root.join("settings.json");
        fs::write(&path, "{ not json").unwrap();
        let (value, message) = load_or_default(Some(path.clone()), "settings", |_| {
            Err::<String, _>(io::Error::other("bad json"))
        });
        assert_eq!(value, "");
        let message = message.unwrap();
        assert!(
            message.contains("settings") && message.contains("settings.json.unreadable"),
            "{message}"
        );
        assert!(!path.exists());
        assert!(root.join("settings.json.unreadable").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_good_file_is_loaded() {
        let root = temp_root("load-good");
        let path = root.join("a.json");
        fs::write(&path, "hello").unwrap();
        let (value, message) = load_or_default(Some(path), "a", |p| fs::read_to_string(p));
        assert_eq!((value.as_str(), message), ("hello", None));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn moves_an_old_folder_when_the_new_one_is_missing() {
        let root = temp_root("move");
        let (old, new) = (root.join("lansweep"), root.join("nested").join("lanping"));
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("settings.json"), "{}").unwrap();
        assert!(move_directory_if_free(&old, &new).unwrap());
        assert!(new.join("settings.json").is_file());
        assert!(!old.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn leaves_both_folders_alone_when_the_new_one_exists_or_the_old_is_missing() {
        let root = temp_root("keep");
        let (old, new) = (root.join("lansweep"), root.join("lanping"));
        assert!(!move_directory_if_free(&old, &new).unwrap());
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        assert!(!move_directory_if_free(&old, &new).unwrap());
        assert!(old.exists() && new.exists());
        let _ = fs::remove_dir_all(&root);
    }
}
