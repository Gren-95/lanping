use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since 1970 now. A clock set before 1970 reads as 0.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Formats Unix seconds as `YYYY-MM-DD HH:MM UTC` without a date library.
pub fn format_unix_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds_of_day / 3600,
        seconds_of_day % 3600 / 60
    )
}

/// Formats Unix seconds as the `HH:MM:SS` of that day in UTC.
pub fn format_unix_time_utc(seconds: u64) -> String {
    let seconds_of_day = seconds % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds_of_day / 3600,
        seconds_of_day % 3600 / 60,
        seconds_of_day % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_epoch() {
        assert_eq!(format_unix_utc(0), "1970-01-01 00:00 UTC");
    }

    #[test]
    fn formats_the_time_of_day() {
        assert_eq!(format_unix_time_utc(0), "00:00:00");
        assert_eq!(format_unix_time_utc(1_709_210_096), "12:34:56");
        assert_eq!(format_unix_time_utc(86_399), "23:59:59");
    }

    #[test]
    fn formats_leap_day() {
        assert_eq!(format_unix_utc(1_709_210_096), "2024-02-29 12:34 UTC");
    }

    #[test]
    fn formats_year_end() {
        assert_eq!(format_unix_utc(1_798_761_540), "2026-12-31 23:59 UTC");
    }
}
