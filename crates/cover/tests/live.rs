//! Live provider smoke test.
//!
//! Opt-in, because it performs a real request to Apple's public search API:
//! ```text
//! cargo test -p iwaks-cover --test live -- --ignored --nocapture
//! ```
//!
//! What it protects is the *provider contract*, which unit tests cannot: the
//! endpoint's URL shape, its camelCase field names, and the CDN path convention
//! that lets a 100px thumbnail be upscaled by string replacement. If Apple
//! renames a field, this fails where the mocked tests would keep passing.
//!
//! It asserts nothing about a specific album's availability — a release can
//! leave the store. It asserts that a *known* release is still reachable and
//! still yields a 600px artwork URL.

use iwaks_cover::lookup;

/// A release that has been on the store for years, used as the canary.
const ARTIST: &str = "Crayon Case";
const ALBUM: &str = "Surabaya";

#[test]
#[ignore = "performs a real network request"]
fn a_known_album_resolves_to_a_reachable_600px_artwork_url() {
    let found = lookup(ARTIST, ALBUM)
        .expect("lookup should be answerable")
        .expect("this release should still be listed");

    assert!(
        found.starts_with("https://"),
        "artwork must be a public https URL: {found}"
    );
    assert!(
        found.contains("600x600bb"),
        "thumbnail should have been upscaled: {found}"
    );
    println!("resolved artwork: {found}");
}

#[test]
#[ignore = "performs a real network request"]
fn the_upscaled_url_actually_serves_an_image() {
    let found = lookup(ARTIST, ALBUM)
        .expect("answerable")
        .expect("this release should still be listed");
    let response = ureq::get(&found)
        .call()
        .expect("artwork URL must be fetchable");
    assert_eq!(response.status(), 200, "{found}");
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("image/")),
        "artwork must be an image: {found}"
    );
}

#[test]
#[ignore = "performs a real network request"]
fn an_album_absent_from_the_store_is_an_answer_not_an_error() {
    // The distinction the cover cache depends on: `Ok(None)` gets remembered as
    // "no cover for this album", while `Err` must stay retryable. A release the
    // store has never heard of is the first case.
    let outcome = lookup(
        "Zzzqqx Nonexistent Artist 9f3a",
        "Zzzqqx Nonexistent Album 9f3a",
    );
    assert!(
        matches!(outcome, Ok(None)),
        "expected a settled \"no match\", got {outcome:?}"
    );
}
