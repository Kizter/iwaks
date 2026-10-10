//! YouTube Music — best-effort community fallback provider.
//!
//! YouTube Music has no keyless public API. This talks to the same internal
//! `youtubei/v1/search` endpoint the web client uses, with the `WEB_REMIX`
//! client and no account: a static client version, a visitor id scraped from
//! the homepage, and the "artist album" query. The response shape is
//! undocumented and may change at any time, so parsing is defensive — anything
//! unexpected degrades to `Ok(None)` (or `Malformed`), never a panic.
//!
//! This only runs after the default provider has found nothing, and only when
//! the user opted into online album art — the network posture is unchanged.

use serde_json::Value;

use crate::CoverError;

/// YouTube Music web origin (also the `Origin` header the endpoint expects).
const DOMAIN: &str = "https://music.youtube.com";
/// Homepage, read only to scrape the per-session visitor id out of `ytcfg`.
const HOMEPAGE: &str = "https://music.youtube.com";
/// Inner-tube search endpoint. Unauthenticated web clients send `alt=json`.
const SEARCH_URL: &str = "https://music.youtube.com/youtubei/v1/search?alt=json";
/// A normal browser UA; the endpoint rejects obviously scripted clients.
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:128.0) Gecko/20100101 Firefox/128.0";

/// Search for `artist` + `album` and return an upscaled thumbnail URL.
pub(crate) fn lookup(artist: &str, album: &str) -> Result<Option<String>, CoverError> {
    if artist.trim().is_empty() && album.trim().is_empty() {
        return Ok(None);
    }
    // Best-effort: the endpoint accepts requests without a visitor id, so a
    // failed bootstrap is not fatal — the search just proceeds without it.
    let visitor = visitor_id();
    let mut headers = vec![
        ("user-agent", USER_AGENT),
        ("accept", "*/*"),
        ("origin", DOMAIN),
    ];
    if let Some(id) = visitor.as_deref() {
        headers.push(("x-goog-visitor-id", id));
    }
    let body = request_body(artist, album);
    let response = crate::net::post_json(SEARCH_URL, &body, &headers)?;
    pick_cover(&response, artist, album)
}

/// The visitor id embedded in the homepage's `ytcfg.set({...})` blob.
fn visitor_id() -> Option<String> {
    crate::net::get_with(HOMEPAGE, &[("user-agent", USER_AGENT)])
        .ok()
        .and_then(|html| extract_visitor_id(&html))
}

/// Pull `"VISITOR_DATA":"…"` out of a page without a regex dependency.
pub(crate) fn extract_visitor_id(html: &str) -> Option<String> {
    const MARKER: &str = "\"VISITOR_DATA\":\"";
    let start = html.find(MARKER)? + MARKER.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// The JSON request body: the search query plus the client context.
pub(crate) fn request_body(artist: &str, album: &str) -> String {
    let query = format!("{artist} {album}");
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": client_version(),
            },
            "user": {},
        },
        "query": query,
    })
    .to_string()
}

/// `1.YYYYMMDD.01.00` — the web client builds this from the current UTC date.
/// Computed rather than hardcoded so it never goes stale.
fn client_version() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86_400) as i64)
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(days);
    format!("1.{year:04}{month:02}{day:02}.01.00")
}

/// Days since the Unix epoch → (year, month, day), after Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// First matching song's thumbnail, upscaled. `Ok(None)` = the endpoint
/// answered and nothing matched (a settled miss).
pub(crate) fn pick_cover(
    body: &str,
    artist: &str,
    album: &str,
) -> Result<Option<String>, CoverError> {
    let parsed: Value = serde_json::from_str(body).map_err(|_| CoverError::Malformed)?;
    let mut items = Vec::new();
    collect_song_items(&parsed, &mut items);
    Ok(items.into_iter().find_map(|item| {
        let subtitle = flex_text(item, 1)?;
        candidate_matches(artist, album, &subtitle).then(|| thumbnail_url(item))?
    }))
}

/// Every `musicResponsiveListItemRenderer` in the tree, however deeply nested
/// — resilient to the shelf/section wrappers the endpoint rearranges.
fn collect_song_items<'a>(value: &'a Value, out: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if let Some(item) = map.get("musicResponsiveListItemRenderer") {
                out.push(item);
            } else {
                for child in map.values() {
                    collect_song_items(child, out);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_song_items(child, out);
            }
        }
        _ => {}
    }
}

/// The text of `flexColumns[index]`, joined across its runs.
fn flex_text(item: &Value, index: usize) -> Option<String> {
    let runs = item
        .get("flexColumns")?
        .get(index)?
        .get("musicResponsiveListItemFlexColumnRenderer")?
        .get("text")?
        .get("runs")?
        .as_array()?;
    let mut text = String::new();
    for run in runs {
        if let Some(part) = run.get("text").and_then(Value::as_str) {
            text.push_str(part);
        }
    }
    (!text.trim().is_empty()).then_some(text)
}

/// Largest thumbnail in the item, upscaled.
fn thumbnail_url(item: &Value) -> Option<String> {
    let thumbs = item
        .get("thumbnail")?
        .get("musicThumbnailRenderer")?
        .get("thumbnail")?
        .get("thumbnails")?
        .as_array()?;
    thumbs
        .iter()
        .filter_map(|thumb| {
            let url = thumb.get("url")?.as_str()?;
            let width = thumb.get("width").and_then(Value::as_u64).unwrap_or(0);
            Some((width, url))
        })
        .max_by_key(|(width, _)| *width)
        .map(|(_, url)| bigger_artwork(url))
}

/// Does the artist/album subtitle describe the release we asked for?
///
/// YouTube Music's subtitle reads `Artist • Album • Year`; match on the
/// normalized parts, strictly, like the iTunes provider. A wrong cover on
/// every track of an album is worse than no cover.
pub(crate) fn candidate_matches(artist: &str, album: &str, subtitle: &str) -> bool {
    let parts: Vec<String> = subtitle
        .split('•')
        .map(crate::itunes::normalize)
        .filter(|part| !part.is_empty())
        .collect();
    let artist_ok = artist.trim().is_empty()
        || parts
            .iter()
            .any(|part| part == &crate::itunes::normalize(artist));
    let album_ok = album.trim().is_empty() || {
        let want = crate::itunes::normalize(crate::itunes::base_album(album));
        !want.is_empty()
            && parts
                .iter()
                .any(|part| crate::itunes::normalize(crate::itunes::base_album(part)) == want)
    };
    artist_ok && album_ok
}

/// Raise a Google image URL's size suffix to 600px (`=w60-h60` → `=w600-h600`).
/// URLs without that suffix (e.g. `i.ytimg.com`) are returned unchanged.
pub(crate) fn bigger_artwork(url: &str) -> String {
    let Some(eq) = url.find("=w") else {
        return url.to_string();
    };
    let rest = &url[eq + 2..];
    let Some(dash) = rest.find("-h") else {
        return url.to_string();
    };
    let after = &rest[dash + 2..];
    let height_digits = after.chars().take_while(char::is_ascii_digit).count();
    if height_digits == 0 {
        return url.to_string();
    }
    format!("{}=w600-h600{}", &url[..eq], &after[height_digits..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, subtitle: &str, thumb: &str) -> String {
        format!(
            r#"{{"musicResponsiveListItemRenderer":{{
                "flexColumns":[
                  {{"musicResponsiveListItemFlexColumnRenderer":{{"text":{{"runs":[{{"text":"{title}"}}]}}}}}},
                  {{"musicResponsiveListItemFlexColumnRenderer":{{"text":{{"runs":[{{"text":"{subtitle}"}}]}}}}}}
                ],
                "thumbnail":{{"musicThumbnailRenderer":{{"thumbnail":{{"thumbnails":[
                  {{"url":"https://lh3.googleusercontent.com/x=w60-h60-l90-rj","width":60,"height":60}},
                  {{"url":"{thumb}","width":544,"height":544}}
                ]}}}}}}
            }}}}"#
        )
    }

    fn body(items: &[String]) -> String {
        format!(
            r#"{{"contents":{{"tabbedSearchResultsRenderer":{{"tabs":[{{"tabRenderer":{{"content":{{"sectionListRenderer":{{"contents":[{{"musicShelfRenderer":{{"contents":[{items}]}}}}]}}}}}}}}]}}}}}}"#,
            items = items.join(",")
        )
    }

    fn found(body: &str, artist: &str, album: &str) -> Option<String> {
        pick_cover(body, artist, album).expect("parsable response")
    }

    #[test]
    fn pick_cover_matches_the_artist_and_album_subtitle() {
        let raw = body(&[item(
            "Surabaya",
            "Crayon Case • Surabaya • 2020",
            "https://lh3.googleusercontent.com/ok=w544-h544-l90-rj",
        )]);
        let url = found(&raw, "Crayon Case", "Surabaya").expect("match");
        assert!(url.contains("/ok=w600-h600"), "{url}");
    }

    #[test]
    fn pick_cover_skips_a_wrong_artist() {
        let raw = body(&[item(
            "Surabaya",
            "Tribute Band • Surabaya",
            "https://lh3.googleusercontent.com/no=w544-h544",
        )]);
        assert!(found(&raw, "Crayon Case", "Surabaya").is_none());
    }

    #[test]
    fn pick_cover_skips_a_different_album() {
        let raw = body(&[item(
            "Greatest Hits",
            "Crayon Case • Greatest Hits",
            "https://lh3.googleusercontent.com/no=w544-h544",
        )]);
        assert!(found(&raw, "Crayon Case", "Surabaya").is_none());
    }

    #[test]
    fn pick_cover_takes_the_first_matching_item() {
        let raw = body(&[
            item(
                "Other",
                "Other Artist • Other Album",
                "https://lh3.googleusercontent.com/no=w544-h544",
            ),
            item(
                "Surabaya",
                "Crayon Case • Surabaya",
                "https://lh3.googleusercontent.com/yes=w544-h544",
            ),
        ]);
        let url = found(&raw, "Crayon Case", "Surabaya").expect("second hits");
        assert!(url.contains("/yes"), "{url}");
    }

    #[test]
    fn pick_cover_accepts_any_release_when_the_album_is_untagged() {
        let raw = body(&[item("X", "Crayon Case • Whatever", "https://x=w544-h544")]);
        assert!(found(&raw, "Crayon Case", "").is_some());
    }

    #[test]
    fn pick_cover_is_a_settled_miss_for_an_empty_result_set() {
        let raw = r#"{"contents":{}}"#;
        assert!(pick_cover(raw, "Crayon Case", "Surabaya")
            .expect("parsable")
            .is_none());
    }

    #[test]
    fn pick_cover_reports_unparsable_json_as_malformed() {
        assert!(matches!(
            pick_cover("{", "A", "B"),
            Err(CoverError::Malformed)
        ));
    }

    #[test]
    fn candidate_matches_ignores_release_qualifiers() {
        assert!(candidate_matches(
            "Crayon Case",
            "Surabaya",
            "Crayon Case • Surabaya - Single • 2020"
        ));
    }

    #[test]
    fn bigger_artwork_bumps_the_google_size_suffix() {
        assert_eq!(
            bigger_artwork("https://lh3.googleusercontent.com/x=w60-h60-l90-rj"),
            "https://lh3.googleusercontent.com/x=w600-h600-l90-rj"
        );
    }

    #[test]
    fn bigger_artwork_leaves_other_urls_alone() {
        let url = "https://i.ytimg.com/vi/abc/maxresdefault.jpg";
        assert_eq!(bigger_artwork(url), url);
    }

    #[test]
    fn extract_visitor_id_reads_the_ytcfg_blob() {
        let html = r#"<script>ytcfg.set({"X":"1","VISITOR_DATA":"CgtAbc123","Y":"2"});</script>"#;
        assert_eq!(extract_visitor_id(html).as_deref(), Some("CgtAbc123"));
        assert_eq!(extract_visitor_id("<html></html>"), None);
    }

    #[test]
    fn request_body_carries_the_web_remix_client_and_query() {
        let body = request_body("Crayon Case", "Surabaya");
        assert!(body.contains("\"clientName\":\"WEB_REMIX\""), "{body}");
        assert!(
            body.contains("\"query\":\"Crayon Case Surabaya\""),
            "{body}"
        );
        assert!(body.contains("\"clientVersion\":\"1."), "{body}");
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }
}
