//! The log actually lands on disk, one line per record.
//!
//! Its own integration-test binary: `init` is deliberately once-per-process, so
//! the round trip cannot be asserted from a unit test beside it.

use std::fs;
use std::path::PathBuf;

use iwaks_log::{line, path as log_path, Level};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("iwaks-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lines_are_appended_to_a_dated_file() {
    let dir = TempDir::new();

    // Before init there is no file at all and logging is a stderr-only no-op.
    assert_eq!(log_path(), None);
    line(Level::Info, "before init");
    assert_eq!(
        fs::read_dir(&dir.0).expect("read dir").count(),
        0,
        "nothing was created yet"
    );

    let file = iwaks_log::init(&dir.0).expect("init opens a log file");
    assert_eq!(log_path(), Some(file.clone()));
    assert!(file.starts_with(&dir.0), "the file lives where we asked");

    line(Level::Info, "hello from the test");
    line(Level::Warn, "second line");
    line(Level::Error, "two\nlines folded");

    // A second init must not switch files underneath the first one.
    assert_eq!(iwaks_log::init(&dir.0), Some(file.clone()));

    let body = fs::read_to_string(&file).expect("log file readable");
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 3, "one record per call: {body:?}");

    // "YYYY-MM-DD HH:MM:SS [LEVEL] message"
    for record in &lines {
        let (date, rest) = record.split_once(' ').expect("date first");
        assert_eq!(date.len(), 10, "date field in {record:?}");
        assert!(
            rest.starts_with(|c: char| c.is_ascii_digit()),
            "time follows in {record:?}"
        );
    }
    assert!(
        lines[0].ends_with("[INFO ] hello from the test"),
        "{}",
        lines[0]
    );
    assert!(lines[1].ends_with("[WARN ] second line"), "{}", lines[1]);
    assert!(
        lines[2].ends_with("[ERROR] two ⏎ lines folded"),
        "newlines are folded: {}",
        lines[2]
    );

    // The file name is the date those records were written on.
    let name = file.file_name().unwrap().to_string_lossy().to_string();
    let date = &name["iwaks-".len()..name.len() - ".log".len()];
    assert_eq!(date.len(), 10, "YYYY-MM-DD in {name}");
    assert!(
        lines[0].starts_with(date),
        "file name matches the line: {}",
        lines[0]
    );
}
