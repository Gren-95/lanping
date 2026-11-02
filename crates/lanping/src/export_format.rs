//! File formats a scan can be exported as, and the save dialog that asks where to put it.

use std::fmt;
use std::path::PathBuf;

use lanping_core::export::{to_csv, to_json};
use lanping_core::report::{ReportInput, to_html};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
    Html,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [ExportFormat::Csv, ExportFormat::Json, ExportFormat::Html];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
            Self::Html => "html",
        }
    }

    pub fn render(self, input: &ReportInput<'_>) -> Result<String, String> {
        match self {
            Self::Csv => Ok(to_csv(input.hosts, input.notes)),
            Self::Json => to_json(input.hosts, input.notes).map_err(|error| error.to_string()),
            Self::Html => Ok(to_html(input)),
        }
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Csv => "Export as CSV",
            Self::Json => "Export as JSON",
            Self::Html => "Export as report (HTML)",
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
    use lanping_core::Host;
    use lanping_core::device_notes::DeviceNotes;
    use lanping_core::labels::DeviceLabels;

    #[test]
    fn renders_each_format_with_its_extension() {
        let hosts = [Host::new(std::net::Ipv4Addr::new(10, 0, 0, 1))];
        let (labels, notes) = (DeviceLabels::default(), DeviceNotes::default());
        let input = ReportInput {
            target: "10.0.0.0/24",
            generated_at: "now",
            hosts: &hosts,
            labels: &labels,
            notes: &notes,
        };
        for format in ExportFormat::ALL {
            assert!(format.render(&input).unwrap().contains("10.0.0.1"));
        }
        assert_eq!(ExportFormat::Csv.extension(), "csv");
        assert_eq!(ExportFormat::Json.extension(), "json");
        assert_eq!(ExportFormat::Html.extension(), "html");
    }
}
