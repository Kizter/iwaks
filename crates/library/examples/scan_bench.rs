//! NFR A2 evidence runner (docs/design.md): "scan 10,000 files < 30 s".
//!
//! Generates nothing — point it at a corpus (see
//! `scripts/gen-scan-bench-corpus.ps1`) and it reports the wall time of a full
//! `scan` against a fresh database, plus the per-file counters:
//!
//! ```text
//! cargo run --release --example scan_bench -- <root> [db-path]
//! ```
//!
//! Running it a second time against the same database exercises the
//! incremental path (everything `skipped`), which is what a warm library sees.

use std::path::PathBuf;
use std::time::Instant;

use iwaks_library::db::Library;
use iwaks_library::scan::{scan, ScanOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root: PathBuf = args
        .next()
        .expect("usage: scan_bench <root> [db-path]")
        .into();
    let db_path = args.next().unwrap_or_else(|| "scan-bench.sqlite".into());
    // `fresh` (default) drops the database for the worst case: every file is
    // new, so metadata is read for all of them. Pass `reuse` against the same
    // path to measure the incremental pass a warm library actually runs.
    let fresh = args.next().unwrap_or_else(|| "fresh".into()) != "reuse";
    if fresh {
        let _ = std::fs::remove_file(&db_path);
    }
    let mut lib = Library::open(&db_path)?;

    let started = Instant::now();
    let report = scan(
        &mut lib,
        &ScanOptions {
            root,
            clean_missing: false,
        },
        &mut |_| {}, // progress callbacks only cost time; the app's UI shows them
    )?;
    let elapsed = started.elapsed().as_secs_f64();

    println!(
        "elapsed {elapsed:.2}s  files={} added={} skipped={} errors={}",
        report.total_files, report.added, report.skipped, report.errors
    );
    if report.total_files == 10_000 {
        println!(
            "NFR A2: 30.00s budget -> {:.0}% used",
            100.0 * elapsed / 30.0
        );
    }
    Ok(())
}
