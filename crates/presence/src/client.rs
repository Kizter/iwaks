//! RPC client state machine — pure event → effects, no I/O.
//!
//! The driver loop (the `Presence` facade) executes [`Effect`]s against a
//! [`Transport`] and feeds outcomes back in as [`Event`]s, so every
//! transition is unit-testable without a real Discord client.

use std::time::Duration;

use crate::protocol::{
    encode_frame, handshake_payload, nonce, set_activity_json, Activity, Frame, OP_CLOSE, OP_FRAME,
    OP_HANDSHAKE, OP_PING, OP_PONG,
};

/// Reconnect delay after the first failure.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Backoff cap — never hammer Discord more often than this.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Connection status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Disconnected,
    Handshaking,
    Ready,
}

/// An event the driver observes and feeds back to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The transport successfully opened a pipe.
    Opened,
    /// The transport could not open any pipe (Discord not running).
    ConnectFailed,
    /// A frame arrived from the pipe.
    Frame(Frame),
    /// The pipe broke / was closed by the peer.
    Disconnected,
    /// The [`Effect::Wait`] duration has elapsed.
    Timer,
}

/// A side effect the driver must carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Open the pipe (tries `discord-ipc-0..9`).
    OpenPipe,
    /// Write these frame bytes to the pipe.
    Write(Vec<u8>),
    /// Close the pipe (no-op when not open).
    ClosePipe,
    /// Wait this long, then feed back [`Event::Timer`].
    Wait(Duration),
}

/// Byte transport the driver talks to; implemented by the Windows named-pipe
/// transport ([`crate::pipe`]) and swapped for a scripted fake in tests.
pub trait Transport {
    /// Try to open the first available Discord pipe. `Err` when none exists
    /// (Discord not running) — the client treats that as
    /// [`Event::ConnectFailed`].
    fn connect(&mut self) -> Result<(), String>;
    /// Write all bytes to the pipe.
    fn write(&mut self, bytes: &[u8]) -> Result<(), String>;
    /// Non-blocking read of one complete frame: `Ok(Some)` when a frame is
    /// available, `Ok(None)` when there is no data yet, `Err` on a broken
    /// pipe. The caller must never block inside this method.
    fn try_read_frame(&mut self) -> Result<Option<Frame>, String>;
    /// Close the pipe (no-op when not open).
    fn close(&mut self);
    /// Whether a pipe is currently open (drives the poll loop).
    fn is_open(&self) -> bool;
}

/// The RPC client at the heart of the presence loop. Pure — no I/O inside.
pub struct Client {
    app_id: String,
    pid: u32,
    seq: u64,
    phase: Phase,
    /// The activity the user last asked for; re-sent after every reconnect
    /// until it changes.
    pending: Option<Activity>,
    /// The activity last written to the pipe (for dedupe).
    sent: Option<Activity>,
    backoff: Duration,
}

impl Client {
    pub fn new(app_id: &str, pid: u32) -> Self {
        Self {
            app_id: app_id.to_string(),
            pid,
            seq: 0,
            phase: Phase::Disconnected,
            pending: None,
            sent: None,
            backoff: INITIAL_BACKOFF,
        }
    }

    /// Current connection phase (used by the driver for diagnostics).
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Ask to display `activity` (`None` clears the presence). Returns the
    /// effects to run now; when not connected yet the activity is stored and
    /// sent right after the next successful handshake.
    pub fn set_activity(&mut self, activity: Option<Activity>) -> Vec<Effect> {
        self.pending = activity;
        if self.phase == Phase::Ready && self.pending != self.sent {
            self.send_pending()
        } else {
            Vec::new()
        }
    }

    /// Feed an observed event; returns the effects to run.
    pub fn on_event(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Opened => {
                if self.phase == Phase::Disconnected {
                    self.phase = Phase::Handshaking;
                    vec![Effect::Write(encode_frame(
                        OP_HANDSHAKE,
                        &handshake_payload(&self.app_id),
                    ))]
                } else {
                    Vec::new()
                }
            }
            Event::Frame(frame) => self.on_frame(frame),
            Event::ConnectFailed | Event::Disconnected => self.retry_later(),
            Event::Timer => {
                if self.phase == Phase::Disconnected {
                    vec![Effect::OpenPipe]
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn on_frame(&mut self, frame: Frame) -> Vec<Effect> {
        match self.phase {
            Phase::Handshaking => {
                if frame.opcode == OP_FRAME
                    && (payload_evt(&frame.payload).as_deref() == Some("READY")
                        || payload_cmd(&frame.payload).as_deref() == Some("READY"))
                {
                    self.phase = Phase::Ready;
                    self.backoff = INITIAL_BACKOFF;
                    if self.pending != self.sent {
                        self.send_pending()
                    } else {
                        Vec::new()
                    }
                } else {
                    // Rejected or unexpected during the handshake — close and
                    // retry later.
                    self.retry_later()
                }
            }
            Phase::Ready => match frame.opcode {
                // Heartbeat: echo the payload back so Discord keeps the
                // connection alive.
                OP_PING => vec![Effect::Write(encode_frame(OP_PONG, &frame.payload))],
                OP_CLOSE => self.retry_later(),
                _ => Vec::new(),
            },
            Phase::Disconnected => Vec::new(),
        }
    }

    /// Send the pending activity (updating `sent` so dedupe works).
    fn send_pending(&mut self) -> Vec<Effect> {
        let nonce = nonce(self.pid, self.seq);
        self.seq += 1;
        let payload = set_activity_json(self.pid, &nonce, self.pending.as_ref());
        self.sent = self.pending.clone();
        vec![Effect::Write(encode_frame(OP_FRAME, &payload))]
    }

    /// Drop the connection (if any) and wait `backoff` before retrying.
    fn retry_later(&mut self) -> Vec<Effect> {
        self.phase = Phase::Disconnected;
        self.sent = None;
        let delay = self.backoff;
        self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
        vec![Effect::ClosePipe, Effect::Wait(delay)]
    }
}

/// Extract the `cmd` field of a frame payload, if present.
fn payload_cmd(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    value.get("cmd")?.as_str().map(str::to_string)
}

/// Extract the `evt` field of a frame payload. Server → client events on the
/// wire are `{"cmd":"DISPATCH","evt":"<name>",...}`, so the handshake's
/// `READY` arrives as `evt:"READY"` — matching the `cmd` field alone would
/// reject every real READY frame.
fn payload_evt(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    value.get("evt")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Activity, Frame};

    /// Extract the `Write` effects only, for assertion convenience.
    fn writes(effects: &[Effect]) -> Vec<&[u8]> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Write(bytes) => Some(bytes.as_slice()),
                _ => None,
            })
            .collect()
    }

    fn opening_handshake(bytes: &[u8]) -> bool {
        let f = crate::protocol::decode_frame(bytes).unwrap();
        f.opcode == crate::protocol::OP_HANDSHAKE
    }

    fn is_set_activity(bytes: &[u8]) -> bool {
        let f = crate::protocol::decode_frame(bytes).unwrap();
        f.opcode == OP_FRAME && f.payload.contains("SET_ACTIVITY")
    }

    /// A READY frame as Discord actually sends it after a successful
    /// handshake: `cmd` is `"DISPATCH"` and the event name lives in `evt`.
    /// (The old test payload `{"cmd":"READY"}` codified a wrong assumption
    /// that rejected every real READY — kept in `discord_readiness_checks_evt_field`
    /// only as the regression guard.)
    fn ready_frame() -> Frame {
        Frame {
            opcode: OP_FRAME,
            payload: r#"{"cmd":"DISPATCH","evt":"READY","data":{"v":1}}"#.to_string(),
        }
    }

    fn ping_frame(payload: &str) -> Frame {
        Frame {
            opcode: crate::protocol::OP_PING,
            payload: payload.to_string(),
        }
    }

    #[test]
    fn phase_progresses_through_connect_to_ready() {
        let mut c = Client::new("1553", 42);
        assert_eq!(c.phase(), Phase::Disconnected);
        c.on_event(Event::Opened);
        assert_eq!(c.phase(), Phase::Handshaking);
        c.on_event(Event::Frame(ready_frame()));
        assert_eq!(c.phase(), Phase::Ready);
    }

    #[test]
    fn discord_readiness_checks_evt_field_with_real_wire_payload() {
        // Regression: the real READY frame is `cmd:"DISPATCH"` + `evt:"READY"`.
        // Checking `cmd == "READY"` (as the old code did) rejected this frame
        // and put the driver into an endless reconnect loop.
        let mut c = Client::new("1553", 42);
        c.on_event(Event::Opened);
        assert_eq!(c.phase(), Phase::Handshaking);
        let real = Frame {
            opcode: OP_FRAME,
            payload:
                r#"{"cmd":"DISPATCH","data":{"v":1,"user":{"id":"782477001000747018","username":"kizt"},"config":{"api_endpoint":"//discord.com/api"}},"evt":"READY","nonce":null}"#
                    .to_string(),
        };
        c.on_event(Event::Frame(real));
        assert_eq!(c.phase(), Phase::Ready);
    }

    #[test]
    fn handshake_needs_no_activity() {
        // Even without any pending activity the handshake always goes out.
        let mut c = Client::new("1553", 42);
        c.on_event(Event::Opened);
        let effects = c.on_event(Event::Frame(ready_frame()));
        assert!(writes(&effects).is_empty());
    }

    #[test]
    fn opened_pipe_sends_handshake_frame() {
        let mut c = Client::new("1553", 42);
        let effects = c.on_event(Event::Opened);
        let ws = writes(&effects);
        assert_eq!(ws.len(), 1);
        assert!(opening_handshake(ws[0]), "expected handshake frame");
    }

    #[test]
    fn ready_resends_pending_activity() {
        let mut c = Client::new("1553", 42);
        c.set_activity(Some(Activity::listening().details("Lagu A")));
        c.on_event(Event::Opened);
        let effects = c.on_event(Event::Frame(ready_frame()));
        let ws = writes(&effects);
        assert_eq!(ws.len(), 1);
        assert!(is_set_activity(ws[0]));
    }

    #[test]
    fn set_activity_while_ready_sends_once_and_dedupes() {
        let mut c = Client::new("1553", 42);
        c.on_event(Event::Opened);
        c.on_event(Event::Frame(ready_frame()));

        let first = c.set_activity(Some(Activity::listening().details("Lagu A")));
        assert_eq!(writes(&first).len(), 1);

        // Same activity → no resend.
        let dup = c.set_activity(Some(Activity::listening().details("Lagu A")));
        assert!(writes(&dup).is_empty(), "same activity must not resend");

        // New activity → send.
        let changed = c.set_activity(Some(Activity::listening().details("Lagu B")));
        assert_eq!(writes(&changed).len(), 1);
    }

    #[test]
    fn ping_answers_with_pong_echoing_payload() {
        let mut c = Client::new("1553", 42);
        c.on_event(Event::Opened);
        c.on_event(Event::Frame(ready_frame()));

        let effects = c.on_event(Event::Frame(ping_frame("hb-1")));
        let ws = writes(&effects);
        assert_eq!(ws.len(), 1);
        let f = crate::protocol::decode_frame(ws[0]).unwrap();
        assert_eq!(f.opcode, OP_PONG);
        assert_eq!(f.payload, "hb-1");
    }

    #[test]
    fn disconnect_schedules_retry_and_reconnects_on_timer() {
        let mut c = Client::new("1553", 42);
        c.on_event(Event::Opened);
        c.on_event(Event::Frame(ready_frame()));

        let down = c.on_event(Event::Disconnected);
        assert!(down.iter().any(|e| matches!(e, Effect::ClosePipe)));
        let wait = down
            .iter()
            .find_map(|e| match e {
                Effect::Wait(d) => Some(*d),
                _ => None,
            })
            .expect("disconnect schedules a retry wait");
        assert!(!wait.is_zero());

        let retry = c.on_event(Event::Timer);
        assert!(
            retry.iter().any(|e| matches!(e, Effect::OpenPipe)),
            "timer after disconnect must reopen the pipe"
        );
    }

    #[test]
    fn connect_failure_backs_off_to_a_cap() {
        let mut c = Client::new("1553", 42);
        let mut seen = Vec::new();
        for _ in 0..10 {
            let effects = c.on_event(Event::ConnectFailed);
            let wait = effects
                .iter()
                .find_map(|e| match e {
                    Effect::Wait(d) => Some(*d),
                    _ => None,
                })
                .expect("connect failure schedules a wait");
            seen.push(wait);
        }
        // Backoff grows monotonically up to the 30 s cap.
        assert!(seen.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(*seen.last().unwrap(), Duration::from_secs(30));
    }

    #[test]
    fn ready_resets_backoff() {
        let mut c = Client::new("1553", 42);
        for _ in 0..4 {
            c.on_event(Event::ConnectFailed);
        }
        c.on_event(Event::Opened);
        c.on_event(Event::Frame(ready_frame()));
        // Next failure → backoff back to the initial 1 s.
        let effects = c.on_event(Event::ConnectFailed);
        let wait = effects
            .iter()
            .find_map(|e| match e {
                Effect::Wait(d) => Some(*d),
                _ => None,
            })
            .unwrap();
        assert_eq!(wait, Duration::from_secs(1));
    }

    #[test]
    fn ready_activity_is_resent_after_reconnect() {
        let mut c = Client::new("1553", 42);
        c.on_event(Event::Opened);
        c.on_event(Event::Frame(ready_frame()));
        c.set_activity(Some(Activity::listening().details("Lagu A")));

        // Drop → timer → reopen → READY again.
        c.on_event(Event::Disconnected);
        c.on_event(Event::Timer);
        c.on_event(Event::Opened);
        let effects = c.on_event(Event::Frame(ready_frame()));
        let ws = writes(&effects);
        assert_eq!(ws.len(), 1, "last activity must be resent after reconnect");
        assert!(is_set_activity(ws[0]));
    }

    #[test]
    fn close_frame_drops_connection_and_retries() {
        let mut c = Client::new("1553", 42);
        c.on_event(Event::Opened);
        c.on_event(Event::Frame(ready_frame()));

        let close = Frame {
            opcode: OP_CLOSE,
            payload: r#"{"code":4000}"#.to_string(),
        };
        let effects = c.on_event(Event::Frame(close));
        assert!(effects.iter().any(|e| matches!(e, Effect::ClosePipe)));
        assert!(effects.iter().any(|e| matches!(e, Effect::Wait(_))));
    }

    #[test]
    fn cleared_activity_sends_empty_activity_payload() {
        let mut c = Client::new("1553", 42);
        c.on_event(Event::Opened);
        c.on_event(Event::Frame(ready_frame()));
        c.set_activity(Some(Activity::listening().details("Lagu A")));

        let clear = c.set_activity(None);
        let ws = writes(&clear);
        assert_eq!(ws.len(), 1);
        let f = crate::protocol::decode_frame(ws[0]).unwrap();
        assert!(f.payload.contains("\"activity\":{}"));
    }

    #[test]
    fn non_ready_frame_during_handshake_is_rejected() {
        let mut c = Client::new("1553", 42);
        c.on_event(Event::Opened);
        let bad = Frame {
            opcode: OP_FRAME,
            payload: r#"{"cmd":"DISPATCH"}"#.to_string(),
        };
        let effects = c.on_event(Event::Frame(bad));
        assert!(effects.iter().any(|e| matches!(e, Effect::ClosePipe)));
        assert!(
            effects.iter().any(|e| matches!(e, Effect::Wait(_))),
            "must schedule a retry after a rejected handshake"
        );
    }
}
