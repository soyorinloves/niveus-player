//! The LRC parser.
//!
//! Only the standard form is understood: a run of `[mm:ss.xx]` timestamps
//! followed by the text they belong to. The word-timed formats the service also
//! offers (`yrc`) are not read — SPEC §13.7 picks one source and this is it.

use thiserror::Error;

/// One line of a song, and the moment it is sung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LrcLine {
    /// When the line starts, in milliseconds from the start of the track.
    pub at_ms: u64,
    /// The text, exactly as the service wrote it.
    pub text: String,
}

/// Why a lyric body could not be turned into a timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ParseError {
    /// The body held no timestamped line at all.
    ///
    /// Untimed text is not a timeline: there is no moment to show it at. This is
    /// the failure the interface reports as `歌词加载失败` (SPEC §13.4) — the
    /// message naming the underlying reason goes to the log, not the screen.
    #[error("歌词中没有可用的时间轴")]
    NoTimedLines,
}

/// Reads every timed line of an LRC body.
///
/// Lines are returned in the order they are sung, whatever order the body
/// listed them in. Text that carries no timestamp is dropped: a line with no
/// moment of its own is not part of the timeline.
pub(crate) fn lines(body: &str) -> Vec<LrcLine> {
    let mut timed: Vec<LrcLine> = Vec::new();
    // A whole-file shift the body asks for, applied once every line is read.
    let mut offset_ms: i64 = 0;

    for raw in body.lines() {
        // Carriage returns survive `lines()` on CRLF bodies.
        let mut rest = raw.trim_end_matches('\r');
        let mut stamps: Vec<u64> = Vec::new();

        while let Some(after_open) = rest.strip_prefix('[') {
            // An unterminated tag is not a tag: the rest of the line is text,
            // and since it has no timestamp it is dropped like any other.
            let Some((tag, tail)) = after_open.split_once(']') else {
                break;
            };
            if let Some(at_ms) = timestamp(tag) {
                stamps.push(at_ms);
            } else if let Some(shift) = offset_tag(tag) {
                offset_ms = shift;
            }
            rest = tail;
        }

        let text = rest.trim();
        if text.is_empty() {
            // Used by some writers to blank the display between verses; it is
            // not a line to show.
            continue;
        }
        for at_ms in stamps {
            timed.push(LrcLine {
                at_ms: at_ms.saturating_add_signed(offset_ms),
                text: text.to_string(),
            });
        }
    }

    // Sorted rather than trusted: bodies are usually in order but need not be,
    // and one line carrying two timestamps can put the pair out of order.
    timed.sort_by_key(|line| line.at_ms);
    timed
}

/// The moment a `[mm:ss.xx]` tag names, if it is one.
///
/// Every field is refused rather than guessed at: a tag that is not written as
/// a time is not a time, whatever it looks like.
fn timestamp(tag: &str) -> Option<u64> {
    let (minutes, rest) = tag.split_once(':')?;
    let minutes: u64 = digits(minutes)?;
    // The fraction is separated by a full stop in the format's own examples and
    // by a colon in bodies the service actually returns — `[00:19:90]` means the
    // same as `[00:19.90]`, and it is common enough that rejecting it would
    // silently drop most of a song.
    let (seconds, fraction) = match rest.split_once(['.', ':']) {
        Some((seconds, fraction)) => (seconds, fraction.trim()),
        None => (rest, ""),
    };
    let seconds: u64 = digits(seconds)?;

    let millis = if fraction.is_empty() {
        0
    } else {
        // Tenths, hundredths and thousandths all appear in the wild; the digits
        // are left-aligned, so `5` means 500ms and not 5ms. Anything longer is
        // not a fraction anyone writes.
        if fraction.len() > 3 {
            return None;
        }
        let value: u64 = digits(fraction)?;
        value.checked_mul(10u64.pow(3 - fraction.len() as u32))?
    };

    minutes
        .checked_mul(60_000)?
        .checked_add(seconds.checked_mul(1_000)?)?
        .checked_add(millis)
}

/// A shift the body asks for, in milliseconds: `[offset:-500]`.
///
/// Part of the format, so it is honoured; ignoring it would be a silent
/// half-second of drift on the bodies that carry one.
fn offset_tag(tag: &str) -> Option<i64> {
    let (key, value) = tag.split_once(':')?;
    if !key.trim().eq_ignore_ascii_case("offset") {
        return None;
    }
    value.trim().parse().ok()
}

/// A tag's number, if it is entirely digits.
///
/// Rejecting anything else is what keeps `[ti:标题]` and `[ar:歌手]` from being
/// read as times.
fn digits(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(times: &[u64], text: &str) -> Vec<LrcLine> {
        times
            .iter()
            .map(|at_ms| LrcLine {
                at_ms: *at_ms,
                text: text.to_string(),
            })
            .collect()
    }

    #[test]
    fn timed_lines_are_read_in_order() {
        let body = "[00:01.00]第一句\n[00:05.50]第二句\n[00:09.25]第三句";
        assert_eq!(
            lines(body),
            [
                at(&[1_000], "第一句"),
                at(&[5_500], "第二句"),
                at(&[9_250], "第三句")
            ]
            .concat()
        );
    }

    #[test]
    fn a_colon_written_fraction_is_the_same_as_a_full_stop() {
        // Taken from a body the service really returned: most of its lines were
        // written this way, so a parser that only accepts the full stop would
        // drop the song and keep its credit lines.
        let body = "[00:19:90]そんなに大事な\n[00:23:47]人がいるのにどうして";
        assert_eq!(
            lines(body),
            at(&[19_900], "そんなに大事な")
                .into_iter()
                .chain(at(&[23_470], "人がいるのにどうして"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_fraction_is_read_by_its_digits_not_its_face_value() {
        // `5` is half a second, not five milliseconds: the digits are
        // left-aligned no matter how many of them the writer used.
        assert_eq!(lines("[00:01.5]a"), at(&[1_500], "a"));
        assert_eq!(lines("[00:01.50]a"), at(&[1_500], "a"));
        assert_eq!(lines("[00:01.500]a"), at(&[1_500], "a"));
        assert_eq!(lines("[00:01.05]a"), at(&[1_050], "a"));
        assert_eq!(lines("[00:01]a"), at(&[1_000], "a"));
    }

    #[test]
    fn a_line_can_carry_several_timestamps() {
        // A chorus that comes round twice is one line of text with two moments.
        assert_eq!(
            lines("[00:10.00][01:20.00]副歌"),
            at(&[10_000, 80_000], "副歌")
        );
    }

    #[test]
    fn a_body_out_of_order_is_sorted_into_the_order_it_is_sung() {
        let body = "[00:30.00]后\n[00:10.00]前";
        assert_eq!(
            lines(body),
            at(&[10_000], "前")
                .into_iter()
                .chain(at(&[30_000], "后"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn credit_tags_are_not_read_as_times() {
        let body = "[ti:标题]\n[ar:歌手]\n[by:某人]\n[00:01.00]歌词";
        assert_eq!(lines(body), at(&[1_000], "歌词"));
    }

    #[test]
    fn an_offset_shifts_every_line_and_never_below_zero() {
        let body = "[offset:-500]\n[00:01.00]a\n[00:00.20]b";
        // The second line is shifted past the start of the track; it stays at
        // zero rather than wrapping round.
        assert_eq!(
            lines(body),
            at(&[0], "b")
                .into_iter()
                .chain(at(&[500], "a"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn text_without_a_timestamp_is_not_part_of_the_timeline() {
        assert!(lines("只是一段没有时间的文字").is_empty());
        assert!(lines("").is_empty());
        assert!(lines("\n\n").is_empty());
    }

    #[test]
    fn a_line_with_no_text_is_not_a_line_to_show() {
        assert!(lines("[00:01.00]").is_empty());
        assert!(lines("[00:01.00]   ").is_empty());
    }

    #[test]
    fn a_broken_tag_is_not_a_crash() {
        // Unterminated, empty, and non-numeric tags all fall through as text.
        assert!(lines("[00:01.00").is_empty());
        assert!(lines("[]x").is_empty());
        assert!(lines("[aa:bb]x").is_empty());
        // The tag is unterminated, so nothing after it is timed either.
        assert_eq!(
            lines("[00:01.00]歌词 [00:02.00]"),
            at(&[1_000], "歌词 [00:02.00]")
        );
    }

    #[test]
    fn carriage_returns_do_not_end_up_in_the_text() {
        assert_eq!(lines("[00:01.00]歌词\r\n"), at(&[1_000], "歌词"));
    }

    #[test]
    fn minutes_are_not_capped_at_an_hour() {
        assert_eq!(lines("[75:00.00]尾声"), at(&[75 * 60_000], "尾声"));
    }

    #[test]
    fn numbers_are_trimmed_but_not_guessed() {
        assert_eq!(lines("[ 00 : 01 . 50 ]a"), at(&[1_500], "a"));
        assert!(lines("[1e3:00.00]a").is_empty());
        // A four-digit fraction is not a tenth-of-a-millisecond value anyone
        // writes; it is refused rather than misread.
        assert!(lines("[00:01.0000]a").is_empty());
    }
}
