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
