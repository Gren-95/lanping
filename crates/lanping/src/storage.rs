use std::path::PathBuf;

use directories::{ProjectDirs, UserDirs};

pub fn history_directory() -> Option<PathBuf> {
    ProjectDirs::from("", "", "lanping").map(|dirs| dirs.data_dir().join("scans"))
}

pub fn export_directory() -> Option<PathBuf> {
    let dirs = UserDirs::new()?;
    dirs.download_dir()
        .map(PathBuf::from)
        .or_else(|| Some(dirs.home_dir().to_path_buf()))
}

pub fn settings_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "lanping").map(|dirs| dirs.config_dir().join("settings.json"))
}

pub fn labels_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "lanping").map(|dirs| dirs.config_dir().join("labels.json"))
}
