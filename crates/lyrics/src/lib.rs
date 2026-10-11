//! LRC parsing and the lyric timeline.
//!
//! Pure logic only: no IO, no async, no network, and no knowledge of any music
//! service. The module is told where playback has got to and works out which
//! line is being sung and what sits either side of it (SPEC §7.1, §13.5); the
//! interface only draws what it is handed.
//!
//! ```text
//! RawLyrics → parse() → Lyrics → window_at(position, rows) → lines → UI
//! ```
//!
//! The window is as tall as the room it is given allows and no taller: see
//! [`window_rows`], which is where the shape of it is decided.
//!
//! Deciding *whether* there is anything to show happens before this: an
//! instrumental, a missing original and placeholder credits are judged by the
//! provider, which is the layer that knows what its service returns (SPEC
//! §13.4, §13.7).

mod parse;
mod timeline;

pub use parse::{LrcLine, ParseError};
pub use timeline::{window_rows, Lyrics, Window};

/// Parses an LRC body into a timeline.
///
/// Fails when the body holds no timestamped line at all — text with no moments
/// of its own is not something this module can place on a timeline.
pub fn parse(body: &str) -> Result<Lyrics, ParseError> {
    let lines = parse::lines(body);
    if lines.is_empty() {
        return Err(ParseError::NoTimedLines);
    }
    Ok(Lyrics::new(lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_parses_into_a_timeline_that_can_be_read_at_a_position() {
        let lyrics = parse("[00:01.00]第一句\n[00:05.00]第二句\n[00:09.00]第三句").unwrap();
        assert_eq!(lyrics.lines().len(), 3);

        // A five-row window: one line each side of the one being sung.
        let window = lyrics.window_at(5_500, 5);
        assert_eq!(
            window.lines(),
            [Some("第一句"), Some("第二句"), Some("第三句")]
        );
        assert_eq!(window.current(), Some("第二句"));
    }

    #[test]
    fn text_with_no_timeline_is_an_error_rather_than_an_empty_song() {
        // Nothing to show is not the same as a song whose lines are all behind
        // us: the panel says the lyrics could not be used, not that the track
        // has none.
        assert_eq!(parse("只是歌词，没有时间"), Err(ParseError::NoTimedLines));
        assert_eq!(parse(""), Err(ParseError::NoTimedLines));
    }

    #[test]
    fn every_line_of_a_long_song_is_kept() {
        let body: String = (0..200)
            .map(|index| format!("[{:02}:{:02}.00]第{}句\n", index / 60, index % 60, index))
            .collect();
        assert_eq!(parse(&body).unwrap().lines().len(), 200);
    }
}
