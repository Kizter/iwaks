//! The iTunes Search API — Apple's public, keyless album-art endpoint.
//!
//! Only provider-specific bits live here (URL shape, response shape, match
//! rules); the orchestration is in the crate root.

use serde::Deserialize;

use crate::CoverError;

/// Candidate count per query. More than one because the match guard has to be
/// able to skip a top hit that belongs to a compilation or a tribute act.
const SEARCH_LIMIT: usize = 10;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    #[serde(default)]
    results: Vec<ResultItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultItem {
    artist_name: String,
    collection_name: Option<String>,
    artwork_url_100: Option<String>,
}

/// Search endpoint for `artist` + `album`, most specific parameters first.
pub(crate) fn search_url(artist: &str, album: &str) -> String {
    // The term goes out as tagged (only whitespace-folded): case is irrelevant
    // to the endpoint, and sending the tag verbatim keeps the URL honest.
    let term = collapse_whitespace(&format!("{artist} {album}"));
    format!(
        "https://itunes.apple.com/search?entity=album&limit={SEARCH_LIMIT}&term={}",
        encode_query(&term)
    )
}

/// First result whose artist and album both check out, at thumbnail size.
///
/// `Ok(None)` = the endpoint answered and has no such release. The guard is
/// deliberately strict: a wrong cover on every track of an album is worse than
/// no cover, so anything unverified is dropped.
pub(crate) fn pick_artwork(
    body: &str,
    artist: &str,
    album: &str,
) -> Result<Option<String>, CoverError> {
    let parsed: SearchResponse = serde_json::from_str(body).map_err(|_| CoverError::Malformed)?;
    Ok(parsed.results.into_iter().find_map(|item| {
        let url = item.artwork_url_100?;
        let album_ok = item
            .collection_name
            .as_deref()
            .is_some_and(|name| album_matches(album, name));
        (artist_matches(artist, &item.artist_name) && album_ok).then(|| bigger_artwork(&url))
    }))
}

/// iTunes serves every thumbnail from one CDN path with the size in the
/// filename, so 100px → 600px is a string swap. 600px is the largest square
/// the endpoint serves and stays under a tenth of a megabyte.
pub(crate) fn bigger_artwork(url: &str) -> String {
    url.replace("100x100bb", "600x600bb")
}

/// Do the tagged artist and the provider's artist name denote the same act?
///
/// Equality after case/whitespace folding and after dropping a trailing
/// parenthetical (`"Crayon Case (Band)"` ≡ `"Crayon Case"`). Substring
/// matching is deliberately *not* used: it would accept `"Crayon Case
/// Tribute"`, `"Crayon Case Orchestra"`, … and put the wrong sleeve on the
/// track.
pub(crate) fn artist_matches(expected: &str, actual: &str) -> bool {
    let expected = normalize(expected);
    let actual = normalize(actual);
    !expected.is_empty() && expected == actual
}

/// Do the tagged album and the provider's collection name denote one release?
///
/// The endpoint appends the release type (`"Surabaya - Single"`,
/// `"… - EP"`), so comparison uses the part before the first `" - "`. An
/// untagged album accepts any release by a matching artist.
pub(crate) fn album_matches(expected: &str, actual: &str) -> bool {
    let expected = normalize(expected);
    if expected.is_empty() {
        return true;
    }
    expected == normalize(base_album(actual))
}

/// The release name without its trailing `- Single` / `- EP` qualifier.
pub(crate) fn base_album(album: &str) -> &str {
    album.split(" - ").next().unwrap_or(album).trim()
}

/// Lowercase, collapse internal whitespace, drop a trailing bracketed aside.
/// Shared by [`artist_matches`] and the cache key so both fold the same way.
pub(crate) fn normalize(text: &str) -> String {
    let lowered = collapse_whitespace(&text.to_lowercase());
    strip_trailing_aside(&lowered)
}

/// Percent-encode a query component. Space becomes `+`, as every form encoder
/// does; everything outside the RFC 3986 unreserved set is escaped byte-wise so
/// non-ASCII tags survive.
pub(crate) fn encode_query(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Drop a trailing `(…)` / `[…]` aside, which is how both tags and store
/// listings decorate a name (`"Crayon Case (Band)"`, `"… [2019 Remaster]"`).
/// An unclosed aside is left alone rather than truncated.
fn strip_trailing_aside(text: &str) -> String {
    for (open, close) in [(" (", ')'), (" [", ']')] {
        if let Some((head, tail)) = text.split_once(open) {
            if tail.ends_with(close) {
                return head.trim_end().to_string();
            }
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- search_url ----------

    #[test]
    fn search_url_targets_the_public_endpoint_with_entity_and_limit() {
        let url = search_url("Crayon Case", "Surabaya");
        assert!(url.starts_with("https://itunes.apple.com/search?"), "{url}");
        assert!(url.contains("entity=album"), "{url}");
        assert!(url.contains("limit=10"), "{url}");
    }

    #[test]
    fn search_url_joins_artist_and_album_into_the_term() {
        assert!(
            search_url("Crayon Case", "Surabaya").contains("term=Crayon+Case+Surabaya"),
            "{}",
            search_url("Crayon Case", "Surabaya")
        );
    }

    #[test]
    fn search_url_keeps_a_missing_album_half_out_of_the_term() {
        let url = search_url("Crayon Case", "   ");
        assert!(url.ends_with("term=Crayon+Case"), "{url}");
    }

    #[test]
    fn search_url_percent_encodes_reserved_characters() {
        let url = search_url("AC/DC", "Bóng Đè");
        assert!(url.contains("term=AC%2FDC+B%C3%B3ng+%C4%90%C3%A8"), "{url}");
    }

    // ---------- pick_artwork ----------

    fn body(results: &str) -> String {
        format!(r#"{{"resultCount":1,"results":[{results}]}}"#)
    }

    fn item(artist: &str, collection: &str) -> String {
        let slug = artist.replace(' ', "-");
        format!(
            r#"{{"artistName":"{artist}","collectionName":"{collection}","artworkUrl100":"https://cdn.example/{slug}/100x100bb.jpg"}}"#
        )
    }

    /// `pick_artwork` for tests that only care about the match outcome — a
    /// response that fails to parse is a different failure and is tested as such.
    fn found(body: &str, artist: &str, album: &str) -> Option<String> {
        pick_artwork(body, artist, album).expect("parsable response")
    }

    #[test]
    fn pick_artwork_upsizes_the_thumbnail() {
        let raw = body(&item("Crayon Case", "Surabaya - Single"));
        let url = found(&raw, "Crayon Case", "Surabaya").expect("match");
        assert!(url.contains("/600x600bb.jpg"), "{url}");
        assert!(url.contains("Crayon-Case"), "{url}");
    }

    #[test]
    fn pick_artwork_skips_a_wrong_artist() {
        let raw = body(&item("Tribute Band", "Surabaya - Single"));
        assert!(found(&raw, "Crayon Case", "Surabaya").is_none());
    }

    #[test]
    fn pick_artwork_skips_a_wrong_album() {
        let raw = body(&item("Crayon Case", "Greatest Hits"));
        assert!(found(&raw, "Crayon Case", "Surabaya").is_none());
    }

    #[test]
    fn pick_artwork_skips_a_result_without_artwork() {
        let raw = r#"{"resultCount":1,"results":[
            {"artistName":"Crayon Case","collectionName":"Surabaya - Single"}]}"#;
        assert!(found(raw, "Crayon Case", "Surabaya").is_none());
    }

    #[test]
    fn pick_artwork_skips_a_result_without_a_collection_name() {
        let raw = r#"{"resultCount":1,"results":[
            {"artistName":"Crayon Case","artworkUrl100":"https://cdn.example/a/100x100bb.jpg"}]}"#;
        assert!(found(raw, "Crayon Case", "Surabaya").is_none());
    }

    #[test]
    fn pick_artwork_takes_any_release_when_the_album_is_untagged() {
        let raw = body(&item("Crayon Case", "Greatest Hits"));
        assert!(found(&raw, "Crayon Case", "").is_some());
    }

    #[test]
    fn pick_artwork_is_none_for_an_empty_result_set() {
        assert!(found(r#"{"resultCount":0,"results":[]}"#, "A", "B").is_none());
    }

    #[test]
    fn pick_artwork_is_none_when_results_key_is_absent() {
        assert!(found(r#"{"resultCount":3}"#, "A", "B").is_none());
    }

    #[test]
    fn pick_artwork_reports_unparsable_json_as_malformed() {
        assert!(matches!(
            pick_artwork("", "A", "B"),
            Err(CoverError::Malformed)
        ));
        assert!(matches!(
            pick_artwork("{", "A", "B"),
            Err(CoverError::Malformed)
        ));
        // A bare array deserializes as an empty struct, so it lands as
        // "no match" rather than "unusable" — harmless either way.
        assert!(found("[]", "A", "B").is_none());
    }

    // ---------- bigger_artwork ----------

    #[test]
    fn bigger_artwork_swaps_the_thumbnail_size() {
        assert_eq!(
            bigger_artwork("https://cdn.example/abc/100x100bb.jpg"),
            "https://cdn.example/abc/600x600bb.jpg"
        );
    }

    #[test]
    fn bigger_artwork_leaves_an_unknown_url_shape_alone() {
        let url = "https://cdn.example/cover.png";
        assert_eq!(bigger_artwork(url), url);
    }

    // ---------- matching ----------

    #[test]
    fn artist_matches_ignores_case_and_padding() {
        assert!(artist_matches("  crayon CASE ", "Crayon Case"));
    }

    #[test]
    fn artist_matches_ignores_a_parenthetical_suffix() {
        assert!(artist_matches("Crayon Case (Band)", "Crayon Case"));
        assert!(artist_matches("Crayon Case", "Crayon Case [2019 Remaster]"));
    }

    #[test]
    fn artist_matches_rejects_a_longer_different_name() {
        // The regression that substring matching would have shipped.
        assert!(!artist_matches("Crayon Case", "Crayon Case Tribute"));
        assert!(!artist_matches("Crayon Case", "Crayon Case Orchestra"));
    }

    #[test]
    fn artist_matches_rejects_an_unrelated_name() {
        assert!(!artist_matches("Crayon Case", "Banda Melodi"));
    }

    #[test]
    fn artist_matches_rejects_empty_input() {
        assert!(!artist_matches("", "Crayon Case"));
        assert!(!artist_matches("Crayon Case", "   "));
    }

    #[test]
    fn album_matches_drops_the_release_type_qualifier() {
        assert!(album_matches("Surabaya", "Surabaya - Single"));
        assert!(album_matches("Surabaya", "Surabaya - EP"));
        assert!(album_matches("Bunga Kuning", "Bunga Kuning - Single"));
    }

    #[test]
    fn album_matches_rejects_another_release() {
        assert!(!album_matches("Surabaya", "Greatest Hits - Single"));
    }

    #[test]
    fn base_album_splits_on_the_first_qualifier() {
        assert_eq!(base_album("Bunga Kuning - Single"), "Bunga Kuning");
        assert_eq!(base_album("Live - Tokyo - Single"), "Live");
        assert_eq!(base_album("Plain Album"), "Plain Album");
    }

    // ---------- normalize / encode ----------

    #[test]
    fn normalize_collapses_internal_whitespace_and_case() {
        assert_eq!(normalize("Crayon   Case \n Band"), "crayon case band");
    }

    #[test]
    fn normalize_drops_a_trailing_bracketed_aside() {
        assert_eq!(normalize("Crayon Case (Band)"), "crayon case");
        assert_eq!(normalize("Crayon Case [2019 Remaster]"), "crayon case");
        assert_eq!(normalize("Crayon Case (Band"), "crayon case (band");
    }

    #[test]
    fn encode_query_escapes_the_unreserved_set_boundary() {
        assert_eq!(encode_query("aZ0-_.~"), "aZ0-_.~");
    }
}
