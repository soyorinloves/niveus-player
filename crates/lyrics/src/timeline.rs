//! The timeline: which line is being sung, and what sits around it.

use crate::parse::LrcLine;

/// A song's lyrics, as the moments they are sung at.
///
/// Lines are held in the order they are sung, which is what makes the window
/// around a position a matter of stepping to the neighbours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lyrics {
    lines: Vec<LrcLine>,
}

impl Lyrics {
    /// Wraps lines that are already in the order they are sung.
    pub(crate) fn new(lines: Vec<LrcLine>) -> Self {
        Self { lines }
    }

    /// Every line, in the order it is sung.
    pub fn lines(&self) -> &[LrcLine] {
        &self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The window of lines the panel shows while playback is at `position_ms`.
    ///
    /// `rows` is how much room there is for it; the window takes the largest
    /// shape that fits (see [`window_rows`]).
    pub fn window_at(&self, position_ms: u64, rows: usize) -> Window<'_> {
        let rows = window_rows(rows);
        if rows == 0 {
            return Window { lines: Vec::new() };
        }

        // The window holds `rows` as lines and blanks alternating, so the lines
        // are half of them rounded up; the line being sung takes the middle.
        let slots = rows.div_ceil(2);
        let span = ((slots - 1) / 2) as isize;

        // How many lines have started by now. The lines are sorted by
        // construction, so this is the boundary the window turns on.
        let started = self.lines.partition_point(|line| line.at_ms <= position_ms) as isize;
        // Nothing has started yet just before the first line: the middle slot is
        // then empty, and the lines coming up sit after it rather than moving.
        let current = started - 1;

        Window {
            lines: (0..slots as isize)
                .map(|slot| {
                    let index = current - span + slot;
                    usize::try_from(index)
                        .ok()
                        .and_then(|index| self.lines.get(index))
                        .map(|line| line.text.as_str())
                })
                .collect(),
        }
    }
}

/// How many rows a window may take, given how many there are.
///
/// Only `1, 5, 9, 13, …` are possible. The panel draws a line, then a blank row,
/// then a line — `N` lines take `2N − 1` rows — and the line being sung has to
/// land in the middle, which happens only when `N` is odd. So the window is
/// `4k + 1` rows: **three rows are not a window at all**, because the row in the
/// middle of `line / blank / line` is the blank one.
///
/// Fewer than one row means no window (SPEC §13.1).
pub fn window_rows(available: usize) -> usize {
    if available == 0 {
        return 0;
    }
    ((available - 1) / 4) * 4 + 1
}

/// The lines around the one being sung.
///
/// The number of slots is always odd, so the middle one is the line being sung
/// and there is the same number of rows above it as below. A slot is empty when
/// the song has no line that far away — the panel keeps the row and leaves it
/// blank rather than moving everything up (SPEC §13.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Window<'a> {
    /// Oldest first.
    lines: Vec<Option<&'a str>>,
}

impl<'a> Window<'a> {
    /// The lines, oldest first, with empty slots for what is missing.
    pub fn lines(&self) -> &[Option<&'a str>] {
        &self.lines
    }

    /// How many lines the window holds.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// How many rows this window takes once it is drawn: a line each, with a
    /// blank row between them.
    pub fn rows(&self) -> usize {
        self.lines.len().saturating_mul(2).saturating_sub(1)
    }

    /// Whether there is a window at all.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The line in a slot, oldest first.
    pub fn line(&self, slot: usize) -> Option<&'a str> {
        self.lines.get(slot).copied().flatten()
    }

    /// The line being sung, which is the middle of the window.
    ///
    /// Empty before the first line of the song has started.
    pub fn current(&self) -> Option<&'a str> {
        let middle = self.lines.len() / 2;
        self.lines.get(middle).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lyrics of a six-line song, the first line at 0:10 and one every ten
    /// seconds after it.
    fn song() -> Lyrics {
        Lyrics::new(
            [
                (10_000u64, "第一句"),
                (20_000, "第二句"),
                (30_000, "第三句"),
                (40_000, "第四句"),
                (50_000, "第五句"),
                (60_000, "第六句"),
            ]
            .into_iter()
            .map(|(at_ms, text)| LrcLine {
                at_ms,
                text: text.to_string(),
            })
            .collect(),
        )
    }

    /// The window as text, with a dot for an empty slot.
    fn slots(position_ms: u64, rows: usize) -> Vec<String> {
        let lyrics = song();
        lyrics
            .window_at(position_ms, rows)
            .lines()
            .iter()
            .map(|line| line.unwrap_or("·").to_string())
            .collect()
    }

    // ---- the shape of a window -------------------------------------------

    #[test]
    fn only_a_line_every_other_row_can_centre_the_line_being_sung() {
        // 3 is the trap: with a blank row between lines, the middle of a
        // three-row window is that blank row.
        assert_eq!(window_rows(0), 0);
        assert_eq!(window_rows(1), 1);
        assert_eq!(window_rows(2), 1);
        assert_eq!(window_rows(3), 1, "three rows is not a window");
        assert_eq!(window_rows(4), 1);
        assert_eq!(window_rows(5), 5);
        assert_eq!(window_rows(6), 5);
        assert_eq!(window_rows(8), 5);
        assert_eq!(window_rows(9), 9);
        assert_eq!(window_rows(13), 13);
        assert_eq!(window_rows(16), 13);
        assert_eq!(window_rows(17), 17);
    }

    #[test]
    fn a_window_only_ever_holds_an_odd_number_of_lines() {
        for available in 1..40usize {
            let slots = window_rows(available);
            assert_eq!(slots % 4, 1, "{available} rows became {slots}");
            assert!(slots <= available, "{available} rows became {slots}");
            // And it is the largest such shape: one more line would overflow.
            assert!(slots + 4 > available, "{available} rows became {slots}");
        }
    }

    #[test]
    fn a_window_takes_a_row_per_line_with_a_blank_between() {
        let lyrics = song();
        for rows in [1usize, 5, 9, 13] {
            let window = lyrics.window_at(35_000, rows);
            assert_eq!(window.rows(), window_rows(rows));
            assert_eq!(window.line_count(), rows.div_ceil(2));
        }
    }

    // ---- what the window shows -------------------------------------------

    #[test]
    fn the_line_being_sung_is_in_the_middle_with_equal_room_either_side() {
        let lyrics = song();
        let window = lyrics.window_at(35_000, 9);
        assert_eq!(
            window.lines(),
            [
                Some("第一句"),
                Some("第二句"),
                Some("第三句"),
                Some("第四句"),
                Some("第五句")
            ]
        );
        // Five lines, so the middle is the third: the line that has started.
        assert_eq!(window.current(), Some("第三句"));
    }

    #[test]
    fn before_the_first_line_the_middle_is_empty_and_the_rest_are_upcoming() {
        // Nothing is being sung yet, so the middle slot is empty and the lines
        // coming up already sit in the places they will move up through.
        assert_eq!(slots(0, 9), ["·", "·", "·", "第一句", "第二句"]);
        assert_eq!(slots(9_999, 5), ["·", "·", "第一句"]);
    }

    #[test]
    fn the_ends_of_the_song_leave_their_side_of_the_window_empty() {
        // No compression and no shifting: the empty slots are still rows, and
        // the line being sung is still the middle one.
        assert_eq!(slots(10_000, 5), ["·", "第一句", "第二句"]);
        assert_eq!(slots(60_000, 5), ["第五句", "第六句", "·"]);
        assert_eq!(slots(u64::MAX, 9), ["第四句", "第五句", "第六句", "·", "·"]);
    }

    #[test]
    fn a_bigger_window_shows_more_neighbours_without_moving_the_current_line() {
        let lyrics = song();
        for rows in [1usize, 5, 9, 13] {
            let window = lyrics.window_at(35_000, rows);
            assert_eq!(window.current(), Some("第三句"), "in a {rows}-row window");
        }
    }

    #[test]
    fn a_single_row_window_shows_only_the_line_being_sung() {
        let lyrics = song();
        let window = lyrics.window_at(35_000, 4);
        assert_eq!(window.line_count(), 1);
        assert_eq!(window.rows(), 1);
        assert_eq!(window.current(), Some("第三句"));
    }

    #[test]
    fn no_room_means_no_window() {
        let lyrics = song();
        let window = lyrics.window_at(35_000, 0);
        assert!(window.is_empty());
        assert_eq!(window.rows(), 0);
        assert_eq!(window.current(), None);
        assert_eq!(window.line(0), None);

        // An empty timeline still has a window — the panel keeps its rows — but
        // every slot in it is empty.
        let empty = Lyrics::new(Vec::new());
        assert!(empty.is_empty());
        let window = empty.window_at(0, 13);
        assert_eq!(window.line_count(), 7);
        assert!(window.lines().iter().all(Option::is_none));
        assert_eq!(window.current(), None);
    }
}
