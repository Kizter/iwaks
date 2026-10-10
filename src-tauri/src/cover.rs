//! Album-art resolution for the Rich Presence.
//!
//! The playback sink fires ~10×/second, so nothing on that path may touch the
//! disk or the network. This module keeps the last answer in memory: the sink
//! asks what to do with the track it just saw, and a background thread does any
//! lookup and publishes the answer for the next tick to pick up. A cover
//! therefore appears within ~100 ms of being resolved, with no second send path
//! and no stale playback position to reconstruct.
//!
//! Everything here is opt-in: with the feature off the sink does no work and
//! no request leaves the machine.

use std::path::Path;
use std::sync::{Arc, Mutex};

use iwaks_cover::CoverError;
use iwaks_library::cover::CoverLookup;
use iwaks_library::db::Library;

/// Settings key holding the opt-in flag (`"true"` / `"false"`).
pub const SETTING_ONLINE_COVER: &str = "discord.online_cover";

/// What the sink should do about cover art for the track it just saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverAction {
    /// The answer for this track is already known. `None` means the album has
    /// no cover, which is a settled answer too.
    Known(Option<String>),
    /// Nothing known and nothing in flight — start a lookup.
    Start,
    /// A lookup for this track is running; show the app logo until it lands.
    Waiting,
}

/// Last known cover-art answer, keyed by album.
#[derive(Debug, Default)]
pub struct CoverMemo {
    /// Album key the stored answer belongs to.
    settled: Option<String>,
    /// The answer for `settled` — `None` when that album has no cover.
    url: Option<String>,
    /// Album key of the lookup currently in flight.
    running: Option<String>,
}

impl CoverMemo {
    pub fn new() -> Self {
        Self::default()
    }

    /// What to do about `key`. A blank key (a track with no album tags) is
    /// never looked up.
    pub fn on_track(&mut self, key: &str) -> CoverAction {
        if key.is_empty() {
            return CoverAction::Known(None);
        }
        if self.settled.as_deref() == Some(key) {
            return CoverAction::Known(self.url.clone());
        }
        if self.running.as_deref() == Some(key) {
            return CoverAction::Waiting;
        }
        self.running = Some(key.to_string());
        CoverAction::Start
    }

    /// Publish the answer for a finished lookup. Answers for a track the user
    /// has already left are dropped, so a slow lookup cannot put album A's
    /// cover on album B.
    pub fn publish(&mut self, key: &str, url: Option<String>) {
        if self.running.as_deref() != Some(key) {
            return;
        }
        self.running = None;
        self.settled = Some(key.to_string());
        self.url = url;
    }

    /// Forget everything — used when the feature is switched off, so a stale
    /// cover cannot survive the change.
    pub fn clear(&mut self) {
        *self = Self::new();
    }
}

/// Shared memo handle passed to both the sink and the lookup threads.
pub type SharedMemo = Arc<Mutex<CoverMemo>>;

/// Resolve the cover for one track off the sink thread and publish it.
///
/// `key` is the cache key for the pair (`iwaks_cover::album_key`); the tags are
/// passed separately because that is what the provider is asked about.
pub fn resolve_in_background(
    db_path: std::path::PathBuf,
    memo: SharedMemo,
    key: String,
    artist: String,
    album: String,
) {
    std::thread::spawn(move || {
        let url = resolve(&db_path, &key, &artist, &album, iwaks_cover::lookup_chain);
        memo.lock().expect("cover memo").publish(&key, url);
    });
}

/// The cached answer for one album, consulting the provider only on a miss.
///
/// A transport failure is deliberately **not** stored: caching it would mean one
/// offline minute hid the cover permanently. Only a successful search with no
/// match is remembered as `Missing`.
pub fn resolve(
    db_path: &Path,
    key: &str,
    artist: &str,
    album: &str,
    fetch: impl FnOnce(&str, &str) -> Result<Option<String>, CoverError>,
) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    let lib = Library::open(&db_path.to_string_lossy()).ok()?;
    if let Some(hit) = lib.cached_cover(key).ok().flatten() {
        return match hit {
            CoverLookup::Found(url) => Some(url),
            CoverLookup::Missing => None,
        };
    }
    match fetch(artist, album) {
        Ok(found) => {
            let stored = match &found {
                Some(url) => CoverLookup::Found(url.clone()),
                None => CoverLookup::Missing,
            };
            let _ = lib.store_cover(key, &stored);
            found
        }
        Err(e) => {
            // Deliberately not cached: a transport failure must stay retryable,
            // so the next tick of this album asks again. Logged once here rather
            // than in the crate — a library returns the error, the app reports it.
            if cfg!(debug_assertions) {
                eprintln!("[debug] cover: lookup not cached ({e})");
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iwaks_cover::CoverError;
    use std::path::PathBuf;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("iwaks-cover-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }
        fn db(&self) -> PathBuf {
            self.0.join("lib.db")
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const ART: &str = "https://cdn.example/album/600x600bb.jpg";

    // ---------- CoverMemo ----------

    #[test]
    fn first_sight_of_a_track_starts_a_lookup() {
        let mut memo = CoverMemo::new();
        assert_eq!(memo.on_track("a"), CoverAction::Start);
    }

    #[test]
    fn a_repeat_tick_does_not_start_a_second_lookup() {
        let mut memo = CoverMemo::new();
        assert_eq!(memo.on_track("a"), CoverAction::Start);
        assert_eq!(memo.on_track("a"), CoverAction::Waiting);
    }

    #[test]
    fn a_published_answer_is_reused_without_another_lookup() {
        let mut memo = CoverMemo::new();
        memo.on_track("a");
        memo.publish("a", Some(ART.to_string()));
        assert_eq!(
            memo.on_track("a"),
            CoverAction::Known(Some(ART.to_string()))
        );
        assert_eq!(
            memo.on_track("a"),
            CoverAction::Known(Some(ART.to_string()))
        );
    }

    #[test]
    fn an_album_without_a_cover_is_a_settled_answer() {
        let mut memo = CoverMemo::new();
        memo.on_track("a");
        memo.publish("a", None);
        assert_eq!(memo.on_track("a"), CoverAction::Known(None));
    }

    #[test]
    fn changing_track_starts_a_new_lookup_and_hides_the_old_cover() {
        let mut memo = CoverMemo::new();
        memo.on_track("a");
        memo.publish("a", Some(ART.to_string()));
        assert_eq!(memo.on_track("b"), CoverAction::Start);
        // "Waiting" is what keeps album A's sleeve off album B while B resolves.
        assert_eq!(memo.on_track("b"), CoverAction::Waiting);
    }

    /// The slow-lookup race: a lookup for the previous track finishes after the
    /// user skipped to the next one. Its answer must be discarded.
    #[test]
    fn a_late_answer_for_an_abandoned_track_is_dropped() {
        let mut memo = CoverMemo::new();
        memo.on_track("a");
        memo.on_track("b");
        memo.publish("a", Some(ART.to_string()));
        assert_eq!(
            memo.on_track("b"),
            CoverAction::Waiting,
            "album A's cover must not attach to album B"
        );
    }

    #[test]
    fn an_untagged_track_is_never_looked_up() {
        let mut memo = CoverMemo::new();
        assert_eq!(memo.on_track(""), CoverAction::Known(None));
        assert_eq!(memo.on_track(""), CoverAction::Known(None));
    }

    #[test]
    fn clearing_forgets_the_previous_cover() {
        let mut memo = CoverMemo::new();
        memo.on_track("a");
        memo.publish("a", Some(ART.to_string()));
        memo.clear();
        assert_eq!(memo.on_track("a"), CoverAction::Start);
    }

    // ---------- resolve ----------

    #[test]
    fn a_cached_hit_is_returned_without_asking_the_provider() {
        let tmp = TempDir::new("hit");
        let db = tmp.db();
        let lib = Library::open(&db.to_string_lossy()).unwrap();
        lib.store_cover("key", &CoverLookup::Found(ART.to_string()))
            .unwrap();

        let asked = std::cell::Cell::new(false);
        let url = resolve(&db, "key", "Crayon Case", "Surabaya", |_, _| {
            asked.set(true);
            Ok(Some("https://other/".into()))
        });
        assert_eq!(url.as_deref(), Some(ART));
        assert!(!asked.get(), "a cached album must not hit the network");
    }

    #[test]
    fn a_provider_hit_is_returned_and_remembered() {
        let tmp = TempDir::new("store");
        let db = tmp.db();
        let url = resolve(&db, "key", "Crayon Case", "Surabaya", |_, _| {
            Ok(Some(ART.to_string()))
        });
        assert_eq!(url.as_deref(), Some(ART));

        // Second call must be served from the cache.
        let asked = std::cell::Cell::new(false);
        let again = resolve(&db, "key", "Crayon Case", "Surabaya", |_, _| {
            asked.set(true);
            Ok(None)
        });
        assert_eq!(again.as_deref(), Some(ART));
        assert!(!asked.get());
    }

    #[test]
    fn a_provider_miss_is_remembered_as_no_cover() {
        let tmp = TempDir::new("missing");
        let db = tmp.db();
        assert_eq!(
            resolve(&db, "key", "Crayon Case", "Surabaya", |_, _| Ok(None)),
            None
        );

        let asked = std::cell::Cell::new(false);
        assert_eq!(
            resolve(&db, "key", "Crayon Case", "Surabaya", |_, _| {
                asked.set(true);
                Ok(Some(ART.to_string()))
            }),
            None
        );
        assert!(!asked.get(), "an album the provider lacks stays un-asked");
    }

    /// The regression that motivates the `Result` split in `iwaks-cover`.
    #[test]
    fn a_transport_failure_is_not_remembered() {
        let tmp = TempDir::new("offline");
        let db = tmp.db();
        assert_eq!(
            resolve(&db, "key", "Crayon Case", "Surabaya", |_, _| Err(
                CoverError::Request("offline".into())
            )),
            None
        );

        let asked = std::cell::Cell::new(false);
        let retry = resolve(&db, "key", "Crayon Case", "Surabaya", |_, _| {
            asked.set(true);
            Ok(Some(ART.to_string()))
        });
        assert_eq!(
            retry.as_deref(),
            Some(ART),
            "the retry must reach the provider"
        );
        assert!(asked.get());
    }

    #[test]
    fn an_untagged_track_never_reaches_the_provider() {
        let tmp = TempDir::new("blank");
        let asked = std::cell::Cell::new(false);
        let url = resolve(&tmp.db(), "", "", "", |_, _| {
            asked.set(true);
            Ok(Some(ART.to_string()))
        });
        assert_eq!(url, None);
        assert!(!asked.get());
    }

    #[test]
    fn an_unusable_database_degrades_to_no_cover() {
        // A path that cannot be a database file: no panic, no cover.
        let url = resolve(
            Path::new("C:\\nul\\missing\\lib.db"),
            "key",
            "A",
            "B",
            |_, _| Ok(Some(ART.to_string())),
        );
        assert_eq!(url, None);
    }
}
