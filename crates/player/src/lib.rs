//! iwaks-player: libmpv playback wrapper + pure queue/repeat logic.
//!
//! libmpv is loaded at **runtime** via `libloading` (Windows: ship
//! `libmpv-2.dll` next to the executable, or set `IWAKS_LIBMPV`). Builds and
//! unit tests compile without the DLL — playback tests skip when absent.

pub mod ffi;
pub mod player;
pub mod queue;

pub use player::{Options, Player, PlayerState, ReplayGainMode};
pub use queue::RepeatMode;
