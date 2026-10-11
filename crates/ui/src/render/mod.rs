//! Drawing the view model.
//!
//! Every rule the design states about structure lives here: horizontal rules
//! between sections, rules spanning the full width of their region, blank space
//! rather than boxes, and no decoration that does not carry information.

use crate::layout::{compute, LayoutTier, Regions};
use crate::state::{ListView, SessionView, UiState};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod list;
mod login;
mod nav;
mod now_playing;
mod secondary;
mod status;
mod toosmall;

/// The rule character used between sections.
pub(crate) const RULE: &str = "─";
/// The rule character used between side-by-side panes.
pub(crate) const PANE_RULE: &str = "│";
/// Shown when a value is unknown.
pub(crate) const UNKNOWN: &str = "—";
/// The marker on the row the cursor is on.
pub(crate) const CURSOR: &str = "▶";

/// The marker a row carries when the cursor is on it.
///
/// There is one cursor and it belongs to the region that holds the focus: Tab
/// moves it there, and the row it leaves keeps only a dimmed label. This is the
/// visible half of the focus model, and its absence would make the arrows act
/// on a region the user cannot pick out (SPEC §10.3, §23.1).
///
/// It is two columns wide with its trailing space, so every row of every list
/// is laid out the same whether or not it is selected.
pub(crate) fn cursor_mark(selected: bool, focused: bool) -> &'static str {
    if selected && focused {
        CURSOR
    } else {
        " "
    }
}

/// How a row's label is drawn, given whether it is selected and whether its
/// region holds the focus.
///
/// The region with the focus draws its selection in the accent colour; the
/// other region keeps it, dimmed, so the user can still see where the cursor
/// will be when Tab brings them back.
pub(crate) fn cursor_style(selected: bool, focused: bool, theme: &Theme) -> Style {
    if !selected {
        theme.normal
    } else if focused {
        theme.accent
    } else {
        theme.muted
    }
}

/// Draws one frame.
pub fn render(frame: &mut Frame, state: &UiState, theme: &Theme) {
    let regions = compute(frame.area(), state.tier, state.cover_size);

    // Sign-in takes the whole screen; there is nothing to browse yet.
    if matches!(state.session, SessionView::Login(_)) {
        login::render(frame, regions.area, state, theme);
        return;
    }

    if state.tier == LayoutTier::TooSmall {
        toosmall::render(frame, regions.area, theme);
        return;
    }

    draw_sections(frame, &regions, state, theme);
}

fn draw_sections(frame: &mut Frame, regions: &Regions, state: &UiState, theme: &Theme) {
    nav::render(frame, regions.nav, state, theme);
    horizontal_rule(frame, regions.nav_rule, theme);

    secondary::render(frame, regions.secondary, state, theme);
    vertical_rule(frame, regions.secondary_rule, theme);
    list::render(frame, regions.main, state, theme);
    if let Some(area) = regions.pane_rule {
        vertical_rule(frame, area, theme);
    }
    if regions.now_playing.is_some() {
        now_playing::render(frame, regions, state, theme);
    }

    horizontal_rule(frame, regions.status_rule, theme);
    status::render(frame, regions.status, state, theme);
}

/// A rule spanning the full width of its region.
pub(crate) fn horizontal_rule(frame: &mut Frame, area: Rect, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let line = RULE.repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(line, theme.divider))),
        area,
    );
}

/// A rule spanning the full height of its region.
pub(crate) fn vertical_rule(frame: &mut Frame, area: Rect, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows: Vec<Line> = (0..area.height)
        .map(|_| Line::from(Span::styled(PANE_RULE, theme.divider)))
        .collect();
    frame.render_widget(Paragraph::new(rows), area);
}

/// The ellipsis appended to anything that had to be shortened.
pub(crate) const ELLIPSIS: &str = "…";
/// Blank columns kept between two columns of content on the same row.
const COLUMN_GAP: usize = 1;

/// Width of a string in terminal columns.
///
/// Chinese, Japanese, Korean and full-width punctuation take two columns each.
/// Measuring in characters would report half their true width — eighteen
/// ideographs are eighteen characters but thirty-six columns — which is enough
/// to push a right-aligned column off the edge of the row. See SPEC §10.
pub(crate) fn display_width(text: &str) -> usize {
    text.width()
}

/// Shortens `text` to at most `max` columns, marking the cut with an ellipsis.
///
/// A wide character is never split: cutting one in half would leave a stray
/// column of colour where half a glyph used to be.
pub(crate) fn truncate(text: &str, max: usize) -> String {
    if display_width(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut kept = cut(text, max - ELLIPSIS.width());
    // A cut can land just after a space, and `word …` reads like a mistake.
    while kept.ends_with(' ') {
        kept.pop();
    }
    kept.push_str(ELLIPSIS);
    kept
}

/// Shortens `text` to at most `max` columns without any marker.
fn cut(text: &str, max: usize) -> String {
    let mut kept = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + width > max {
            break;
        }
        kept.push(ch);
        used += width;
    }
    kept
}

/// Shortens a run of styled spans to at most `max` columns.
///
/// Styles survive the cut, and the ellipsis takes the style of whatever it
/// replaced so the join does not read as a different kind of text.
pub(crate) fn truncate_spans(spans: Vec<Span<'_>>, max: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(Span::width).sum();
    if total <= max {
        return spans.into_iter().map(own).collect();
    }

    let budget = max.saturating_sub(ELLIPSIS.width());
    let mut kept = Vec::new();
    let mut used = 0;
    let mut cut_style = Style::default();

    for span in spans {
        let content = span.content.into_owned();
        let width = display_width(&content);
        if used + width <= budget {
            used += width;
            cut_style = span.style;
            kept.push(Span::styled(content, span.style));
            continue;
        }
        cut_style = span.style;
        let mut remainder = cut(&content, budget.saturating_sub(used));
        while remainder.ends_with(' ') {
            remainder.pop();
        }
        if !remainder.is_empty() {
            kept.push(Span::styled(remainder, span.style));
        }
        break;
    }

    if max > 0 {
        kept.push(Span::styled(ELLIPSIS, cut_style));
    }
    kept
}

fn own(span: Span<'_>) -> Span<'static> {
    Span::styled(span.content.into_owned(), span.style)
}

/// Renders one line with `right` pushed against the region's trailing edge.
///
/// The left side is shortened when the two cannot both fit, so the value on the
/// right — a duration, a count — is never the thing that gets dropped. When the
/// right side alone would not fit, it is the one that keeps the row.
pub(crate) fn split_line(left: Vec<Span<'_>>, right: Vec<Span<'_>>, width: u16) -> Line<'static> {
    let width = width as usize;
    let right_width: usize = right.iter().map(Span::width).sum();

    if right_width == 0 {
        return Line::from(truncate_spans(left, width));
    }
    if right_width + COLUMN_GAP >= width {
        return Line::from(truncate_spans(right, width));
    }

    let available = width - right_width;
    let mut spans = truncate_spans(left, available - COLUMN_GAP);

    // Pad so the right value lands against the trailing edge.
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::raw(" ".repeat(available.saturating_sub(used))));
    spans.extend(right.into_iter().map(own));

    Line::from(spans)
}

/// A single styled line, padded to the region width.
pub(crate) fn line_of(text: impl Into<String>, style: Style) -> Line<'static> {
    Line::from(Span::styled(text.into(), style))
}

/// `mm:ss`, or `—` when the duration is unknown.
pub(crate) fn format_duration(ms: Option<u64>) -> String {
    match ms {
        Some(ms) => {
            let seconds = ms / 1000;
            format!("{:02}:{:02}", seconds / 60, seconds % 60)
        }
        None => UNKNOWN.to_string(),
    }
}

/// `MM-DD` for a moment given in seconds since the Unix epoch.
///
/// Deliberately not a full date and time: the history is a list of what was
/// heard, and the day it was heard on is what tells one entry from another.
/// Working the calendar out here keeps the domain model free of a date
/// dependency — it stores a plain number and this turns it into something a
/// person reads.
pub(crate) fn format_date(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let (_year, month, day) = civil_from_days(days);
    format!("{month:02}-{day:02}")
}

/// The civil date for a count of days since 1970-01-01.
///
/// Howard Hinnant's `civil_from_days`, which is exact for every date a `u64`
/// count of seconds can reach and needs no lookup tables.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The note shown when a list has no rows and nothing is being fetched.
pub(crate) fn empty_note(list: &ListView<'_>) -> &'static str {
    use crate::strings::list as text;
    match list {
        ListView::Playlists { .. } => text::EMPTY_PLAYLISTS,
        ListView::Songs { .. } => text::EMPTY_SONGS,
        ListView::Likes { .. } => text::EMPTY_LIKED,
        ListView::History { .. } => text::EMPTY_HISTORY,
        ListView::Queue { .. } => text::EMPTY_QUEUE,
        ListView::UpNext { .. } => text::EMPTY_UP_NEXT,
        ListView::Settings { .. } | ListView::Keybindings { .. } | ListView::Message(_) => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::line_text;
    use ratatui::style::Modifier;

    #[test]
    fn a_right_hand_value_is_pushed_to_the_edge() {
        let line = split_line(vec![Span::raw("left")], vec![Span::raw("04:21")], 20);
        let text = line_text(&line);
        assert_eq!(display_width(&text), 20);
        assert!(text.starts_with("left"));
        assert!(text.ends_with("04:21"));
    }

    #[test]
    fn an_overlong_right_hand_value_takes_the_row_to_itself() {
        // The left side is dropped entirely, and the value is then shortened to
        // the region rather than overhanging it.
        let line = split_line(vec![Span::raw("left")], vec![Span::raw("0123456789")], 5);
        let text = line_text(&line);
        assert_eq!(line.width(), 5);
        assert!(text.starts_with("0123"), "{text:?}");
        assert!(!text.contains("left"), "{text:?}");
    }

    #[test]
    fn no_right_hand_value_leaves_the_left_one_alone() {
        let line = split_line(vec![Span::raw("left")], vec![], 20);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "left");
    }

    #[test]
    fn durations_are_formatted_or_replaced_by_a_dash() {
        assert_eq!(format_duration(Some(0)), "00:00");
        assert_eq!(format_duration(Some(269_000)), "04:29");
        assert_eq!(format_duration(Some(3_600_000)), "60:00");
        assert_eq!(format_duration(None), "—");
    }

    // ---- display width -------------------------------------------------
    //
    // These are the tests that matter: every one of them fails on ASCII-only
    // content, because characters and columns coincide there.

    #[test]
    fn chinese_text_is_twice_as_wide_as_its_character_count() {
        let text = "播放控制尚未接入";
        // The contrast is the whole point of this test: eight characters, and
        // twice that many columns to lay out.
        assert_eq!(text.chars().count(), 8);
        assert_eq!(display_width(text), 16);
    }

    #[test]
    fn full_width_punctuation_counts_as_two_columns() {
        assert_eq!(display_width("，"), 2);
        assert_eq!(display_width("。"), 2);
        assert_eq!(display_width("："), 2);
        // The separator used between a title and its artist is the half-width
        // middle dot, which takes a single column.
        assert_eq!(display_width("·"), 1);
        assert_eq!(display_width("a·b"), 3);
    }

    #[test]
    fn truncation_cuts_on_a_character_boundary_and_marks_the_cut() {
        let text = "这是一首非常长的中文歌曲标题";
        let short = truncate(text, 10);

        assert!(display_width(&short) <= 10, "{short:?} is too wide");
        assert!(short.ends_with(ELLIPSIS));

        let kept = short.trim_end_matches(ELLIPSIS);
        assert!(text.starts_with(kept), "{kept:?} is not a prefix");
        // Whole characters only: cutting one in half leaves a stray column.
        assert_eq!(display_width(kept) % 2, 0, "{kept:?} holds half a glyph");
    }

    #[test]
    fn a_cut_that_lands_on_a_space_does_not_leave_a_gap_before_the_marker() {
        // Both cuts fall immediately after a space, which would otherwise be
        // left dangling in front of the ellipsis.
        assert_eq!(truncate("abc def", 5), "abc…");
        assert_eq!(truncate("晴天 你好", 7), "晴天…");
        assert!(!truncate("晴天 你好", 7).contains(" …"));
    }

    #[test]
    fn truncation_leaves_short_text_untouched() {
        assert_eq!(truncate("晴天", 10), "晴天");
        assert_eq!(truncate("晴天", 4), "晴天");
        assert_eq!(truncate("anything", 0), "");
    }

    #[test]
    fn truncating_spans_keeps_the_styling() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let spans = vec![
            Span::styled("▶ ".to_string(), Style::default()),
            Span::styled("这是一首很长的歌名".to_string(), bold),
        ];
        let cut = truncate_spans(spans, 9);

        let text: String = cut.iter().map(|s| s.content.as_ref()).collect();
        assert!(display_width(&text) <= 9);
        assert!(text.ends_with(ELLIPSIS));
        // The ellipsis inherits the style of the text it replaced.
        assert_eq!(cut.last().unwrap().style, bold);
    }

    #[test]
    fn a_long_chinese_left_side_gives_way_to_the_right_value() {
        let line = split_line(
            vec![Span::raw("是一首名字非常非常长的中文歌曲需要被截断")],
            vec![Span::raw("04:21")],
            30,
        );
        let text = line_text(&line);

        assert_eq!(line.width(), 30, "{text:?}");
        assert!(text.ends_with("04:21"), "{text:?}");
        assert!(text.contains(ELLIPSIS), "{text:?}");
        // The value never collides with the ellipsis.
        assert!(!text.contains(&format!("{ELLIPSIS}04:21")), "{text:?}");
    }

    #[test]
    fn a_chinese_left_side_that_fits_is_not_shortened() {
        let line = split_line(vec![Span::raw("晴天")], vec![Span::raw("04:29")], 40);
        let text = line_text(&line);

        assert_eq!(line.width(), 40);
        assert!(text.starts_with("晴天"));
        assert!(text.ends_with("04:29"));
        assert!(!text.contains(ELLIPSIS));
    }

    #[test]
    fn a_right_value_too_wide_for_the_region_still_owns_the_row() {
        let line = split_line(vec![Span::raw("晴天")], vec![Span::raw("04:29")], 3);
        let text = line_text(&line);
        assert!(line.width() <= 3, "{text:?}");
        assert!(text.starts_with("04"), "{text:?}");
        assert!(!text.contains('晴'), "{text:?}");
    }

    #[test]
    fn chinese_text_is_measured_in_columns_when_right_aligned() {
        // The whole point: character counting would leave this two columns
        // short and pull the duration inwards.
        let line = split_line(vec![Span::raw("▶ 01  晴天")], vec![Span::raw("04:29")], 40);
        assert_eq!(line.width(), 40, "{:?}", line_text(&line));
    }
}
