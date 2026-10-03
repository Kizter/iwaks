//! Discord Rich Presence for Iwaks — a small custom RPC client over the
//! Discord local named pipe (no third-party library, no C build).
//!
//! Layout follows the repo pattern of "pure logic + thin platform glue":
//! - [`protocol`] — pure: RPC frame encode/decode + the `SET_ACTIVITY`
//!   payload builder. Fully unit-tested.
//! - `client` — state machine (connect → handshake → ready → update) over a
//!   `Transport` trait; reconnection with backoff. Tested with a fake
//!   transport.
//! - `pipe` — thin `Transport` implementation over the Windows named pipe
//!   (`\\.\pipe\discord-ipc-0..9`).
//! - `register` — writes the `discord-<app_id>://` registry protocol key so
//!   Discord detects Iwaks as an app (required for presence + overlay).
//! - `presence` — the facade tying the sink events to the client.

pub mod client;
pub mod pipe;
pub mod presence;
pub mod protocol;
pub mod register;

pub use presence::{start, NowPlaying};
