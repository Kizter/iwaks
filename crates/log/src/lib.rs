//! Append-only file log for every Iwaks crate, plus the panic hook.
//!
//! Until [`init`] runs, [`line`] does nothing beyond writing to stderr — so
//! library crates, unit tests and the player tests can call it freely without
//! creating files. Once [`init`] has opened a sink, each line is appended to
//! `iwaks-YYYY-MM-DD.log` in the app log directory.
//!
//! Deliberately dependency-free: the log is the thing you read when something
//! is already broken, so it must not be able to break itself. Every failure
//! path here is ignored on purpose, and locks are poison-tolerant.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Severity of one logged line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Developer detail that is off in a normal run.
    Debug,
    Info,
    /// Something unexpected that the app recovered from.
    Warn,
    /// A failure the user is likely to notice.
    Error,
}

impl Level {
    /// Fixed-width tag so log columns line up.
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
        }
    }
}

struct Sink {
    file: Mutex<File>,
    path: PathBuf,
}

static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();

/// Start appending to `iwaks-YYYY-MM-DD.log` inside `dir`.
///
/// Returns the file it opened, or `None` if the directory or file could not be
/// created — the app then keeps logging to stderr only, which is exactly how it
/// behaved before. Calling this more than once keeps the first sink.
pub fn init(dir: &Path) -> Option<PathBuf> {
    let slot = SINK.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = guard.as_ref() {
        // Read the path from the guard we already hold: `path()` would try to
        // lock the same non-reentrant mutex again.
        return Some(existing.path.clone());
    }
    let secs = now_secs();
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let path = dir.join(format!("iwaks-{year:04}-{month:02}-{day:02}.log"));
    std::fs::create_dir_all(dir).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    *guard = Some(Sink {
        file: Mutex::new(file),
        path: path.clone(),
    });
    Some(path)
}

/// The log file currently being written, if any.
pub fn path() -> Option<PathBuf> {
    SINK.get().and_then(|slot| {
        slot.lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.path.clone())
    })
}

/// Append one line: `<timestamp> <LEVEL> <message>`.
///
/// Newlines in `message` are folded so one record stays one line. Never panics
/// and never blocks on anything but its own mutex.
pub fn line(level: Level, message: &str) {
    // The console copy is kept because it is the only output when no sink is
    // open, and it costs nothing when one is.
    eprintln!("[{}] {message}", level.as_str());
    let Some(slot) = SINK.get() else { return };
    let record = format!(
        "{} [{}] {}\n",
        stamp(now_secs()),
        level.as_str(),
        message.replace('\n', " ⏎ ")
    );
    // Held straight through: the sink lives inside the lock, and locking it
    // again here would mean a re-entrant borrow on every logged line.
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sink) = guard.as_mut() else { return };
    let mut file = sink.file.lock().unwrap_or_else(|e| e.into_inner());
    let _ = file.write_all(record.as_bytes());
    let _ = file.flush();
}

/// Record every panic (message + source location) in the log before handing it
/// to the hook that was installed before us.
///
/// A panic hook that can itself panic turns a recoverable bug into an abort,
/// so nothing here allocates beyond the format buffers and no IO error is
/// propagated.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic payload was not a string".to_string());
        let where_ = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        line(Level::Error, &format!("panic: {message} ({where_})"));
        previous(info);
    }));
}

/// Seconds since the Unix epoch (0 if the clock is before it).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `YYYY-MM-DD HH:MM:SS` in UTC for a Unix timestamp.
fn stamp(secs: u64) -> String {
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let tod = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// Days since 1970-01-01 → civil `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`: shifting the era to start in March makes
/// the leap day the last day of the year, which removes every special case.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_itself_is_day_zero() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn known_dates_come_out_right() {
        // Chosen as real calendar dates around the interesting boundaries.
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        assert_eq!(civil_from_days(58), (1970, 2, 28)); // 1970 was not a leap year
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // 2024 is a leap year
        assert_eq!(civil_from_days(19_783), (2024, 3, 1)); // …and its leap day
        assert_eq!(civil_from_days(20_737), (2026, 10, 11));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn every_day_of_a_leap_and_a_common_year_is_well_formed() {
        // Walks four years day by day: month must stay in 1..=12, and each
        // month must advance exactly when its length says it should.
        let mut previous: Option<(i64, u32, u32)> = None;
        for days in 19_000..19_000 + 4 * 366 {
            let (year, month, day) = civil_from_days(days);
            assert!((1..=12).contains(&month), "month out of range at {days}");
            assert!((1..=31).contains(&day), "day out of range at {days}");
            if let Some((py, pm, pd)) = previous {
                assert!(
                    (year, month, day) > (py, pm, pd),
                    "not strictly increasing at {days}"
                );
            }
            previous = Some((year, month, day));
        }
    }

    #[test]
    fn stamps_are_zero_padded_and_utc() {
        assert_eq!(stamp(0), "1970-01-01 00:00:00");
        assert_eq!(stamp(1_700_000_000), "2023-11-14 22:13:20");
        assert_eq!(stamp(86_399), "1970-01-01 23:59:59");
        assert_eq!(stamp(86_400), "1970-01-02 00:00:00");
    }

    #[test]
    fn levels_have_a_fixed_width_tag() {
        let tags: Vec<&str> = [Level::Debug, Level::Info, Level::Warn, Level::Error]
            .iter()
            .map(|l| l.as_str())
            .collect();
        assert!(tags.iter().all(|t| t.len() == 5), "{tags:?}");
    }
}
