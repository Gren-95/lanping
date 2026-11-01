//! File formats a scan can be exported as, and the save dialog that asks where to put it.

use std::fmt;
use std::path::PathBuf;

use lanping_core::Host;
use lanping_core::export::{to_csv, to_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 2] = [ExportFormat::Csv, ExportFormat::Json];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }

    pub fn render(self, hosts: &[Host]) -> Result<String, String> {
        match self {
            Self::Csv => Ok(to_csv(hosts)),
            Self::Json => to_json(hosts).map_err(|error| error.to_string()),
        }
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Csv => "Export as CSV",
            Self::Json => "Export as JSON",
        })
    }
}

/// Asks the user where to save an export. `None` when they cancel.
pub async fn ask_save_path(
    format: ExportFormat,
    file_name: String,
    start_directory: Option<PathBuf>,
) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title("Export scan")
        .set_file_name(file_name)
        .add_filter(format.extension().to_uppercase(), &[format.extension()]);
    if let Some(directory) = start_directory {
        dialog = dialog.set_directory(directory);
    }
    let chosen = dialog.save_file().await?;
    let mut path = chosen.path().to_path_buf();
    if path.extension().is_none() {
        path.set_extension(format.extension());
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_each_format_with_its_extension() {
        let hosts = [Host::new(std::net::Ipv4Addr::new(10, 0, 0, 1))];
        assert!(
            ExportFormat::Csv
                .render(&hosts)
                .unwrap()
                .contains("10.0.0.1")
        );
        assert!(
            ExportFormat::Json
                .render(&hosts)
                .unwrap()
                .contains("10.0.0.1")
        );
        assert_eq!(ExportFormat::Csv.extension(), "csv");
        assert_eq!(ExportFormat::Json.extension(), "json");
    }
}
