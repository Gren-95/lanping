//! How results are printed: aligned tables for people, JSON for scripts.

use serde_json::Value;

/// Prints `value` as indented JSON.
pub fn print_json(value: &Value) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(error) => eprintln!("lanping-cli: could not write JSON: {error}"),
    }
}

/// Rows of text under headings, printed with each column as wide as its widest cell.
pub struct Table {
    headings: Vec<&'static str>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headings: &[&'static str]) -> Self {
        Self {
            headings: headings.to_vec(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    /// The table as lines, without trailing spaces.
    pub fn lines(&self) -> Vec<String> {
        let widths: Vec<usize> = (0..self.headings.len())
            .map(|column| {
                self.rows
                    .iter()
                    .filter_map(|row| row.get(column))
                    .map(|cell| cell.chars().count())
                    .chain(std::iter::once(self.headings[column].chars().count()))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let line = |cells: &[String]| {
            cells
                .iter()
                .zip(&widths)
                .map(|(cell, width)| format!("{cell:<width$}"))
                .collect::<Vec<_>>()
                .join("  ")
                .trim_end()
                .to_owned()
        };
        let headings: Vec<String> = self.headings.iter().map(|h| (*h).to_owned()).collect();
        std::iter::once(line(&headings))
            .chain(self.rows.iter().map(|row| line(row)))
            .collect()
    }

    pub fn print(&self) {
        for line in self.lines() {
            println!("{line}");
        }
    }
}

/// Seconds as milliseconds with one decimal, for tables.
pub fn milliseconds(seconds: f64) -> String {
    format!("{:.1} ms", seconds * 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_line_up_and_lines_have_no_trailing_spaces() {
        let mut table = Table::new(&["ADDRESS", "NAME"]);
        table.row(vec!["10.0.0.1".to_owned(), "router".to_owned()]);
        table.row(vec!["10.0.0.200".to_owned(), String::new()]);
        assert_eq!(
            table.lines(),
            ["ADDRESS     NAME", "10.0.0.1    router", "10.0.0.200"]
        );
    }

    #[test]
    fn wide_characters_count_by_character() {
        let mut table = Table::new(&["A", "B"]);
        table.row(vec!["é".to_owned(), "x".to_owned()]);
        assert_eq!(table.lines(), ["A  B", "é  x"]);
    }

    #[test]
    fn milliseconds_have_one_decimal() {
        assert_eq!(milliseconds(0.0123), "12.3 ms");
    }
}
