//! The workspace's only network calls, shared by every provider.
//!
//! Keeping the transport here means one place owns the timeout and the
//! error mapping, so a slow or dead network always collapses to the same
//! retryable [`CoverError::Request`] regardless of which provider asked.

use std::time::Duration;

use crate::CoverError;

/// Whole-request budget. Lookups run off the playback path, so a slow or dead
/// network must not pile up threads behind the sink.
const TIMEOUT: Duration = Duration::from_secs(5);

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .new_agent()
}

/// GET a URL and return the body as text.
pub(crate) fn get(url: &str) -> Result<String, CoverError> {
    finish(agent().get(url).call())
}

/// GET with extra request headers (the YouTube Music visitor-id bootstrap).
pub(crate) fn get_with(url: &str, headers: &[(&str, &str)]) -> Result<String, CoverError> {
    let mut request = agent().get(url);
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    finish(request.call())
}

/// POST a JSON body and return the response as text.
pub(crate) fn post_json(
    url: &str,
    body: &str,
    headers: &[(&str, &str)],
) -> Result<String, CoverError> {
    let mut request = agent().post(url).header("content-type", "application/json");
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    finish(request.send(body))
}

/// Collapse a ureq outcome to [`CoverError`], reading the body on success.
fn finish(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<String, CoverError> {
    match result {
        Ok(mut response) => response
            .body_mut()
            .read_to_string()
            .map_err(|e| CoverError::Request(e.to_string())),
        Err(ureq::Error::StatusCode(code)) => Err(CoverError::Request(format!("HTTP {code}"))),
        Err(e) => Err(CoverError::Request(e.to_string())),
    }
}
