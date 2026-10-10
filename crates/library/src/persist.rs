//! Durable player state: the preferences and last-session snapshot the app
//! restores on launch. Stored as JSON in the generic `settings` table so the
//! player crate stays storage-agnostic — it only knows `PlayerState`, and the
//! app layer maps to/from these plain structs.

use serde::{Deserialize, Serialize};

use crate::db::{Library, LibraryError};

/// Key holding the [`PlayerSettings`] blob.
pub const SETTING_PLAYER: &str = "player.settings";
/// Key holding the [`SessionSnapshot`] blob.
pub const SETTING_SESSION: &str = "player.session";

/// Player preferences that survive a restart.
///
/// Enum-like fields carry the player's own lowercase wire names
/// (`"off" | "all" | "one"`, `"track" | "album" | "off"`) so this crate needs
/// no dependency on `iwaks-player`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSettings {
    pub volume: i64,
    pub mute: bool,
    pub repeat: String,
    pub shuffle: bool,
    pub speed: f64,
    pub replaygain: String,
    pub eq_preamp: f64,
    pub eq: Vec<f64>,
    /// Seconds left on the sleep timer when the app last wrote state.
    pub sleep_seconds: Option<f64>,
}

/// Queue + position captured so a restart can resume where playback left off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    /// Track paths in play order (the shuffle permutation when shuffle is on).
    pub paths: Vec<String>,
    /// Index into `paths` of the track that was current.
    pub index: usize,
    /// Playback position of that track, in seconds.
    pub position: f64,
    pub shuffle: bool,
}

impl Library {
    /// Write the player preferences blob, replacing any previous value.
    pub fn save_player_settings(&self, settings: &PlayerSettings) -> Result<(), LibraryError> {
        self.set_setting(SETTING_PLAYER, &serde_json::to_string(settings)?)
    }

    /// The stored player preferences, or `None` when nothing was ever written.
    pub fn load_player_settings(&self) -> Result<Option<PlayerSettings>, LibraryError> {
        match self.setting(SETTING_PLAYER)? {
            Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
            None => Ok(None),
        }
    }

    /// Write the last-session snapshot, replacing any previous value.
    pub fn save_session(&self, session: &SessionSnapshot) -> Result<(), LibraryError> {
        self.set_setting(SETTING_SESSION, &serde_json::to_string(session)?)
    }

    /// The stored session snapshot, or `None` when nothing was ever written.
    pub fn load_session(&self) -> Result<Option<SessionSnapshot>, LibraryError> {
        match self.setting(SETTING_SESSION)? {
            Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
            None => Ok(None),
        }
    }
}
