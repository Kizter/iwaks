//! Time formatting helpers (pure).

/// Format milliseconds as `m:ss` (e.g. 61_000 -> "1:01"). Negative input is
/// clamped to zero.
pub fn format_duration(ms: i64) -> String {
    let ms = ms.max(0);
    let total_secs = ms / 1000;
    format!("{}:{:02}", total_secs / 60, total_secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_seconds_and_minutes() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(61_000), "1:01");
        assert_eq!(format_duration(366_000), "6:06");
        assert_eq!(format_duration(3_659_000), "60:59");
    }

    #[test]
    fn clamps_negative_to_zero() {
        assert_eq!(format_duration(-500), "0:00");
    }
}
