//! `init` creates the log directory, including missing parents — the app hands
//! it a path that does not exist yet on a fresh machine.
//!
//! Its own test binary because `init` is once-per-process.

use std::fs;

#[test]
fn a_missing_directory_is_created() {
    let dir = std::env::temp_dir().join(format!("iwaks-log-mkdir-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);

    let nested = dir.join("deeply/nested/logs");
    let file = iwaks_log::init(&nested).expect("init creates the path");
    assert!(nested.is_dir(), "the whole chain was created");
    assert!(file.starts_with(&nested));
    assert!(file.is_file());

    iwaks_log::line(iwaks_log::Level::Info, "after init");
    let body = fs::read_to_string(&file).expect("log file readable");
    assert!(body.contains("after init"), "{body}");

    let _ = fs::remove_dir_all(&dir);
}
