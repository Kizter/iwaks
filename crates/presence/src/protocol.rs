//! Discord RPC wire protocol — pure encode/decode + activity JSON.
//!
//! Frame layout: 8-byte header (u32 little-endian opcode, then u32
//! little-endian payload length) followed by the JSON payload.

use thiserror::Error;

/// RPC opcode: client → server handshake.
pub const OP_HANDSHAKE: u32 = 0;
/// RPC opcode: generic command frame (e.g. `SET_ACTIVITY`), also used for
/// server events such as `READY`.
pub const OP_FRAME: u32 = 1;
/// RPC opcode: close the connection.
pub const OP_CLOSE: u32 = 2;
/// RPC opcode: heartbeat request — the server sends this; we must answer
/// with [`OP_PONG`] echoing the payload.
pub const OP_PING: u32 = 3;
/// RPC opcode: heartbeat response.
pub const OP_PONG: u32 = 4;

/// Maximum length (in chars) of Discord `details` / `state` / text fields.
pub const FIELD_LIMIT: usize = 128;
/// Maximum length (in chars) of an asset key (`large_image` / `small_image`).
pub const ASSET_KEY_LIMIT: usize = 32;

/// One decoded RPC frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub opcode: u32,
    pub payload: String,
}

/// Why a frame buffer could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FrameError {
    #[error("frame is shorter than the 8-byte header")]
    TooShort,
    #[error("payload is shorter than the length declared in the header")]
    Incomplete,
    #[error("unknown opcode {0}")]
    UnknownOpcode(u32),
    #[error("payload is not valid UTF-8")]
    InvalidUtf8,
}

/// Encode a frame: 8-byte little-endian header (opcode, then length) +
/// payload bytes. Verified against the real Discord client: the wire header
/// is `[opcode][length]`, NOT `[length][opcode]`.
pub fn encode_frame(opcode: u32, payload: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload.as_bytes());
    out
}

/// Decode a complete frame from a buffer holding at least the header + the
/// declared payload length. Trailing bytes past the declared length are
/// ignored, so buffered reads can be sliced.
pub fn decode_frame(buf: &[u8]) -> Result<Frame, FrameError> {
    if buf.len() < 8 {
        return Err(FrameError::TooShort);
    }
    let opcode = u32::from_le_bytes(buf[..4].try_into().unwrap());
    let len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let payload = buf
        .get(8..)
        .filter(|p| p.len() >= len)
        .ok_or(FrameError::Incomplete)?;
    let payload = std::str::from_utf8(&payload[..len]).map_err(|_| FrameError::InvalidUtf8)?;
    if !matches!(
        opcode,
        OP_HANDSHAKE | OP_FRAME | OP_CLOSE | OP_PING | OP_PONG
    ) {
        return Err(FrameError::UnknownOpcode(opcode));
    }
    Ok(Frame {
        opcode,
        payload: payload.to_string(),
    })
}

/// Handshake payload: `{"v":1,"client_id":"<id>"}`.
pub fn handshake_payload(client_id: &str) -> String {
    serde_json::json!({ "v": 1, "client_id": client_id }).to_string()
}

/// A per-message unique id (`<pid>-<seq>`); Discord echoes it in responses.
pub fn nonce(pid: u32, seq: u64) -> String {
    format!("{pid}-{seq}")
}

/// Closed-open interval timestamps for the activity (Unix seconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timestamps {
    pub start: Option<u64>,
    pub end: Option<u64>,
}

/// Rich presence activity. Missing fields are omitted from the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    /// Discord activity type: 2 = Listening.
    pub kind: u32,
    /// Short label Discord repeats next to the user in the member list
    /// ("Listening to <name>").
    pub name: Option<String>,
    pub details: Option<String>,
    pub state: Option<String>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    pub timestamps: Option<Timestamps>,
}

impl Activity {
    /// A fresh Listening activity with no text fields set.
    pub fn listening() -> Self {
        Self {
            kind: 2,
            name: None,
            details: None,
            state: None,
            large_image: None,
            large_text: None,
            small_image: None,
            small_text: None,
            timestamps: None,
        }
    }

    /// Member-list label. Verified against the live client: this field — not
    /// `details` — is what Discord shows beside the user, so the track title
    /// belongs here.
    pub fn name(mut self, value: &str) -> Self {
        self.name = Some(clamp_field(value));
        self
    }

    pub fn details(mut self, value: &str) -> Self {
        self.details = Some(clamp_field(value));
        self
    }

    pub fn state(mut self, value: &str) -> Self {
        self.state = Some(clamp_field(value));
        self
    }

    pub fn large_image(mut self, key: &str) -> Self {
        self.large_image = Some(clamp_key(key));
        self
    }

    pub fn large_text(mut self, value: &str) -> Self {
        self.large_text = Some(clamp_field(value));
        self
    }

    /// Small overlay image. Only names of assets uploaded in the Discord
    /// Developer Portal render here — URL and `file://` values are dropped by
    /// Discord, so the key must match the portal exactly.
    pub fn small_image(mut self, key: &str) -> Self {
        self.small_image = Some(clamp_key(key));
        self
    }

    pub fn small_text(mut self, value: &str) -> Self {
        self.small_text = Some(clamp_field(value));
        self
    }

    pub fn timestamps(mut self, timestamps: Timestamps) -> Self {
        self.timestamps = Some(timestamps);
        self
    }
}

/// Truncate to [`FIELD_LIMIT`] chars, never splitting a UTF-8 char.
fn clamp_field(value: &str) -> String {
    value.chars().take(FIELD_LIMIT).collect()
}

/// Truncate to [`ASSET_KEY_LIMIT`] chars.
fn clamp_key(value: &str) -> String {
    value.chars().take(ASSET_KEY_LIMIT).collect()
}

/// Build the `SET_ACTIVITY` payload. `None` clears the presence
/// (`"activity":{}`), keeping the RPC connection alive.
pub fn set_activity_json(pid: u32, nonce: &str, activity: Option<&Activity>) -> String {
    let activity_value = match activity {
        None => serde_json::json!({}),
        Some(a) => {
            let mut map = serde_json::Map::new();
            map.insert("type".to_string(), serde_json::json!(a.kind));
            if let Some(n) = &a.name {
                map.insert("name".to_string(), serde_json::json!(n));
            }
            if let Some(d) = &a.details {
                map.insert("details".to_string(), serde_json::json!(d));
            }
            if let Some(s) = &a.state {
                map.insert("state".to_string(), serde_json::json!(s));
            }
            if a.large_image.is_some()
                || a.large_text.is_some()
                || a.small_image.is_some()
                || a.small_text.is_some()
            {
                let mut assets = serde_json::Map::new();
                if let Some(k) = &a.large_image {
                    assets.insert("large_image".to_string(), serde_json::json!(k));
                }
                if let Some(t) = &a.large_text {
                    assets.insert("large_text".to_string(), serde_json::json!(t));
                }
                if let Some(k) = &a.small_image {
                    assets.insert("small_image".to_string(), serde_json::json!(k));
                }
                if let Some(t) = &a.small_text {
                    assets.insert("small_text".to_string(), serde_json::json!(t));
                }
                map.insert("assets".to_string(), serde_json::Value::Object(assets));
            }
            if let Some(t) = &a.timestamps {
                let mut ts = serde_json::Map::new();
                if let Some(start) = t.start {
                    ts.insert("start".to_string(), serde_json::json!(start));
                }
                if let Some(end) = t.end {
                    ts.insert("end".to_string(), serde_json::json!(end));
                }
                map.insert("timestamps".to_string(), serde_json::Value::Object(ts));
            }
            serde_json::Value::Object(map)
        }
    };
    serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": pid, "activity": activity_value },
        "nonce": nonce,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_frame_writes_le_header_and_payload() {
        let bytes = encode_frame(OP_FRAME, "{\"a\":1}");
        assert_eq!(&bytes[..4], &OP_FRAME.to_le_bytes());
        assert_eq!(&bytes[4..8], &7u32.to_le_bytes());
        assert_eq!(&bytes[8..], b"{\"a\":1}");
    }

    #[test]
    fn decode_roundtrip() {
        let bytes = encode_frame(OP_PING, "payload");
        let f = decode_frame(&bytes).unwrap();
        assert_eq!(f.opcode, OP_PING);
        assert_eq!(f.payload, "payload");
    }

    #[test]
    fn decode_rejects_short_buffer() {
        assert!(matches!(
            decode_frame(&[0, 1, 2]),
            Err(FrameError::TooShort)
        ));
    }

    #[test]
    fn decode_rejects_declared_length_longer_than_payload() {
        let mut bytes = OP_FRAME.to_le_bytes().to_vec();
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(b"ab");
        assert!(matches!(decode_frame(&bytes), Err(FrameError::Incomplete)));
    }

    #[test]
    fn decode_rejects_unknown_opcode() {
        let mut bytes = 99u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert!(matches!(
            decode_frame(&bytes),
            Err(FrameError::UnknownOpcode(99))
        ));
    }

    #[test]
    fn decode_rejects_invalid_utf8_payload() {
        let mut bytes = OP_FRAME.to_le_bytes().to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(0xFF);
        assert!(matches!(decode_frame(&bytes), Err(FrameError::InvalidUtf8)));
    }

    #[test]
    fn decode_ignores_trailing_bytes_beyond_declared_length() {
        let mut bytes = encode_frame(OP_FRAME, "ab");
        bytes.extend_from_slice(b"zzz");
        let f = decode_frame(&bytes).unwrap();
        assert_eq!(f.payload, "ab");
    }

    #[test]
    fn handshake_payload_is_valid_json() {
        let v: serde_json::Value = serde_json::from_str(&handshake_payload("1553")).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["client_id"], "1553");
    }

    #[test]
    fn nonce_is_pid_seq() {
        assert_eq!(nonce(1234, 7), "1234-7");
    }

    #[test]
    fn set_activity_listening_payload() {
        let act = Activity::listening()
            .details("Lagu A")
            .state("Artis — Album")
            .large_image("art")
            .timestamps(Timestamps {
                start: Some(1000),
                end: None,
            });
        let v: serde_json::Value =
            serde_json::from_str(&set_activity_json(42, &nonce(42, 1), Some(&act))).unwrap();
        assert_eq!(v["cmd"], "SET_ACTIVITY");
        assert_eq!(v["args"]["pid"], 42);
        assert_eq!(v["args"]["activity"]["type"], 2);
        assert_eq!(v["args"]["activity"]["details"], "Lagu A");
        assert_eq!(v["args"]["activity"]["state"], "Artis — Album");
        assert_eq!(v["args"]["activity"]["assets"]["large_image"], "art");
        assert_eq!(v["args"]["activity"]["timestamps"]["start"], 1000);
        assert_eq!(v["nonce"], "42-1");
    }

    #[test]
    fn set_activity_none_sends_empty_activity() {
        let v: serde_json::Value = serde_json::from_str(&set_activity_json(1, "x", None)).unwrap();
        assert_eq!(v["args"]["activity"], serde_json::json!({}));
    }

    /// `name` is the member-list line; it must serialize even when no other
    /// text field is set.
    #[test]
    fn name_serializes_next_to_type() {
        let act = Activity::listening().name("Surabaya - Iwaks");
        let v: serde_json::Value =
            serde_json::from_str(&set_activity_json(1, "x", Some(&act))).unwrap();
        assert_eq!(v["args"]["activity"]["name"], "Surabaya - Iwaks");
        assert!(v["args"]["activity"].get("details").is_none());
    }

    /// Both image slots live in one `assets` object — Discord rejects the
    /// activity if the small image is sent as a sibling of it.
    #[test]
    fn small_image_shares_the_assets_object() {
        let act = Activity::listening()
            .large_image("logo")
            .large_text("Iwaks")
            .small_image("note")
            .small_text("now playing");
        let v: serde_json::Value =
            serde_json::from_str(&set_activity_json(1, "x", Some(&act))).unwrap();
        let assets = &v["args"]["activity"]["assets"];
        assert_eq!(assets["large_image"], "logo");
        assert_eq!(assets["large_text"], "Iwaks");
        assert_eq!(assets["small_image"], "note");
        assert_eq!(assets["small_text"], "now playing");
    }

    /// A small image alone still has to produce an `assets` object.
    #[test]
    fn small_image_alone_still_emits_assets() {
        let act = Activity::listening().small_image("note");
        let v: serde_json::Value =
            serde_json::from_str(&set_activity_json(1, "x", Some(&act))).unwrap();
        assert_eq!(v["args"]["activity"]["assets"]["small_image"], "note");
        assert!(v["args"]["activity"]["assets"].get("large_image").is_none());
    }

    #[test]
    fn details_clamped_to_128_chars() {
        let act = Activity::listening().details(&"x".repeat(300));
        let v: serde_json::Value =
            serde_json::from_str(&set_activity_json(1, "x", Some(&act))).unwrap();
        assert_eq!(
            v["args"]["activity"]["details"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            128
        );
    }

    #[test]
    fn asset_key_clamped_to_32_chars() {
        let act = Activity::listening().large_image(&"k".repeat(80));
        let v: serde_json::Value =
            serde_json::from_str(&set_activity_json(1, "x", Some(&act))).unwrap();
        assert_eq!(
            v["args"]["activity"]["assets"]["large_image"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            32
        );
    }

    #[test]
    fn clamping_keeps_utf8_char_boundaries() {
        // "é" is 2 bytes; truncating at byte 127 would split it.
        let act = Activity::listening().details(&"é".repeat(200));
        let v: serde_json::Value =
            serde_json::from_str(&set_activity_json(1, "x", Some(&act))).unwrap();
        let details = v["args"]["activity"]["details"].as_str().unwrap();
        assert_eq!(details.chars().count(), 128);
        assert!(details.chars().all(|c| c == 'é'));
    }

    #[test]
    fn missing_fields_are_omitted() {
        let act = Activity::listening();
        let v: serde_json::Value =
            serde_json::from_str(&set_activity_json(1, "x", Some(&act))).unwrap();
        let activity = &v["args"]["activity"];
        assert_eq!(activity["type"], 2);
        assert!(activity.get("details").is_none());
        assert!(activity.get("state").is_none());
        assert!(activity.get("assets").is_none());
        assert!(activity.get("timestamps").is_none());
    }
}
