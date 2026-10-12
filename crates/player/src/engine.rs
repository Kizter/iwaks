//! The libmpv cores that make up one playback session.
//!
//! A session owns [`CORES`] handles: the one you hear, the one fading out after
//! a crossfade handover, and the one the next track is prefetched into. Only the
//! **pump thread** ever calls into a core — `mpv_wait_event(h, 0)` is
//! non-blocking, so one thread can poll every core inside the same loop
//! iteration. Keeping that invariant is the point: the hardest bugs in this
//! crate (#03, the END_FILE property-read hazard) all came from touching libmpv
//! off the pump thread.
//!
//! Stage 1 introduced the slots, stage 2 the prefetch, stage 3 the fade. The
//! second core is created lazily, so a session that never crossfades only ever
//! has one.

use std::sync::Arc;

use crate::ffi::Mpv;
use crate::player::{Options, ReplayGainMode};

/// Cores per session.
///
/// Three, not two, because a crossfade hands over at its midpoint: after the
/// swap the *new* core is active while the old one is still fading out, and the
/// track after that wants prefetching in the same window. With three slots there
/// is always exactly one free one.
pub const CORES: usize = 3;

/// Which core is playing, which one is fading, and which one is free.
pub struct Engine {
    cores: [Option<Arc<Mpv>>; CORES],
    /// Index of the audible core.
    active: usize,
    /// Core that is still fading out after a handover, if any.
    outgoing: Option<usize>,
    /// Startup options, kept so a second core is created exactly like the first.
    options: Options,
    /// The `af` chain applied to the idle core. Tracked because the user's EQ
    /// can change *while* a track is prefetched, and the incoming core has to be
    /// brought up to date before it is allowed to play.
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
            cores: [None, None, None],
            active: 0,
            outgoing: None,
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

    /// Create the free core if it does not exist yet, and return it. `None` when
    /// libmpv refuses another core — the caller then simply prefetches nothing.
    pub fn ensure_idle(&mut self) -> Option<Arc<Mpv>> {
        if let Some(existing) = self.idle() {
            return Some(existing);
        }
        let slot = self.idle_slot()?;
        let options = self.options.clone();
        match self.init_core(slot, &options) {
            Ok(api) => Some(api),
            Err(e) => {
                iwaks_log::line(
                    iwaks_log::Level::Warn,
                    &format!("could not create a second playback core ({e})"),
                );
                None
            }
        }
    }

    /// The slot a prefetch should load into: the first that is neither playing
    /// nor fading out. With three slots this always exists.
    pub fn idle_slot(&self) -> Option<usize> {
        (0..CORES).find(|slot| *slot != self.active && Some(*slot) != self.outgoing)
    }

    /// The free core — where a prefetch would go. `None` until it is created.
    pub fn idle(&self) -> Option<Arc<Mpv>> {
        self.idle_slot().and_then(|slot| self.cores[slot].clone())
    }

    /// Is the free core loaded and ready?
    pub fn has_idle(&self) -> bool {
        self.idle().is_some()
    }

    /// Remove the free core and hand it back so the caller can stop it with
    /// libmpv (pump-thread work). Dropping the returned handle releases it.
    pub fn take_idle(&mut self) -> Option<Arc<Mpv>> {
        let slot = self.idle_slot()?;
        self.idle_af = None;
        self.cores[slot].take()
    }

    /// The core still fading out after a handover, if any.
    pub fn outgoing(&self) -> Option<Arc<Mpv>> {
        self.outgoing.and_then(|slot| self.cores[slot].clone())
    }

    /// Index of the core still fading out, if any.
    pub fn outgoing_slot(&self) -> Option<usize> {
        self.outgoing
    }

    /// Mark `slot` as the core that is fading out, so it is not mistaken for the
    /// free one.
    ///
    /// Only valid *after* a handover: before the midpoint the fading-out track
    /// is still the one playing, and claiming its slot here would let
    /// [`Engine::take_outgoing`] evict the core the session is running on.
    pub fn set_outgoing(&mut self, slot: usize) {
        assert_ne!(
            slot, self.active,
            "the playing core is never the outgoing one"
        );
        self.outgoing = Some(slot);
    }

    /// Forget the fading core's role without releasing it (its handle is still
    /// needed while it finishes fading out).
    pub fn clear_outgoing(&mut self) {
        self.outgoing = None;
    }

    /// Release the fading core and hand it back so the caller can stop it.
    ///
    /// Refuses to hand back the core the session is playing on — losing it would
    /// leave the pump with nothing to read state from.
    pub fn take_outgoing(&mut self) -> Option<Arc<Mpv>> {
        let slot = self.outgoing?;
        self.outgoing = None;
        if slot == self.active {
            return None;
        }
        self.cores[slot].take()
    }

    /// Is a fade in flight?
    pub fn is_fading(&self) -> bool {
        self.outgoing.is_some()
    }

    /// Hand the session to `slot` (the crossfade midpoint). The caller has
    /// already made the incoming core audible at the matching level.
    pub fn make_active(&mut self, slot: usize) {
        assert!(slot < CORES, "core slot out of range");
        self.active = slot;
    }

    /// The core that plays.
    ///
    /// Panics when the session never started, which is the same contract the
    /// single-handle version had: every caller runs after `Player::start`
    /// returned `Ok`.
    pub fn active(&self) -> Arc<Mpv> {
        self.cores[self.active].clone().expect("player not started")
    }

    /// Index of the playing slot.
    pub fn active_slot(&self) -> usize {
        self.active
    }

    /// Whether `api` is the core currently playing.
    pub fn is_active(&self, api: &Arc<Mpv>) -> bool {
        self.cores[self.active]
            .as_ref()
            .map(|active| Arc::ptr_eq(active, api))
            .unwrap_or(false)
    }

    /// The core at `slot`, if it exists.
    pub fn core(&self, slot: usize) -> Option<Arc<Mpv>> {
        self.cores.get(slot).cloned().flatten()
    }

    /// Slots that currently hold a core — the pump polls these for events.
    pub fn live_slots(&self) -> Vec<usize> {
        (0..CORES)
            .filter(|slot| self.cores[*slot].is_some())
            .collect()
    }

    /// True when no core has been created (before `Player::start`).
    pub fn is_empty(&self) -> bool {
        self.cores[self.active].is_none()
    }

    /// Is this a session on WASAPI exclusive output?
    ///
    /// Only set at startup (`Options`), so a crossfade has to ask here rather
    /// than take another argument: exclusive mode takes the device for itself
    /// and a second core cannot share it.
    pub fn is_exclusive(&self) -> bool {
        self.options.audio_exclusive
    }

    /// The `af` chain the free core was given, if any.
    pub fn idle_af(&self) -> Option<&str> {
        self.idle_af.as_deref()
    }

    /// Record the `af` chain the free core was given.
    pub fn set_idle_af(&mut self, chain: Option<String>) {
        self.idle_af = chain;
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
    /// DLL — which matters, because CI machines have none (and asking a machine
    /// with no audio endpoint to enumerate devices crashes libmpv, see #23).
    #[test]
    fn an_empty_engine_has_no_active_core() {
        let engine = Engine::empty();
        assert!(engine.is_empty());
        assert_eq!(engine.active_slot(), 0);
        assert_eq!(engine.idle_slot(), Some(1));
        // `Mpv` is neither `Debug` nor `PartialEq`, so presence is asserted
        // with `is_none`/`is_some` rather than `assert_eq!`.
        assert!(engine.idle().is_none());
        assert!(engine.core(0).is_none());
        assert!(engine.core(CORES).is_none(), "out of range reads as None");
        assert!(!engine.is_fading());
        assert!(engine.live_slots().is_empty());
    }

    #[test]
    fn a_fading_core_is_not_offered_as_the_free_one() {
        let mut engine = Engine::empty();
        engine.make_active(2);
        assert_eq!(engine.idle_slot(), Some(0), "lowest free slot");
        engine.set_outgoing(1);
        assert_eq!(
            engine.idle_slot(),
            Some(0),
            "the outgoing slot is not the free one"
        );
        // After the handover the outgoing core becomes free again — and with
        // nothing fading, the free slot is simply the lowest one that is not
        // playing.
        engine.take_outgoing();
        assert!(!engine.is_fading());
        assert_eq!(engine.idle_slot(), Some(0));
    }

    #[test]
    fn the_playing_core_is_never_handed_back() {
        let mut engine = Engine::empty();
        engine.make_active(1);
        // Before a crossfade handover the fading core is still the playing one.
        // Claiming it as outgoing would let `take_outgoing` evict the core the
        // session reads its state from.
        assert!(engine.outgoing_slot().is_none());
        assert!(engine.take_outgoing().is_none(), "nothing is marked yet");
        engine.set_outgoing(0);
        assert_eq!(engine.outgoing_slot(), Some(0));
        engine.make_active(2); // the handover
                               // Nothing comes back — this engine holds no libmpv handles — but the role
                               // is released and the slot is free for the next prefetch.
        engine.take_outgoing();
        assert!(engine.outgoing_slot().is_none(), "the role is consumed");
        assert_eq!(engine.idle_slot(), Some(0), "the slot is free again");
    }

    #[test]
    fn the_free_slot_is_never_the_playing_or_fading_one() {
        let mut engine = Engine::empty();
        for active in 0..CORES {
            engine.make_active(active);
            let free = engine.idle_slot().expect("always a free slot");
            assert_ne!(free, active);
            if let Some(outgoing) = (0..CORES).find(|s| *s != active) {
                engine.set_outgoing(outgoing);
                assert_ne!(engine.idle_slot().expect("still free"), outgoing);
                engine.take_outgoing();
            }
        }
    }

    #[test]
    fn default_is_an_empty_engine() {
        assert!(Engine::default().is_empty());
    }
}
