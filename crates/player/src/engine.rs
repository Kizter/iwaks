//! The libmpv cores that make up one playback session.
//!
//! A session owns [`CORES`] handles: the one you hear, and the one the next
//! track is prefetched into for a crossfade (issue #27). Only the **pump thread**
//! ever calls into a core — `mpv_wait_event(h, 0)` is non-blocking, so one
//! thread can poll every core inside the same loop iteration. Keeping that
//! invariant is the point: the hardest bugs in this crate (#03, the END_FILE
//! property-read hazard) all came from touching libmpv off the pump thread.
//!
//! Stage 1 of the crossfade plan: this type exists so that "the other core" is
//! a slot rather than a special case. It initialises **one** core, exactly as
//! before; the second slot is created lazily when a prefetch first needs it.

use std::sync::Arc;

use crate::ffi::Mpv;
use crate::player::{Options, ReplayGainMode};

/// Cores per session: the audible one plus the prefetch target.
pub const CORES: usize = 2;

/// Which core is playing, and which one is free.
pub struct Engine {
    cores: [Option<Arc<Mpv>>; CORES],
    /// Index of the audible core.
    active: usize,
    /// Startup options, kept so a second core is created exactly like the first.
    options: Options,
    /// The `af` chain applied to the idle core. Tracked because the user's EQ
    /// can change *while* a track is prefetched, and the incoming core has to be
    /// brought up to date before it is allowed to play (stage 3).
    idle_af: Option<String>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::empty()
    }
}

impl Engine {
    /// A session with no core yet — the state before the pump starts.
    pub fn empty() -> Self {
        Self {
            cores: [None, None],
            active: 0,
            options: Options::default(),
            idle_af: None,
        }
    }

    /// Create and initialise the core at `slot`, applying the same startup
    /// options to it. Fails without touching the other slot.
    ///
    /// Runs on the pump thread (it calls libmpv).
    pub fn init_core(&mut self, slot: usize, options: &Options) -> Result<Arc<Mpv>, String> {
        assert!(slot < CORES, "core slot out of range");
        let api = setup_core(options)?;
        self.cores[slot] = Some(api);
        if slot == self.active {
            self.options = options.clone();
        }
        Ok(self.cores[slot].clone().expect("just stored"))
    }

    /// Initialise the audible core (slot 0). The usual entry point.
    pub fn init_primary(&mut self, options: &Options) -> Result<Arc<Mpv>, String> {
        self.init_core(0, options)
    }

    /// Create the idle core if it does not exist yet, and return it. `None` when
    /// libmpv refuses a second core — the caller then simply prefetches nothing.
    pub fn ensure_idle(&mut self) -> Option<Arc<Mpv>> {
        if let Some(existing) = self.idle() {
            return Some(existing);
        }
        let options = self.options.clone();
        let slot = self.idle_slot();
        match self.init_core(slot, &options) {
            Ok(api) => Some(api),
            Err(e) => {
                iwaks_log::line(
                    iwaks_log::Level::Warn,
                    &format!("could not create the second playback core ({e})"),
                );
                None
            }
        }
    }

    /// Remove the idle core and hand it back so the caller can stop it with
    /// libmpv (pump-thread work). Dropping the returned handle releases it.
    pub fn take_idle(&mut self) -> Option<Arc<Mpv>> {
        let taken = self.cores[self.idle_slot()].take();
        self.idle_af = None;
        taken
    }

    /// Is a second core loaded and ready?
    pub fn has_idle(&self) -> bool {
        self.cores[self.idle_slot()].is_some()
    }

    /// The `af` chain the idle core was given, if any.
    pub fn idle_af(&self) -> Option<&str> {
        self.idle_af.as_deref()
    }

    /// Record the `af` chain the idle core was given.
    pub fn set_idle_af(&mut self, chain: Option<String>) {
        self.idle_af = chain;
    }

    /// The core that plays.
    ///
    /// Panics when the session never started, which is the same contract the
    /// single-handle version had: every caller runs after `Player::start`
    /// returned `Ok`.
    pub fn active(&self) -> Arc<Mpv> {
        self.cores[self.active].clone().expect("player not started")
    }

    /// The core that is not playing — where a prefetch would go. `None` while
    /// it has not been created yet.
    pub fn idle(&self) -> Option<Arc<Mpv>> {
        self.cores[self.idle_slot()].clone()
    }

    /// Index of the non-playing slot.
    pub fn idle_slot(&self) -> usize {
        1 - self.active
    }

    /// Index of the playing slot.
    pub fn active_slot(&self) -> usize {
        self.active
    }

    /// The core at `slot`, if it exists.
    pub fn core(&self, slot: usize) -> Option<Arc<Mpv>> {
        self.cores.get(slot).cloned().flatten()
    }

    /// Whether `api` is the core currently playing. Lets the pump tell which
    /// instance an event came from without holding the engine lock.
    pub fn is_active(&self, api: &Arc<Mpv>) -> bool {
        self.cores[self.active]
            .as_ref()
            .map(|active| Arc::ptr_eq(active, api))
            .unwrap_or(false)
    }

    /// Hand playback to the other core. The caller is responsible for having
    /// stopped the outgoing one.
    pub fn swap(&mut self) {
        self.active = self.idle_slot();
    }

    /// True when no core has been created (before `Player::start`).
    pub fn is_empty(&self) -> bool {
        self.cores[self.active].is_none()
    }
}

/// Create one mpv core and apply the session's startup options to it.
///
/// Split out of `Player` in stage 1 because a second core needs exactly the
/// same setup — anything added here applies to a prefetching core too, which is
/// how the fade-in gets the EQ and ReplayGain the rest of the album uses.
fn setup_core(options: &Options) -> Result<Arc<Mpv>, String> {
    let api = Mpv::new()?;
    let apply = |key: &str, value: &str| -> Result<(), String> {
        api.set_option(key, value)
            .map_err(|code| format!("option {key}={value}: {code}"))
    };
    apply("idle", "yes")?; // stay alive between files
    apply("vo", "null")?; // headless app: no window
    apply("ao", if options.ao_null { "null" } else { "wasapi" })?;
    if options.ao_null {
        // Real-time pacing so position/seek assertions are deterministic.
        apply("ao-null-untimed", "no")?;
    }
    if options.audio_exclusive {
        apply("audio-exclusive", "yes")?;
    }
    if !options.ao_null && !options.audio_device.is_empty() {
        // Startup device choice (issue #23); "auto" when nothing is stored.
        apply("audio-device", &options.audio_device)?;
    }
    apply("audio-client-name", "Iwaks")?;
    apply("volume-max", "100")?;
    apply("keep-open", "no")?;
    apply("gapless-audio", "yes")?;
    // ReplayGain on by default (design §4.2: `replaygain=track`); the mode is
    // switchable at runtime (`set replaygain <mode>`), read back each tick, and
    // applied from the next loaded file onward.
    apply("replaygain", ReplayGainMode::Track.as_mpv())?;
    apply("replaygain-preamp", "0")?;
    apply("replaygain-clip", "yes")?;
    apply("replaygain-fallback", "0")?;
    let _ = api.set_option("audio-resampler", "soxr"); // optional build; harmless
    if let Ok(level) = std::env::var("IWAKS_MPV_LOG") {
        let _ = api.request_log_messages(&level); // debug hook
    }
    api.initialize()
        .map_err(|code| format!("mpv_initialize failed: {code}"))?;
    Ok(Arc::new(api))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slot logic is pure arithmetic, so it is testable without a libmpv
    /// DLL — which matters, because CI machines have none.
    #[test]
    fn an_empty_engine_has_no_active_core() {
        let engine = Engine::empty();
        assert!(engine.is_empty());
        assert_eq!(engine.active_slot(), 0);
        assert_eq!(engine.idle_slot(), 1);
        // `Mpv` is neither `Debug` nor `PartialEq`, so presence is asserted
        // with `is_some`/`is_none` rather than `assert_eq!`.
        assert!(engine.idle().is_none());
        assert!(engine.core(0).is_none());
        assert!(
            engine.core(CORES).is_none(),
            "out-of-range slots read as None, not a panic"
        );
    }

    #[test]
    fn swapping_flips_which_slot_is_idle() {
        let mut engine = Engine::empty();
        engine.swap();
        assert_eq!(engine.active_slot(), 1);
        assert_eq!(engine.idle_slot(), 0);
        engine.swap();
        assert_eq!(engine.active_slot(), 0);
    }

    #[test]
    fn the_idle_slot_is_never_the_active_one() {
        let mut engine = Engine::empty();
        for _ in 0..4 {
            assert_ne!(engine.active_slot(), engine.idle_slot());
            engine.swap();
        }
    }

    #[test]
    fn default_is_an_empty_engine() {
        assert!(Engine::default().is_empty());
    }
}
