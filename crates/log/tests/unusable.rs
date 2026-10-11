//! The degraded path: when the log directory cannot be used, the app keeps
//! running on stderr output exactly as it did before logging existed.
//!
//! Its own test binary on purpose — `init` is once-per-process, so the failure
//! path needs a process that has not already opened a sink.

use std::fs;

#[test]
fn an_unusable_directory_is_reported_instead_of_panicking() {
    // A regular file where a directory is expected: `create_dir_all` fails.
    let blocker = std::env::temp_dir().join(format!("iwaks-log-blocked-{}", std::process::id()));
    fs::write(&blocker, b"not a directory").expect("write blocker file");

    let result = iwaks_log::init(&blocker.join("logs"));
    assert_eq!(result, None, "unusable directory is reported, not panicked");
    assert_eq!(iwaks_log::path(), None);

    // Logging still works — it just goes to the console.
    iwaks_log::line(iwaks_log::Level::Debug, "logging still works");
    assert_eq!(iwaks_log::path(), None);

    let _ = fs::remove_file(&blocker);
}
