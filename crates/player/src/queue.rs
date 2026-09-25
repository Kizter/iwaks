//! Playback queue & repeat logic — pure, no mpv, fully unit-tested.
//!
//! Values are immutable: every operation returns a *new* `Queue`; the actual
//! queue slot inside `Player` is a `Mutex` that swaps in fresh values.

use serde::{Deserialize, Serialize};

/// Repeat behavior after the current track ends or at list boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    /// Stop when the last track ends.
    Off,
    /// Wrap around to the first track.
    All,
    /// Loop the current track.
    One,
}

/// Immutable snapshot of the playback queue position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub len: usize,
    pub index: Option<usize>,
    pub repeat: RepeatMode,
}

impl Queue {
    /// Empty queue (nothing selected).
    pub fn new(len: usize) -> Self {
        Self {
            len,
            index: None,
            repeat: RepeatMode::Off,
        }
    }

    /// Start playing `index` (clamped to the list; empty list stays empty).
    pub fn start(&self, index: usize) -> Self {
        if self.len == 0 {
            return self.clone();
        }
        Self {
            index: Some(index.min(self.len - 1)),
            ..self.clone()
        }
    }

    pub fn with_repeat(&self, repeat: RepeatMode) -> Self {
        Self {
            repeat,
            ..self.clone()
        }
    }

    pub fn current(&self) -> Option<usize> {
        self.index
    }

    /// User-initiated "next": always advances; wraps only in `All` mode,
    /// otherwise the last track is a stop.
    pub fn user_next(&self) -> Option<usize> {
        match self.index {
            None => None,
            Some(i) if i + 1 < self.len => Some(i + 1),
            Some(_) => match self.repeat {
                RepeatMode::All => Some(0),
                _ => None,
            },
        }
    }

    /// User-initiated "prev": always wraps to the end.
    pub fn user_prev(&self) -> Option<usize> {
        match self.index {
            None => None,
            Some(0) => Some(self.len - 1),
            Some(i) => Some(i - 1),
        }
    }

    /// What to play after the current file ends naturally.
    pub fn after_end(&self) -> Option<usize> {
        match (self.index, self.repeat) {
            (None, _) => None,
            (Some(i), RepeatMode::One) => Some(i),
            (Some(i), _) if i + 1 < self.len => Some(i + 1),
            (Some(_), RepeatMode::All) => Some(0),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_queue_is_empty_and_off() {
        let q = Queue::new(0);
        assert_eq!(q.len, 0);
        assert_eq!(q.index, None);
        assert_eq!(q.repeat, RepeatMode::Off);
    }

    #[test]
    fn start_clamps_out_of_range_index() {
        let q = Queue::new(3).start(9);
        assert_eq!(q.current(), Some(2));
    }

    #[test]
    fn start_on_empty_stays_unselected() {
        assert_eq!(Queue::new(0).start(2).current(), None);
    }

    #[test]
    fn user_next_advances_in_middle() {
        let q = Queue::new(3).start(1);
        assert_eq!(q.user_next(), Some(2));
    }

    #[test]
    fn user_next_at_end_stops_unless_all() {
        let stop = Queue::new(3).start(2);
        assert_eq!(stop.user_next(), None, "repeat off => stop");

        let wrap = stop.with_repeat(RepeatMode::All);
        assert_eq!(wrap.user_next(), Some(0), "repeat all => wrap");
    }

    #[test]
    fn user_prev_wraps_to_last() {
        let q = Queue::new(3).start(0);
        assert_eq!(q.user_prev(), Some(2));
        assert_eq!(Queue::new(3).start(1).user_prev(), Some(0));
    }

    #[test]
    fn after_end_advances_mid_list() {
        let q = Queue::new(3).start(0);
        assert_eq!(q.after_end(), Some(1));
    }

    #[test]
    fn after_end_repeat_one_loops_same_track() {
        let q = Queue::new(3).start(1).with_repeat(RepeatMode::One);
        assert_eq!(q.after_end(), Some(1));
    }

    #[test]
    fn after_end_wraps_or_stops_on_last() {
        let last = Queue::new(3).start(2);
        assert_eq!(last.after_end(), None, "off => stop");
        assert_eq!(
            last.with_repeat(RepeatMode::All).after_end(),
            Some(0),
            "all => wrap"
        );
    }

    #[test]
    fn unselected_queue_has_no_moves() {
        let q = Queue::new(3);
        assert_eq!(q.user_next(), None);
        assert_eq!(q.user_prev(), None);
        assert_eq!(q.after_end(), None);
    }

    #[test]
    fn with_repeat_keeps_position() {
        let q = Queue::new(2).start(1).with_repeat(RepeatMode::One);
        assert_eq!(q.current(), Some(1));
        assert_eq!(q.repeat, RepeatMode::One);
    }
}
