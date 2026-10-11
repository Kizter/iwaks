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
/// Key holding the user's saved [`EqPreset`] list.
pub const SETTING_EQ_PRESETS: &str = "player.eq_presets";

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

/// A named equalizer setup saved by the user. Built-in templates live in the
/// frontend; only the user's own presets round-trip through here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqPreset {
    pub name: String,
    pub preamp: f64,
    /// 10 band gains in dB at the ISO frequencies.
    pub gains: Vec<f64>,
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

    /// Write the saved equalizer presets, replacing any previous list.
    pub fn save_eq_presets(&self, presets: &[EqPreset]) -> Result<(), LibraryError> {
        self.set_setting(SETTING_EQ_PRESETS, &serde_json::to_string(presets)?)
    }

    /// The saved equalizer presets; empty when none were ever saved.
    pub fn load_eq_presets(&self) -> Result<Vec<EqPreset>, LibraryError> {
        match self.setting(SETTING_EQ_PRESETS)? {
            Some(raw) => Ok(serde_json::from_str(&raw)?),
            None => Ok(Vec::new()),
        }
    }
}
