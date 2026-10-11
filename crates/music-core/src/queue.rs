//! The playback queue and its mode.
//!
//! This module is pure data plus pure operations: navigation, reordering and
//! deletion live here so the app layer only ever decides *when* to call them.
//!
//! Three separate ideas are easy to confuse, so they are named apart:
//!
//! * `items` — the queue itself, in order.
//! * `pending_next` — tracks queued to play *before* the queue continues, which
//!   do not disturb the order of `items`.
//! * `played_history` — what was heard before the current track, most recent
//!   last. Shuffle walks back through it instead of rolling again.

use crate::id::SongId;
use crate::song::Song;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashSet, VecDeque};
use std::fmt;

/// How the queue advances when a track ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    #[default]
    Sequential,
    ListLoop,
    SingleTrackLoop,
    Shuffle,
}

impl PlaybackMode {
    /// Every mode, in the order the `playback_mode` key cycles through them.
    pub const CYCLE: [PlaybackMode; 4] = [
        PlaybackMode::Sequential,
        PlaybackMode::ListLoop,
        PlaybackMode::SingleTrackLoop,
        PlaybackMode::Shuffle,
    ];

    /// The next mode in cycle order; wraps back to `Sequential`.
    pub fn cycle(self) -> Self {
        self.step(1)
    }

    /// The mode `delta` steps away in cycle order, wrapping round.
    ///
    /// The order is the one the `playback_mode` key cycles through, so stepping
    /// round it is defined here rather than at each of its users.
    pub fn step(self, delta: isize) -> Self {
        let index = Self::CYCLE
            .iter()
            .position(|m| *m == self)
            .expect("CYCLE enumerates every variant") as isize;
        Self::CYCLE[(index + delta).rem_euclid(Self::CYCLE.len() as isize) as usize]
    }

    pub fn label(self) -> &'static str {
        match self {
            PlaybackMode::Sequential => "Sequential",
            PlaybackMode::ListLoop => "List Loop",
            PlaybackMode::SingleTrackLoop => "Single Track Loop",
            PlaybackMode::Shuffle => "Shuffle",
        }
    }
}

impl fmt::Display for PlaybackMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Queue {
    /// The queue itself, in the order the user arranged it.
    #[serde(default)]
    pub items: Vec<Song>,
    /// Where the queue has got to, as a position in `items`.
    ///
    /// While a "play next" track is playing this still points at the track the
    /// list will resume from, which is what keeps the inserted track from
    /// disturbing the order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_index: Option<usize>,
    /// The mode is part of what is written down, so a queue read back keeps
    /// looping or shuffling the way it was left (§29). An older file without
    /// one gets the default rather than failing to parse.
    #[serde(default)]
    pub playback_mode: PlaybackMode,
    /// Tracks queued by "play next", consumed FIFO before the main list.
    #[serde(default)]
    pub pending_next: VecDeque<Song>,
    /// What was heard before the track playing now, most recent last.
    ///
    /// Distinct from the HISTORY page's recently-played list: this is the trail
    /// that Shuffle's "previous" walks back through.
    #[serde(default)]
    pub played_history: Vec<Song>,
    /// The track playing right now.
    ///
    /// Usually the one at `current_index`. A track queued by "play next" is not
    /// in `items` at all, so it can only be held here — which is also what lets
    /// the queue view mark the right row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playing: Option<Song>,
    /// Where shuffle is in its sequence. Not part of the queue's identity.
    #[serde(skip)]
    rng: Rng,
}

/// Equality ignores shuffle's position in its sequence, which is not part of
/// what the queue *is* — two queues holding the same tracks in the same order
/// are the same queue however the next random pick would differ.
impl PartialEq for Queue {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
            && self.current_index == other.current_index
            && self.playback_mode == other.playback_mode
            && self.pending_next == other.pending_next
            && self.played_history == other.played_history
            && self.playing == other.playing
    }
}

impl Eq for Queue {}

impl Queue {
    pub fn new(playback_mode: PlaybackMode) -> Self {
        Self {
            playback_mode,
            ..Self::default()
        }
    }

    /// A queue whose shuffle picks are reproducible, for tests and for anyone
    /// who needs the same order twice.
    pub fn seeded(playback_mode: PlaybackMode, seed: u64) -> Self {
        Self {
            playback_mode,
            rng: Rng::from_seed(seed),
            ..Self::default()
        }
    }

    /// Replaces the queue contents with `items`, starting over.
    ///
    /// Everything chosen for the previous contents goes with them: the
    /// position, the trail, whatever was queued to play next.
    pub fn replace_items(&mut self, items: Vec<Song>) {
        self.items = items;
        self.current_index = None;
        self.played_history.clear();
        self.pending_next.clear();
        self.playing = None;
    }

    pub fn current(&self) -> Option<&Song> {
        self.playing.as_ref()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Queues a track to play next, ahead of the remaining main list.
    pub fn push_next(&mut self, song: Song) {
        self.pending_next.push_back(song);
    }

    /// Takes the next "play next" entry, if any.
    pub fn pop_next(&mut self) -> Option<Song> {
        self.pending_next.pop_front()
    }

    /// Starts the track at `index`.
    pub fn select(&mut self, index: usize) -> Option<Song> {
        let song = self.items.get(index)?.clone();
        self.depart(&song);
        self.current_index = Some(index);
        self.playing = Some(song.clone());
        Some(song)
    }

    /// Moves to whatever comes next, and returns it.
    ///
    /// Returns `None` at the end of a queue that does not loop, which is where
    /// playback stops.
    pub fn advance(&mut self) -> Option<Song> {
        // A track queued with "play next" goes first, and leaves the list be.
        if let Some(song) = self.take_pending() {
            return Some(song);
        }

        let index = match self.current_index {
            Some(current) => self.step_forward(current)?,
            None if self.items.is_empty() => return None,
            // Nothing has played yet: the queue starts at its first track.
            None => 0,
        };
        self.select(index)
    }

    /// Moves back one track, and returns it.
    ///
    /// Under shuffle this replays what was heard rather than drawing again;
    /// otherwise it steps back through the queue's own order.
    pub fn previous(&mut self) -> Option<Song> {
        if self.playback_mode == PlaybackMode::Shuffle {
            let song = self.played_history.pop()?;
            // It may have come from "play next" and never have been in the
            // list; then it simply plays again without a row to point at.
            self.current_index = self
                .items
                .iter()
                .position(|candidate| candidate.id == song.id)
                .or(self.current_index);
            self.playing = Some(song.clone());
            return Some(song);
        }

        let index = self.step_back(self.current_index?)?;
        // The track being returned to is no longer behind the current one.
        if self.played_history.last().map(|song| song.id)
            == self.items.get(index).map(|song| song.id)
        {
            self.played_history.pop();
        }
        let song = self.items.get(index)?.clone();
        self.current_index = Some(index);
        self.playing = Some(song.clone());
        Some(song)
    }

    /// Removes the track at `index` from the queue.
    ///
    /// A track already playing keeps playing — it is in the player, not in the
    /// queue — but the position moves on so the rest of the queue still makes
    /// sense.
    pub fn remove(&mut self, index: usize) -> Option<Song> {
        if index >= self.items.len() {
            return None;
        }
        let removed = self.items.remove(index);

        if let Some(current) = self.current_index {
            self.current_index = match current.cmp(&index) {
                // Anything before the position shifts it down one place.
                Ordering::Greater => Some(current - 1),
                // The position itself was removed; whatever followed takes it.
                Ordering::Equal => (index < self.items.len()).then_some(index),
                Ordering::Less => Some(current),
            };
        }

        Some(removed)
    }

    /// Moves the track at `from` so that it sits at `to`.
    pub fn move_to(&mut self, from: usize, to: usize) -> bool {
        if from >= self.items.len() || to >= self.items.len() || from == to {
            return false;
        }
        let song = self.items.remove(from);
        self.items.insert(to, song);

        // The position follows the track it was pointing at.
        if let Some(current) = self.current_index {
            self.current_index = Some(shift(current, from, to));
        }
        true
    }

    /// Moves the track at `index` one place earlier.
    pub fn move_up(&mut self, index: usize) -> bool {
        index > 0 && self.move_to(index, index - 1)
    }

    /// Moves the track at `index` one place later.
    pub fn move_down(&mut self, index: usize) -> bool {
        index + 1 < self.items.len() && self.move_to(index, index + 1)
    }

    /// Notes that the track playing now is being replaced by `arriving`.
    ///
    /// Repeating a track is not a departure, so a single-track loop does not
    /// pile the same song onto the trail for as long as it plays.
    fn depart(&mut self, arriving: &Song) {
        if let Some(leaving) = self.playing.take() {
            if leaving.id != arriving.id {
                self.played_history.push(leaving);
            }
        }
    }

    /// Takes the next "play next" track and starts it.
    fn take_pending(&mut self) -> Option<Song> {
        let song = self.pending_next.pop_front()?;
        self.depart(&song);
        // `current_index` is deliberately left alone: the list is waiting to
        // resume where it left off once the queued tracks are done.
        self.playing = Some(song.clone());
        Some(song)
    }

    fn step_forward(&mut self, current: usize) -> Option<usize> {
        let len = self.items.len();
        if len == 0 {
            return None;
        }
        match self.playback_mode {
            // The end of the queue is the end of playback.
            // (`then`, not `then_some`: the arithmetic must not be evaluated
            // when the test it is guarded by fails.)
            PlaybackMode::Sequential => (current + 1 < len).then(|| current + 1),
            PlaybackMode::ListLoop => Some((current + 1) % len),
            PlaybackMode::SingleTrackLoop => Some(current),
            PlaybackMode::Shuffle => self.shuffle_pick(),
        }
    }

    fn step_back(&self, current: usize) -> Option<usize> {
        let len = self.items.len();
        if len == 0 {
            return None;
        }
        match self.playback_mode {
            PlaybackMode::Sequential => (current > 0).then(|| current - 1),
            PlaybackMode::ListLoop => Some((current + len - 1) % len),
            PlaybackMode::SingleTrackLoop => Some(current),
            // Handled by `previous` before it gets here.
            PlaybackMode::Shuffle => None,
        }
    }

    /// Picks the next track under shuffle.
    ///
    /// Tracks that have not been heard this round come first, so a round is a
    /// true shuffle rather than a lottery that can repeat itself; once the
    /// round is over, a new one begins.
    fn shuffle_pick(&mut self) -> Option<usize> {
        let len = self.items.len();
        if len == 0 {
            return None;
        }
        if len == 1 {
            return Some(0);
        }

        if self.played_history.len() >= len {
            // A round has been heard. The track playing now is carried over so
            // the new round does not open by repeating it.
            let just_played = self.played_history.last().cloned();
            self.played_history.clear();
            self.played_history.extend(just_played);
        }

        let heard: HashSet<SongId> = self.played_history.iter().map(|song| song.id).collect();
        let fresh: Vec<usize> = (0..len)
            .filter(|index| {
                Some(*index) != self.current_index && !heard.contains(&self.items[*index].id)
            })
            .collect();
        if !fresh.is_empty() {
            return Some(fresh[self.rng.below(fresh.len())]);
        }

        // Everything else has been heard this round; anything but the track
        // playing now will do.
        let others: Vec<usize> = (0..len)
            .filter(|index| Some(*index) != self.current_index)
            .collect();
        Some(others[self.rng.below(others.len())])
    }
}

/// Where a position moves to when the entry at `from` is inserted at `to`.
fn shift(current: usize, from: usize, to: usize) -> usize {
    if current == from {
        to
    } else if from < current && current <= to {
        current - 1
    } else if to <= current && current < from {
        current + 1
    } else {
        current
    }
}

/// The generator behind shuffle.
///
/// Written out rather than pulled in as a dependency: the domain model takes no
/// crate dependencies, and a uniform pick needs nothing more than this.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        // The only thing this crate takes from outside itself, and only so that
        // two runs do not shuffle identically. Nothing is read or written.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self::from_seed(now)
    }
}

impl Rng {
    /// xorshift64* — the zero state is a fixed point, so it is never used.
    fn from_seed(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next_u64(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `0..bound`.
    fn below(&mut self, bound: usize) -> usize {
        if bound <= 1 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: u64) -> Song {
        Song {
            id: SongId(id),
            title: format!("song {id}"),
            artists: Vec::new(),
            album: None,
            duration_ms: None,
        }
    }

    fn ids(queue: &Queue) -> Vec<u64> {
        queue.items.iter().map(|song| song.id.get()).collect()
    }

    fn filled(mode: PlaybackMode, count: u64) -> Queue {
        let mut queue = Queue::seeded(mode, 20_261_007);
        queue.replace_items((1..=count).map(song).collect());
        queue
    }

    // ---- mode ------------------------------------------------------------

    #[test]
    fn mode_cycles_in_the_documented_order() {
        let mut mode = PlaybackMode::Sequential;
        let mut seen = vec![mode];
        for _ in 0..PlaybackMode::CYCLE.len() - 1 {
            mode = mode.cycle();
            seen.push(mode);
        }
        assert_eq!(
            seen,
            vec![
                PlaybackMode::Sequential,
                PlaybackMode::ListLoop,
                PlaybackMode::SingleTrackLoop,
                PlaybackMode::Shuffle,
            ]
        );
        assert_eq!(mode.cycle(), PlaybackMode::Sequential);
    }

    #[test]
    fn every_mode_is_in_the_cycle_exactly_once() {
        for mode in PlaybackMode::CYCLE {
            assert_eq!(
                PlaybackMode::CYCLE.iter().filter(|m| **m == mode).count(),
                1
            );
        }
    }

    #[test]
    fn default_mode_is_sequential() {
        assert_eq!(PlaybackMode::default(), PlaybackMode::Sequential);
        assert_eq!(Queue::default().playback_mode, PlaybackMode::Sequential);
    }

    // ---- play next -------------------------------------------------------

    #[test]
    fn play_next_is_fifo_and_does_not_touch_the_main_list() {
        let mut queue = Queue::new(PlaybackMode::Sequential);
        queue.replace_items(vec![song(1), song(2)]);
        queue.push_next(song(10));
        queue.push_next(song(11));

        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop_next().unwrap().id, SongId(10));
        assert_eq!(queue.pop_next().unwrap().id, SongId(11));
        assert_eq!(queue.pop_next(), None);
        assert_eq!(queue.items.len(), 2);
    }

    #[test]
    fn queued_tracks_play_before_the_list_resumes() {
        // The documented order: with A B C queued and X Y Z added to play next,
        // what follows is X Y Z A B C.
        let mut queue = filled(PlaybackMode::Sequential, 3);
        queue.push_next(song(21));
        queue.push_next(song(22));
        queue.push_next(song(23));

        let mut order = Vec::new();
        while let Some(next) = queue.advance() {
            order.push(next.id.get());
        }

        assert_eq!(order, vec![21, 22, 23, 1, 2, 3]);
        // The list itself was never rearranged.
        assert_eq!(ids(&queue), vec![1, 2, 3]);
    }

    #[test]
    fn a_queued_track_leaves_the_position_where_it_was() {
        let mut queue = filled(PlaybackMode::Sequential, 3);
        queue.select(1); // playing the second track
        queue.push_next(song(99));

        assert_eq!(queue.advance().unwrap().id, SongId(99));
        // The player is on the inserted track, but the list will resume after
        // the track it came from rather than from the start.
        assert_eq!(queue.advance().unwrap().id, SongId(3));
        assert_eq!(ids(&queue), vec![1, 2, 3]);
    }

    // ---- sequential / loops ----------------------------------------------

    #[test]
    fn sequential_plays_the_queue_once_and_then_stops() {
        let mut queue = filled(PlaybackMode::Sequential, 3);
        assert_eq!(queue.advance().unwrap().id, SongId(1));
        assert_eq!(queue.advance().unwrap().id, SongId(2));
        assert_eq!(queue.advance().unwrap().id, SongId(3));
        // The end of the queue is the end of playback, not a wrap-around.
        assert_eq!(queue.advance(), None);
    }

    #[test]
    fn list_loop_returns_to_the_start() {
        let mut queue = filled(PlaybackMode::ListLoop, 3);
        for expected in [1, 2, 3, 1, 2] {
            assert_eq!(queue.advance().unwrap().id, SongId(expected));
        }
    }

    #[test]
    fn list_loop_steps_back_over_the_start_to_the_end() {
        let mut queue = filled(PlaybackMode::ListLoop, 3);
        assert_eq!(queue.advance().unwrap().id, SongId(1));
        assert_eq!(queue.previous().unwrap().id, SongId(3));
    }

    #[test]
    fn single_track_loop_replays_the_same_track() {
        let mut queue = filled(PlaybackMode::SingleTrackLoop, 3);
        queue.select(1);
        for _ in 0..5 {
            assert_eq!(queue.advance().unwrap().id, SongId(2));
        }
        // Looping is not a departure: the trail does not fill with one song.
        assert!(
            queue.played_history.is_empty(),
            "{:?}",
            queue.played_history
        );
    }

    #[test]
    fn sequential_steps_back_through_the_queue_order() {
        let mut queue = filled(PlaybackMode::Sequential, 4);
        queue.select(2);
        assert_eq!(queue.previous().unwrap().id, SongId(2));
        assert_eq!(queue.previous().unwrap().id, SongId(1));
        // Nothing before the first track.
        assert_eq!(queue.previous(), None);
    }

    #[test]
    fn the_start_of_the_queue_has_nothing_before_it() {
        let mut queue = filled(PlaybackMode::Sequential, 3);
        assert_eq!(queue.advance().unwrap().id, SongId(1));
        assert_eq!(queue.previous(), None);
    }

    // ---- shuffle ---------------------------------------------------------

    #[test]
    fn shuffle_plays_every_track_once_before_repeating() {
        let mut queue = filled(PlaybackMode::Shuffle, 12);

        let mut heard = Vec::new();
        for _ in 0..12 {
            heard.push(queue.advance().unwrap().id.get());
        }

        let mut sorted = heard.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            (1..=12).collect::<Vec<u64>>(),
            "a round skipped or repeated: {heard:?}"
        );
        // ...and it did not simply walk the list in order.
        assert_ne!(
            heard,
            (1..=12).collect::<Vec<u64>>(),
            "{heard:?} was not shuffled"
        );
    }

    #[test]
    fn a_new_round_does_not_open_with_the_track_that_just_played() {
        let mut queue = filled(PlaybackMode::Shuffle, 6);
        for _ in 0..6 {
            queue.advance();
        }
        let last = queue.current().unwrap().id;
        assert_ne!(queue.advance().unwrap().id, last);
    }

    #[test]
    fn shuffle_never_repeats_the_track_playing_now() {
        let mut queue = filled(PlaybackMode::Shuffle, 4);
        let mut previous = queue.advance().unwrap().id;
        for _ in 0..200 {
            let next = queue.advance().unwrap().id;
            assert_ne!(next, previous, "shuffle replayed the current track");
            previous = next;
        }
    }

    #[test]
    fn going_back_under_shuffle_replays_rather_than_rolling_again() {
        let mut queue = filled(PlaybackMode::Shuffle, 8);
        let first = queue.advance().unwrap().id;
        let second = queue.advance().unwrap().id;
        assert_ne!(first, second);

        // What was heard, in order, is what going back walks through.
        assert_eq!(queue.previous().unwrap().id, first);
        assert_eq!(queue.previous(), None);

        // And forward again from there carries on.
        queue.advance();
        assert_eq!(queue.previous().unwrap().id, first);
    }

    #[test]
    fn a_single_track_queue_shuffles_to_itself() {
        let mut queue = filled(PlaybackMode::Shuffle, 1);
        assert_eq!(queue.advance().unwrap().id, SongId(1));
        assert_eq!(queue.advance().unwrap().id, SongId(1));
    }

    #[test]
    fn seeding_the_same_queue_twice_gives_the_same_order() {
        let order = |seed| {
            let mut queue = Queue::seeded(PlaybackMode::Shuffle, seed);
            queue.replace_items((1..=20).map(song).collect());
            (0..20)
                .map(|_| queue.advance().unwrap().id.get())
                .collect::<Vec<_>>()
        };
        assert_eq!(order(7), order(7));
        assert_ne!(order(7), order(8));
    }

    // ---- editing ---------------------------------------------------------

    #[test]
    fn removing_a_track_before_the_position_shifts_it() {
        let mut queue = filled(PlaybackMode::Sequential, 4);
        queue.select(3);

        assert_eq!(queue.remove(0).unwrap().id, SongId(1));
        assert_eq!(queue.current_index, Some(2));
        assert_eq!(queue.current().unwrap().id, SongId(4));
    }

    #[test]
    fn removing_the_track_playing_now_leaves_it_playing() {
        // It is in the player, not in the queue: deleting the row must not cut
        // the sound off.
        let mut queue = filled(PlaybackMode::Sequential, 3);
        queue.select(1);
        queue.remove(1);

        assert_eq!(ids(&queue), vec![1, 3]);
        assert_eq!(queue.current().unwrap().id, SongId(2));
        // The position lands on whatever took its place.
        assert_eq!(queue.current_index, Some(1));
    }

    #[test]
    fn removing_the_last_track_clears_the_position() {
        let mut queue = filled(PlaybackMode::Sequential, 2);
        queue.select(1);
        queue.remove(1);
        assert_eq!(queue.current_index, None);
        assert_eq!(queue.advance().unwrap().id, SongId(1));
    }

    #[test]
    fn removing_outside_the_queue_does_nothing() {
        let mut queue = filled(PlaybackMode::Sequential, 2);
        assert_eq!(queue.remove(9), None);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn reordering_moves_the_track_and_the_position_with_it() {
        let mut queue = filled(PlaybackMode::Sequential, 4);
        queue.select(0);

        // The track that was playing moves to the end, and the position follows.
        assert!(queue.move_to(0, 3));
        assert_eq!(ids(&queue), vec![2, 3, 4, 1]);
        assert_eq!(queue.current_index, Some(3));
        assert_eq!(queue.current().unwrap().id, SongId(1));
    }

    #[test]
    fn reordering_across_the_position_shifts_the_other_way() {
        let mut queue = filled(PlaybackMode::Sequential, 4);
        queue.select(3); // playing the fourth

        assert!(queue.move_to(0, 2));
        assert_eq!(ids(&queue), vec![2, 3, 1, 4]);
        assert_eq!(queue.current_index, Some(3));
        assert_eq!(queue.current().unwrap().id, SongId(4));
    }

    #[test]
    fn moving_up_and_down_is_bounded() {
        let mut queue = filled(PlaybackMode::Sequential, 3);
        assert!(!queue.move_up(0));
        assert!(!queue.move_down(2));
        assert!(queue.move_down(0));
        assert_eq!(ids(&queue), vec![2, 1, 3]);
        assert!(queue.move_up(1));
        assert_eq!(ids(&queue), vec![1, 2, 3]);
    }

    #[test]
    fn moving_a_track_onto_itself_changes_nothing() {
        let mut queue = filled(PlaybackMode::Sequential, 3);
        assert!(!queue.move_to(1, 1));
        assert_eq!(ids(&queue), vec![1, 2, 3]);
    }

    // ---- housekeeping ----------------------------------------------------

    #[test]
    fn replacing_items_clears_the_selection() {
        let mut queue = Queue::new(PlaybackMode::Shuffle);
        queue.current_index = Some(1);
        queue.played_history.push(song(9));
        queue.playing = Some(song(8));
        queue.push_next(song(7));
        queue.replace_items(vec![song(1)]);

        assert_eq!(queue.current_index, None);
        assert!(queue.played_history.is_empty());
        assert!(queue.playing.is_none());
        assert!(queue.pending_next.is_empty());
        assert!(queue.current().is_none());
    }

    #[test]
    fn an_empty_queue_has_nowhere_to_go() {
        let mut queue = Queue::new(PlaybackMode::ListLoop);
        assert_eq!(queue.advance(), None);
        assert_eq!(queue.previous(), None);
    }

    #[test]
    fn queue_survives_a_serde_round_trip() {
        let mut queue = Queue::new(PlaybackMode::ListLoop);
        queue.replace_items(vec![song(1), song(2)]);
        queue.select(1);
        queue.push_next(song(3));

        let json = serde_json::to_string(&queue).unwrap();
        let restored: Queue = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, queue);
        // Shuffle's position in its sequence is not persisted: it is not state
        // the user asked for, and a fresh one is as good as an old one.
        assert_eq!(queue.current().unwrap().id, SongId(2));
    }

    #[test]
    fn the_rng_stays_out_of_the_serialised_form() {
        let queue = filled(PlaybackMode::Shuffle, 3);
        let json = serde_json::to_string(&queue).unwrap();
        assert!(!json.contains("rng"), "{json}");
    }

    #[test]
    fn a_zero_seed_still_produces_a_usable_sequence() {
        // xorshift is a fixed point at zero: seeding with it must not pin every
        // pick to the same index.
        let mut queue = Queue::seeded(PlaybackMode::Shuffle, 0);
        queue.replace_items((1..=8).map(song).collect());
        let picks: Vec<u64> = (0..8).map(|_| queue.advance().unwrap().id.get()).collect();
        let mut sorted = picks.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (1..=8).collect::<Vec<u64>>(), "{picks:?}");
    }
}
