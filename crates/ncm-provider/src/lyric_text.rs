//! Judging and filtering what a lyric field holds.
//!
//! This lives here rather than in the lyrics crate because it is a fact about
//! *this* service: what counts as placeholder text is whatever NetEase writes
//! into the field when a track has no words to sing (SPEC §13.7). The lyrics
//! crate stays data-source-agnostic and only parses what this layer hands it.
//!
//! Two things are done with the same per-line predicate:
//!
//! * **Judge**: a body where every line is placeholder has no lyrics at all.
//! * **Filter**: a body with real lyrics has its placeholder lines — credits,
//!   mostly — dropped before anything parses them. They are metadata about the
//!   track, not words to sing, and the panel is three lines tall.
//!
//! Only `[..]` tags have to be removed to make either decision; the body is not
//! parsed as LRC here, and no line that survives is altered.

/// Credit lines that say who made the track rather than what is sung.
///
/// Matched by the label before the colon. Every one of these is a label that
/// only appears on a credit line, which is what the list is chosen for: a wrong
/// entry now costs a line of lyrics (the filter would drop it) rather than
/// nothing, and a silent loss is worse than visible noise. A label left out is
/// the cheaper mistake — it leaves a name on the panel, which is what this
/// filter exists to prevent but is at least visible.
const CREDIT_LABELS: [&str; 12] = [
    "作词",
    "作曲",
    "编曲",
    "制作人",
    "监制",
    "混音",
    "母带",
    "录音",
    "和声",
    "出品",
    "OP",
    "SP",
];

/// A note written in place of lyrics on instrumental tracks.
///
/// Matched anywhere in the line rather than at its start, because bodies write
/// it as `纯音乐，请欣赏`, as `[00:05.00]纯音乐` and, occasionally, with a word or
/// two in front. A lyric that happens to mention it would lose that one line;
/// measured over 2036 lines of real lyrics, none does.
const INSTRUMENTAL_MARKER: &str = "纯音乐";

/// Whether a body holds at least one line of real lyrics.
///
/// The rule is per line and deliberately one-sided: one real line anywhere in
/// the body makes the whole body lyrics. A track whose text is nothing but
/// credits, or an apology for having no words, has no lyrics to show — and the
/// interface shows that as `♪ 纯音乐`, not as an error (SPEC §13.4).
pub(crate) fn has_real_lyrics(body: &str) -> bool {
    body.lines().any(is_real_line)
}

/// The body without its placeholder lines, for a body that has real lyrics.
///
/// Only whole placeholder lines go; what is left is byte-for-byte what the
/// service sent, timestamps and all, including the blank lines some writers use
/// to hold a gap open (SPEC §13.7, §13.8).
pub(crate) fn without_placeholders(body: &str) -> String {
    body.lines()
        .filter(|line| !is_placeholder_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One line of real lyrics: something to sing, rather than a credit.
fn is_real_line(line: &str) -> bool {
    let text = text_of(line);
    !text.is_empty() && !is_placeholder(text)
}

/// One line that says who made the track instead of what is sung.
///
/// A line holding nothing but a timestamp is neither: it is left alone, and the
/// parser drops it for having no words to place on the timeline.
fn is_placeholder_line(line: &str) -> bool {
    let text = text_of(line);
    !text.is_empty() && is_placeholder(text)
}

/// A line's words, with its leading `[..]` tags removed.
fn text_of(line: &str) -> &str {
    without_tags(line).trim()
}

/// One line's words, judged on their own.
fn is_placeholder(text: &str) -> bool {
    if text.contains(INSTRUMENTAL_MARKER) {
        return true;
    }
    match split_credit(text) {
        Some(label) => CREDIT_LABELS
            .iter()
            .any(|known| label.eq_ignore_ascii_case(known)),
        None => false,
    }
}

/// The label of a `标签：内容` line, when it is written as one.
fn split_credit(text: &str) -> Option<&str> {
    // Both colons appear in real bodies, in the same file even.
    let (label, _) = text.split_once([':', '：'])?;
    let label = label.trim();
    if label.is_empty() || label.chars().count() > 8 {
        return None;
    }
    Some(label)
}

/// A line with its leading `[..]` tags removed.
///
/// Only the tags at the front matter: the judgement is about what a line says,
/// and a timestamp is not part of that.
fn without_tags(line: &str) -> &str {
    let mut rest = line.trim_start();
    while let Some(after_open) = rest.strip_prefix('[') {
        let Some(close) = after_open.find(']') else {
            break;
        };
        rest = after_open[close + 1..].trim_start();
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_of_real_lyrics_has_lyrics() {
        assert!(has_real_lyrics("[00:01.00]晴天\n[00:05.00]故事的小黄花"));
    }

    #[test]
    fn a_body_that_only_credits_the_people_who_made_it_has_none() {
        // A real case: an instrumental whose lyric field holds nothing but the
        // two credit lines, with no mention of `纯音乐` anywhere.
        let body = "[00:00.00]作曲 : BOY\n[00:00.00]编曲 : BOY";
        assert!(!has_real_lyrics(body));
    }

    #[test]
    fn a_note_saying_the_track_is_instrumental_is_not_a_lyric() {
        assert!(!has_real_lyrics("[00:00.00]纯音乐，请欣赏"));
    }

    #[test]
    fn one_real_line_among_credits_is_enough() {
        let body = "[00:00.00]作词 : 某人\n[00:12.00]第一句歌词";
        assert!(has_real_lyrics(body));
    }

    #[test]
    fn an_empty_or_blank_body_has_none() {
        assert!(!has_real_lyrics(""));
        assert!(!has_real_lyrics("   \n\n  "));
    }

    #[test]
    fn the_judgement_is_made_per_line_and_ignores_timestamps() {
        // The tags come off first, so a credit line is recognised however it is
        // timed — and a line that is only a timestamp is not a lyric.
        assert!(!has_real_lyrics("[00:01.00][00:02.00]作词：某人"));
        assert!(!has_real_lyrics("[00:01.00]"));
        assert!(has_real_lyrics("[00:01.00]   \n[00:02.00]真正的歌词"));
    }

    #[test]
    fn a_label_that_is_not_a_credit_is_left_alone() {
        // Lyrics often contain a colon; only the known labels make a credit.
        assert!(has_real_lyrics("[00:01.00]他说：你好"));
        // And a long line is never read as a label.
        assert!(has_real_lyrics(
            "[00:01.00]这是一行很长很长的歌词内容它不应该被当作制作人员信息"
        ));
    }

    #[test]
    fn both_colons_are_recognised() {
        assert!(!has_real_lyrics(
            "[00:01.00]作词: 某人\n[00:01.00]作曲：某人"
        ));
    }

    // ---- filtering ---------------------------------------------------------

    #[test]
    fn leading_credits_are_taken_out_so_the_first_line_is_a_lyric() {
        // The case this exists for: the panel is three lines tall, and a body
        // that opens with two credits spends two of them on a list of names.
        let body = "[00:00.00] 作词 : 田島貴男\n[00:01.00] 作曲 : 田島貴男\n[00:03.96]長く甘い口づけを交わす";
        assert_eq!(
            without_placeholders(body),
            "[00:03.96]長く甘い口づけを交わす"
        );
    }

    #[test]
    fn credits_in_the_middle_go_and_the_lines_around_them_do_not_move() {
        let body = "[00:01.00]第一句\n[00:02.00]编曲 : 某人\n[00:03.00]第二句";
        assert_eq!(
            without_placeholders(body),
            "[00:01.00]第一句\n[00:03.00]第二句"
        );
    }

    #[test]
    fn a_body_that_is_all_credits_filters_down_to_nothing() {
        // Never handed to the parser in practice — this body is judged to have
        // no lyrics at all — but the filter must not invent a lyric either.
        assert_eq!(
            without_placeholders("[00:00.00]作曲 : BOY\n[00:00.00]编曲 : BOY"),
            ""
        );
    }

    #[test]
    fn what_survives_is_untouched() {
        // §13.8: the lines that stay are the service's own bytes, tags included.
        let body = "[00:01.00]作词 : 某人\n[00:05.50]故事的小黄花\n[00:09.00]から降る雪";
        assert_eq!(
            without_placeholders(body),
            "[00:05.50]故事的小黄花\n[00:09.00]から降る雪"
        );
    }

    #[test]
    fn lines_that_are_only_timestamps_are_not_credits_and_are_left_alone() {
        // Some writers use them to hold a gap open; dropping them here would be
        // the filter overreaching into the parser's business.
        let body = "[00:01.00]第一句\n[00:02.00]\n[00:03.00]第二句";
        assert_eq!(without_placeholders(body), body);
    }

    #[test]
    fn an_ordinary_body_is_returned_whole() {
        let body = "[00:01.00]晴天\n[00:05.00]故事的小黄花";
        assert_eq!(without_placeholders(body), body);
        assert!(without_placeholders("").is_empty());
    }
}
