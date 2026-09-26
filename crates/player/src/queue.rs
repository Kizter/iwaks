//! Playback queue & repeat logic — pure, no mpv, fully unit-tested.
//!
//! Values are immutable: every operation returns a *new* `Queue`; the actual
//! queue slot inside `Player` is a `Mutex` that swaps in fresh values.
//!
//! With shuffle enabled the queue keeps a permutation (`order`) of the track
//! indices: `index` addresses a *position* inside `order`, and `current()`
//! translates back to the real track index. Next/prev/after_end just walk
//! `order`, so repeat semantics stay identical to the non-shuffled case.

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
    /// Position inside `order` (not the track index once shuffled).
    pub index: Option<usize>,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    order: Vec<usize>,
}

impl Queue {
    /// Empty queue (nothing selected).
    pub fn new(len: usize) -> Self {
        Self {
            len,
            index: None,
            repeat: RepeatMode::Off,
            shuffle: false,
            order: (0..len).collect(),
        }
    }

    /// Start playing track `index` (clamped to the list; empty list stays
    /// empty). With shuffle the track is located inside `order`.
    pub fn start(&self, index: usize) -> Self {
        if self.len == 0 {
            return self.clone();
        }
        let track = index.min(self.len - 1);
        let pos = self.order.iter().position(|&t| t == track).unwrap_or(track);
        Self {
            index: Some(pos),
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
        self.index.map(|pos| self.order[pos])
    }

    /// User-initiated "next": always advances; wraps only in `All` mode,
    /// otherwise the last track is a stop.
    pub fn user_next(&self) -> Option<usize> {
        match self.index {
            None => None,
            Some(pos) if pos + 1 < self.len => Some(self.order[pos + 1]),
            Some(_) => match self.repeat {
                RepeatMode::All => Some(self.order[0]),
                _ => None,
            },
        }
    }

    /// User-initiated "prev": always wraps to the end.
    pub fn user_prev(&self) -> Option<usize> {
        match self.index {
            None => None,
            Some(0) => Some(self.order[self.len - 1]),
            Some(pos) => Some(self.order[pos - 1]),
        }
    }

    /// What to play after the current file ends naturally.
    pub fn after_end(&self) -> Option<usize> {
        match (self.index, self.repeat) {
            (None, _) => None,
            (Some(pos), RepeatMode::One) => Some(self.order[pos]),
            (Some(pos), _) if pos + 1 < self.len => Some(self.order[pos + 1]),
            (Some(_), RepeatMode::All) => Some(self.order[0]),
            _ => None,
        }
    }

    // ---- shuffle ----

    /// Enable shuffle with a deterministic permutation of the whole list.
    /// The currently playing track is pinned to the front of the shuffled
    /// order, so "next" walks the entire list; then it stops (repeat off) or
    /// wraps (repeat all) — never a silent mid-list skip.
    pub fn with_shuffle(&self, seed: u64) -> Self {
        let current_track = self.current();
        let mut order = permute(seed, self.len);
        if let Some(t) = current_track {
            if let Some(pos) = order.iter().position(|&x| x == t) {
                order.swap(0, pos);
            }
        }
        Self {
            shuffle: true,
            index: current_track.map(|_| 0),
            order,
            ..self.clone()
        }
    }

    /// Re-shuffle the list; the current track never moves. When shuffle is
    /// off, this enables it first (same as `with_shuffle`).
    pub fn reshuffle(&self, seed: u64) -> Self {
        let current_track = self.current();
        if !self.shuffle {
            return self.with_shuffle(seed);
        }
        let mut q = Self {
            order: reshuffle_rest(seed, &self.order, self.index),
            ..self.clone()
        };
        q.relocate_to(current_track);
        q
    }

    /// Turn shuffle off: order returns to identity and the current *track*
    /// stays selected (its position becomes its index).
    pub fn with_shuffle_off(&self) -> Self {
        let mut q = Self {
            shuffle: false,
            order: (0..self.len).collect(),
            ..self.clone()
        };
        if let Some(pos) = self.index {
            let track = self.order[pos];
            q.index = Some(track.min(self.len - 1));
        }
        q
    }

    /// Re-point `index` at the position of `track` inside the current order.
    fn relocate_to(&mut self, track: Option<usize>) {
        self.index = track.and_then(|t| self.order.iter().position(|&x| x == t));
    }
}

/// Deterministic Fisher–Yates shuffle of `0..n` from a SplitMix64 generator.
fn permute(seed: u64, n: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    let mut rng = seed;
    let mut i = n;
    while i > 1 {
        i -= 1;
        let j = (splitmix64(&mut rng) % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

/// Shuffle every slot of `order` except the one pinned at `pin` (a position).
fn reshuffle_rest(seed: u64, order: &[usize], pin: Option<usize>) -> Vec<usize> {
    let len = order.len();
    if pin.is_none() {
        return permute(seed, len);
    }
    let mut rest: Vec<usize> = order
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != pin)
        .map(|(_, &t)| t)
        .collect();
    let mut rng = seed;
    let mut i = rest.len();
    while i > 1 {
        i -= 1;
        let j = (splitmix64(&mut rng) % (i as u64 + 1)) as usize;
        rest.swap(i, j);
    }
    let mut out = vec![0usize; len];
    let pin = pin.expect("checked above");
    out[pin] = order[pin];
    let mut it = rest.into_iter();
    for (slot, cell) in out.iter_mut().enumerate() {
        if slot == pin {
            continue;
        }
        *cell = it.next().unwrap_or(0);
    }
    out
}

/// SplitMix64: tiny deterministic PRNG (no external dependency).
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
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
        assert!(!q.shuffle);
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

    // ---------- shuffle ----------

    #[test]
    fn shuffle_is_a_permutation_and_pins_current() {
        let q = Queue::new(5).start(2).with_shuffle(42);
        assert!(q.shuffle);
        assert_eq!(q.current(), Some(2), "current track never changes");
        assert_eq!(q.index, Some(0), "current track pinned to the front");
        let mut sorted = q.order.clone();
        sorted.sort();
        assert_eq!(sorted, (0..5).collect::<Vec<_>>(), "order is a permutation");
    }

    #[test]
    fn shuffle_is_deterministic_for_seed() {
        let a = Queue::new(8).with_shuffle(7);
        let b = Queue::new(8).with_shuffle(7);
        assert_eq!(a.order, b.order);
    }

    #[test]
    fn shuffle_next_walks_every_track_before_stopping() {
        let q = Queue::new(6).start(0).with_shuffle(99);
        // Current track sits at the front, so walking forward covers all 6.
        let mut walked = vec![q.current().unwrap()];
        let mut pos = q.index.expect("selected");
        while pos + 1 < 6 {
            pos += 1;
            walked.push(q.order[pos]);
        }
        assert_eq!(walked.len(), 6, "all tracks visited exactly once");
        for (i, t) in walked.iter().enumerate() {
            assert!(!walked[..i].contains(t), "no track repeats");
        }

        // user_next returns exactly that same sequence, None at the end.
        let mut cursor = q.clone();
        let mut nexts: Vec<usize> = Vec::new();
        while let Some(n) = cursor.user_next() {
            nexts.push(n);
            cursor.index = Some(cursor.index.unwrap() + 1);
        }
        assert_eq!(
            &nexts[..],
            &walked[1..],
            "user_next matches the walked order"
        );

        assert_eq!(
            Queue::new(6).with_shuffle(5).user_next(),
            None,
            "unselected stays unselected even when shuffled"
        );
    }

    #[test]
    fn reshuffle_keeps_current_track_with_shuffle_on() {
        let q = Queue::new(6).start(3).with_shuffle(11);
        let after = q.reshuffle(22);
        assert!(after.shuffle);
        assert_eq!(after.current(), Some(3), "current pinned through reshuffle");
        assert_eq!(after.index, q.index, "current keeps its position");
        assert_ne!(after.order, q.order, "the rest is reordered");
    }

    #[test]
    fn reshuffle_with_shuffle_off_enables_shuffle() {
        let q = Queue::new(4).start(1);
        let after = q.reshuffle(5);
        assert!(after.shuffle);
        assert_eq!(after.current(), Some(1));
    }

    #[test]
    fn disabling_shuffle_restores_identity_and_current() {
        let q = Queue::new(5).start(2).with_shuffle(42);
        let off = q.with_shuffle_off();
        assert!(!off.shuffle);
        assert_eq!(off.current(), Some(2), "same track stays selected");
        assert_eq!(off.order, (0..5).collect::<Vec<_>>());
        assert_eq!(off.user_next(), Some(3), "linear order after disable");
    }

    #[test]
    fn shuffle_repeat_all_wraps_to_shuffled_start() {
        let q = Queue::new(4)
            .start(0)
            .with_shuffle(9)
            .with_repeat(RepeatMode::All);
        let first = q.order[0];
        // At the last shuffled position, next wraps to the shuffled front.
        let at_last = Queue {
            index: Some(3),
            ..q.clone()
        };
        assert_eq!(at_last.after_end(), Some(first), "after_end wraps");
        assert_eq!(at_last.user_next(), Some(first), "user_next wraps");
    }
}
