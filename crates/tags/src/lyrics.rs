//! Lyrics extraction: embedded tags (USLT / Vorbis `LYRICS` / iTunes `©lyr`)
//! plus `.lrc` sidecar files with synchronized timestamps.
//!
//! Priority for *timed* lines: the `.lrc` sidecar (path with the extension
//! swapped to `.lrc`) wins; otherwise embedded text is parsed as LRC when it
//! contains timestamps. *Plain* text comes from the embedded tag when present,
//! falling back to the raw sidecar content.

use std::path::Path;

use lofty::prelude::*;
use lofty::tag::ItemKey;

use crate::TagError;

/// One synchronized lyric line (LRC semantics; timestamp in seconds).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedLine {
    pub time: f64,
    pub text: String,
}

/// Lyrics for a track: timed lines plus optional unsynchronized full text.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// Synchronized lines, sorted ascending; empty when only plain text.
    pub timed: Vec<TimedLine>,
    /// Unsynchronized full text (embedded tag, or the raw sidecar when the
    /// embedded tag is absent). `None` when there are only timed lines.
    pub plain: Option<String>,
}

/// Read lyrics for `path`: embedded tag first, then a `<file>.lrc` sidecar.
pub fn read_lyrics(path: &Path) -> Result<Option<Lyrics>, TagError> {
    let embedded = read_embedded_lyrics(path)?;
    let sidecar = read_sidecar_lrc(path)?;

    let mut timed = Vec::new();
    let mut plain = embedded.clone();
    if let Some(lrc) = &sidecar {
        timed = parse_lrc(lrc).timed;
        if plain.is_none() {
            plain = Some(lrc.clone());
        }
    } else if let Some(text) = &embedded {
        let parsed = parse_lrc(text);
        if !parsed.timed.is_empty() {
            timed = parsed.timed;
        }
    }

    if timed.is_empty() && plain.is_none() {
        Ok(None)
    } else {
        Ok(Some(Lyrics { timed, plain }))
    }
}

/// Embedded unsynchronized lyrics — USLT (ID3), `LYRICS` (Vorbis), `©lyr`
/// (iTunes) all map to `ItemKey::Lyrics` in lofty.
fn read_embedded_lyrics(path: &Path) -> Result<Option<String>, TagError> {
    let tagged = lofty::read_from_path(path)?;
    for tag in tagged.tags() {
        if let Some(text) = tag.get_string(&ItemKey::Lyrics) {
            let text = text.trim();
            if !text.is_empty() {
                return Ok(Some(text.to_string()));
            }
        }
    }
    Ok(None)
}

/// `<file>.lrc` sitting next to the audio file, when it exists.
fn read_sidecar_lrc(path: &Path) -> Result<Option<String>, TagError> {
    let lrc_path = path.with_extension("lrc");
    if !lrc_path.is_file() {
        return Ok(None);
    }
    Ok(Some(std::fs::read_to_string(&lrc_path)?))
}

/// Parse LRC `[mm:ss.xx]` (and `[hh:mm:ss.xx]`) timestamps from a line,
/// returning the seconds for each tag in order; `None` when the line has no
/// timestamp tags.
fn line_timestamps(tag: &str) -> Option<f64> {
    let parts: Vec<&str> = tag.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut seconds = 0.0;
    let mut idx = 0;
    if parts.len() == 3 {
        seconds += parts[0].trim().parse::<f64>().ok()? * 3600.0;
        idx = 1;
    }
    let minutes: f64 = parts[idx].trim().parse().ok()?;
    let secs: f64 = parts[idx + 1].trim().parse().ok()?;
    if minutes < 0.0 || secs < 0.0 {
        return None;
    }
    Some(seconds + minutes * 60.0 + secs)
}

/// Parse LRC text into its timed lines, applying `[offset:±ms]` to every
/// timestamp. Metadata tags (`[ti:]`, `[ar:]`, ...) are skipped; plain lines
/// without timestamps are dropped from `timed`.
fn parse_lrc(text: &str) -> ParsedLrc {
    let mut timed = Vec::new();
    let mut offset_ms: i64 = 0;
    for raw in text.lines() {
        let mut rest = raw.trim();
        if rest.is_empty() {
            continue;
        }
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            let tag = &stripped[..end];
            rest = stripped[end + 1..].trim_start();
            match tag {
                t if t.starts_with("offset:") => {
                    if let Ok(ms) = t[7..].trim().parse::<i64>() {
                        offset_ms = ms;
                    }
                }
                t => {
                    if let Some(secs) = line_timestamps(t) {
                        times.push(secs);
                    }
                }
            }
        }
        if !times.is_empty() {
            for t in times {
                timed.push(TimedLine {
                    time: (t + offset_ms as f64 / 1000.0).max(0.0),
                    text: rest.to_string(),
                });
            }
        }
    }
    timed.sort_by(|a, b| a.time.total_cmp(&b.time));
    let offset = offset_ms;
    ParsedLrc { timed, offset }
}

#[derive(Debug, Default, PartialEq)]
struct ParsedLrc {
    timed: Vec<TimedLine>,
    /// `[offset:±ms]` — kept for tests; applied in `timed` already.
    offset: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- pure LRC parsing ----

    #[test]
    fn parses_timestamps_and_applies_offset() {
        let parsed = parse_lrc("[offset:500]\n[00:12.00]Hello\n[01:02.5]World\n");
        assert_eq!(parsed.offset, 500);
        assert_eq!(
            parsed.timed,
            vec![
                TimedLine {
                    time: 12.5,
                    text: "Hello".into()
                },
                TimedLine {
                    time: 62.5 + 0.5,
                    text: "World".into()
                },
            ]
        );
    }

    #[test]
    fn multiple_timestamps_per_line_and_hours() {
        let parsed = parse_lrc("[00:01.00][00:05.00]same\n[01:02:03.4]long\n");
        assert_eq!(
            parsed.timed,
            vec![
                TimedLine {
                    time: 1.0,
                    text: "same".into()
                },
                TimedLine {
                    time: 5.0,
                    text: "same".into()
                },
                TimedLine {
                    time: 3723.4,
                    text: "long".into()
                },
            ]
        );
    }

    #[test]
    fn metadata_and_bad_tags_are_ignored() {
        let parsed = parse_lrc("[ti:Title]\n[ar:Artist]\n[oops]bad\nplain line\n");
        assert!(parsed.timed.is_empty(), "no parseable timestamps");
        assert_eq!(parsed.offset, 0);
    }

    #[test]
    fn offset_can_be_negative_and_clamps_at_zero() {
        let parsed = parse_lrc("[offset:-300]\n[00:00.20]early\n[00:10.00]later\n");
        assert_eq!(
            parsed.timed,
            vec![
                TimedLine {
                    time: 0.0,
                    text: "early".into()
                }, // clamped
                TimedLine {
                    time: 9.7,
                    text: "later".into()
                },
            ]
        );
    }

    // ---- fixtures (valid playable FLAC, as in cover.rs) ----

    fn flac_streaminfo() -> Vec<u8> {
        let mut b = Vec::with_capacity(34);
        b.extend_from_slice(&4096u16.to_be_bytes());
        b.extend_from_slice(&4096u16.to_be_bytes());
        b.extend_from_slice(&[0, 0, 0]);
        b.extend_from_slice(&[0, 0, 0]);
        let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36) | 44_100;
        b.extend_from_slice(&packed.to_be_bytes());
        b.extend_from_slice(&[0u8; 16]);
        b
    }

    /// VORBIS_COMMENT block (type 4) body.
    fn vorbis_comment_block(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes()); // vendor length
        body.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (k, v) in entries {
            let kv = format!("{k}={v}");
            body.extend_from_slice(&(kv.len() as u32).to_le_bytes());
            body.extend_from_slice(kv.as_bytes());
        }
        body
    }

    fn build_flac_with_lyrics(lyrics: Option<&str>) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(b"fLaC");
        let si = flac_streaminfo();
        f.push(0x00); // metadata block header prefix (not last)
        f.extend_from_slice(&(si.len() as u32).to_be_bytes()[1..]);
        f.extend_from_slice(&si);
        let body = match lyrics {
            Some(text) => vorbis_comment_block(&[("LYRICS", text)]),
            None => Vec::new(),
        };
        f.push(0x84); // last block, type 4
        f.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        f.extend_from_slice(&body);
        f
    }

    fn fixture_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("iwaks-lyrics-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    // ---- file-level reads ----

    #[test]
    fn reads_embedded_vorbis_lyrics() {
        let dir = fixture_dir("embedded");
        let path = dir.join("song.flac");
        std::fs::write(
            &path,
            build_flac_with_lyrics(Some("First line\nSecond line")),
        )
        .unwrap();

        let lyrics = read_lyrics(&path).expect("read ok").expect("has lyrics");
        assert_eq!(lyrics.timed, Vec::<TimedLine>::new());
        assert_eq!(lyrics.plain.as_deref(), Some("First line\nSecond line"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sidecar_lrc_wins_for_timed_lines() {
        let dir = fixture_dir("sidecar");
        let path = dir.join("song.flac");
        std::fs::write(&path, build_flac_with_lyrics(Some("Embedded text"))).unwrap();
        std::fs::write(dir.join("song.lrc"), "[00:05.00]Sidecar line").unwrap();

        let lyrics = read_lyrics(&path).expect("read ok").expect("has lyrics");
        assert_eq!(
            lyrics.timed,
            vec![TimedLine {
                time: 5.0,
                text: "Sidecar line".into()
            }]
        );
        assert_eq!(
            lyrics.plain.as_deref(),
            Some("Embedded text"),
            "embedded kept as plain"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sidecar_plain_fallback_when_no_embedded_text() {
        let dir = fixture_dir("sidecar-only");
        let path = dir.join("song.flac");
        std::fs::write(&path, build_flac_with_lyrics(None)).unwrap();
        std::fs::write(dir.join("song.lrc"), "[00:01.00]One\n[00:02.00]Two\n").unwrap();

        let lyrics = read_lyrics(&path).expect("read ok").expect("has lyrics");
        assert_eq!(lyrics.timed.len(), 2);
        assert_eq!(
            lyrics.plain,
            Some("[00:01.00]One\n[00:02.00]Two\n".to_string()),
            "raw sidecar doubles as plain text"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn embedded_lrc_text_is_parsed_for_timing() {
        let dir = fixture_dir("embedded-lrc");
        let path = dir.join("song.flac");
        std::fs::write(&path, build_flac_with_lyrics(Some("[00:03.00]Timed\n"))).unwrap();

        let lyrics = read_lyrics(&path).expect("read ok").expect("has lyrics");
        assert_eq!(
            lyrics.timed,
            vec![TimedLine {
                time: 3.0,
                text: "Timed".into()
            }]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_lyrics_anywhere_returns_none() {
        let dir = fixture_dir("none");
        let path = dir.join("song.flac");
        std::fs::write(&path, build_flac_with_lyrics(None)).unwrap();

        assert_eq!(read_lyrics(&path).expect("read ok"), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
