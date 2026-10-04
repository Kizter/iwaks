//! Album-art lookup for the Discord Rich Presence.
//!
//! This is the only crate in the workspace that opens a network connection —
//! `iwaks-presence` stays dependency-light and pipe-only. It is opt-in: nothing
//! here runs unless the user turns the feature on.
//!
//! Discord renders `large_image` / `small_image` from a URL its *own* servers
//! fetch, so the art never has to be hosted locally — only be reachable from
//! the internet. That rules out `file://` and loopback, and rules in any
//! public CDN. Verified against the live client: `i.ytimg.com` and
//! `is1-ssl.mzstatic.com` artwork both display.
//!
//! Provider is Apple's public **iTunes Search API**: no key, no account, stable
//! CDN URLs that do not expire, and real square cover art (unlike YouTube's
//! 16:9 video thumbnails).

use std::time::Duration;

mod itunes;

/// Whole-request budget. The lookup runs off the playback path, so a slow or
/// dead network must not pile up threads behind the sink.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Cache key for one album: normalized `artist` + `album`, so tag casing and
/// whitespace variants share a cache row. `None` on either side yields an empty
/// key, which callers treat as "not cacheable".
pub fn album_key(artist: Option<&str>, album: Option<&str>) -> String {
    let artist = artist.map(itunes::normalize).unwrap_or_default();
    let album = album.map(itunes::normalize).unwrap_or_default();
    if artist.is_empty() && album.is_empty() {
        return String::new();
    }
    // U+001F (unit separator) cannot occur in tag text, so two different
    // artist/album pairs can never collide on one key.
    format!("{artist}\u{1f}{album}")
}

/// Why a lookup could not be answered.
///
/// The split matters: only a *successful* search with no match may be cached as
/// "this album has no cover". A network failure must stay retryable, otherwise
/// one offline minute would poison the cache for good.
#[derive(Debug, thiserror::Error)]
pub enum CoverError {
    /// The endpoint could not be reached (offline, DNS, TLS, timeout, 5xx).
    #[error("cover lookup request failed: {0}")]
    Request(String),
    /// The response was not the JSON shape the endpoint documents.
    #[error("cover lookup response was unusable")]
    Malformed,
}

/// Look up cover art for `artist` / `album`.
///
/// `Ok(None)` means the provider has no matching release — a fact worth
/// remembering. `Err` means the question could not be answered at all, so the
/// caller must ask again later.
pub fn lookup(artist: &str, album: &str) -> Result<Option<String>, CoverError> {
    lookup_with(fetch_itunes, artist, album)
}

/// Decision logic, with the HTTP fetch injected so it is testable offline.
fn lookup_with(
    fetch: impl FnOnce(&str) -> Result<String, CoverError>,
    artist: &str,
    album: &str,
) -> Result<Option<String>, CoverError> {
    if artist.trim().is_empty() && album.trim().is_empty() {
        return Ok(None);
    }
    let url = itunes::search_url(artist, album);
    let body = fetch(&url)?;
    itunes::pick_artwork(&body, artist, album)
}

/// The single `ureq` call in the workspace. Timeouts are bounded and every
/// failure mode collapses to [`CoverError`].
fn fetch_itunes(url: &str) -> Result<String, CoverError> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .new_agent();
    match agent.get(url).call() {
        Ok(mut response) => response
            .body_mut()
            .read_to_string()
            .map_err(|e| CoverError::Request(e.to_string())),
        Err(ureq::Error::StatusCode(code)) => Err(CoverError::Request(format!("HTTP {code}"))),
        Err(e) => Err(CoverError::Request(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HIT: &str = r#"{"resultCount":1,"results":[
        {"artistName":"Crayon Case","collectionName":"Surabaya - Single",
         "artworkUrl100":"https://is1-ssl.mzstatic.com/image/thumb/Music211/v4/bd/36/e6/x/100x100bb.jpg"}]}"#;

    /// Ask with a canned provider answer instead of the network.
    fn ask(body: &str, artist: &str, album: &str) -> Result<Option<String>, CoverError> {
        lookup_with(|_| Ok(body.to_string()), artist, album)
    }

    /// Same, for the tests that only care whether a cover came back.
    fn answer(body: &str, artist: &str, album: &str) -> Option<String> {
        ask(body, artist, album).expect("provider answered")
    }

    #[test]
    fn lookup_returns_600px_artwork_for_a_matching_album() {
        let url = ask(HIT, "Crayon Case", "Surabaya")
            .expect("answered")
            .expect("artwork found");
        assert!(
            url.contains("/600x600bb.jpg"),
            "small thumbnail upscaled: {url}"
        );
    }

    #[test]
    fn lookup_asks_the_search_api_for_that_artist_and_album() {
        let mut asked = String::new();
        lookup_with(
            |url| {
                asked = url.to_string();
                Ok(HIT.to_string())
            },
            "Crayon Case",
            "Surabaya",
        )
        .expect("answered")
        .expect("artwork found");
        assert!(
            asked.starts_with("https://itunes.apple.com/search?"),
            "{asked}"
        );
        assert!(asked.contains("term=Crayon+Case+Surabaya"), "{asked}");
    }

    #[test]
    fn lookup_reports_a_transport_failure_as_retryable() {
        // Not `Ok(None)`: the cache must not remember this as "album has no cover".
        let err = lookup_with(
            |_| Err(CoverError::Request("offline".into())),
            "Crayon Case",
            "Surabaya",
        )
        .expect_err("network failure is an error");
        assert!(matches!(err, CoverError::Request(_)), "{err}");
    }

    #[test]
    fn lookup_reports_an_unusable_response_as_retryable() {
        let err = lookup_with(|_| Ok("{".into()), "Crayon Case", "Surabaya")
            .expect_err("broken json is an error");
        assert!(matches!(err, CoverError::Malformed), "{err}");
    }

    #[test]
    fn lookup_skips_a_wrong_artist_result() {
        let body = r#"{"resultCount":1,"results":[
            {"artistName":"Someone Else","collectionName":"Surabaya - Single",
             "artworkUrl100":"https://cdn.example/other/100x100bb.jpg"}]}"#;
        assert!(answer(body, "Crayon Case", "Surabaya").is_none());
    }

    #[test]
    fn lookup_accepts_a_parenthetical_artist_variant() {
        let body = r#"{"resultCount":1,"results":[
            {"artistName":"Crayon Case","collectionName":"Surabaya - Single",
             "artworkUrl100":"https://cdn.example/ok/100x100bb.jpg"}]}"#;
        assert!(answer(body, "Crayon Case (Band)", "Surabaya").is_some());
    }

    #[test]
    fn lookup_rejects_a_tribute_album_result() {
        // Substring matching would have accepted this and shown the wrong sleeve.
        let body = r#"{"resultCount":1,"results":[
            {"artistName":"Crayon Case Tribute","collectionName":"Surabaya - Single",
             "artworkUrl100":"https://cdn.example/tribute/100x100bb.jpg"}]}"#;
        assert!(answer(body, "Crayon Case", "Surabaya").is_none());
    }

    #[test]
    fn lookup_rejects_a_different_album_by_the_same_artist() {
        let body = r#"{"resultCount":1,"results":[
            {"artistName":"Crayon Case","collectionName":"Greatest Hits",
             "artworkUrl100":"https://cdn.example/wrong/100x100bb.jpg"}]}"#;
        assert!(answer(body, "Crayon Case", "Surabaya").is_none());
    }

    #[test]
    fn lookup_picks_the_first_match_when_earlier_results_are_off() {
        let body = r#"{"resultCount":3,"results":[
            {"artistName":"Crayon Case Tribute","collectionName":"Surabaya - Single",
             "artworkUrl100":"https://cdn.example/no/100x100bb.jpg"},
            {"artistName":"Banda Melodi","collectionName":"Bunga Kuning - Single",
             "artworkUrl100":"https://cdn.example/no2/100x100bb.jpg"},
            {"artistName":"Crayon Case","collectionName":"Surabaya - Single",
             "artworkUrl100":"https://cdn.example/yes/100x100bb.jpg"}]}"#;
        let url = answer(body, "Crayon Case", "Surabaya").expect("third hits");
        assert!(url.contains("/yes/"), "{url}");
    }

    #[test]
    fn lookup_tolerates_an_empty_result_set() {
        assert!(answer(r#"{"results":[]}"#, "A", "B").is_none());
    }

    #[test]
    fn lookup_without_any_tag_text_does_not_reach_the_network() {
        let mut called = false;
        let out = lookup_with(
            |_| {
                called = true;
                Ok(HIT.to_string())
            },
            "  ",
            "",
        )
        .expect("answered");
        assert!(out.is_none());
        assert!(!called, "no request for a blank tag");
    }

    #[test]
    fn album_key_normalizes_case_and_whitespace() {
        assert_eq!(
            album_key(Some("Crayon  Case"), Some(" Surabaya ")),
            album_key(Some("crayon case"), Some("surabaya"))
        );
    }

    #[test]
    fn album_key_is_empty_without_tags() {
        assert_eq!(album_key(None, None), "");
        assert_eq!(album_key(Some("  "), None), "");
    }

    #[test]
    fn album_key_cannot_collide_across_pairs() {
        // Without a separator, ("ab", "c") and ("a", "bc") would share a key.
        assert_ne!(
            album_key(Some("ab"), Some("c")),
            album_key(Some("a"), Some("bc"))
        );
    }

    #[test]
    fn album_key_survives_partial_tags() {
        assert_ne!(album_key(Some("Crayon Case"), None), "");
    }
}
