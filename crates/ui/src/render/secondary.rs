//! The secondary column: the items of whichever first-level item is showing.
//!
//! It only chooses a category; opening one of its items — a playlist, a track,
//! a setting — happens in the main area beside it (SPEC §10.2).

use super::{cursor_mark, cursor_style, truncate_spans};
use crate::state::{Focus, UiState};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub(crate) fn render(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // The cursor here is the accent only while the column holds the focus; it
    // is dimmed otherwise, so which region Tab has left the arrows acting on is
    // visible (SPEC §10.3).
    let focused = state.focus == Focus::Secondary;

    let lines: Vec<Line> = state
        .primary
        .sections()
        .iter()
        .map(|section| {
            let selected = *section == state.section;
            let style = cursor_style(selected, focused, theme);
            let marker_style = style;
            Line::from(truncate_spans(
                vec![
                    Span::styled(format!("{} ", cursor_mark(selected, focused)), marker_style),
                    Span::styled(section.label(), style),
                ],
                area.width as usize,
            ))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use crate::layout::{compute, LayoutTier, SECONDARY_WIDTH};
    use crate::state::{
        Focus, ListView, LoginView, LyricsView, NowPlayingView, Primary, Section, SessionView,
        StatusView, UiState,
    };
    use crate::theme::Theme;
    use niveus_config::{CoverSize, Keymap};
    use niveus_music_core::{PlaybackMode, PlaybackStatus};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    fn draw(primary: Primary, section: Section, focus: Focus) -> (Buffer, Rect) {
        let keymap = Keymap::default();
        let area = Rect::new(0, 0, 120, 40);
        let regions = compute(area, LayoutTier::Standard, CoverSize::Normal);
        let state = UiState {
            primary,
            section,
            focus,
            session: SessionView::Login(LoginView::Starting),
            list: ListView::Message(""),
            now_playing: NowPlayingView {
                song: None,
                metadata: false,
                position_ms: None,
                duration_ms: None,
                artwork: None,
                lyrics: LyricsView::Pending,
                source: None,
            },
            status: StatusView {
                loading: false,
                notice: None,
                volume: 72,
                playback_mode: PlaybackMode::Sequential,
                queue_len: 0,
                playback: PlaybackStatus::Stopped,
                bitrate: None,
            },
            keymap: &keymap,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        };
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                super::render(frame, regions.secondary, &state, &Theme::default());
            })
            .unwrap();
        (terminal.backend().buffer().clone(), regions.secondary)
    }

    fn column(buffer: &Buffer, area: Rect) -> Vec<String> {
        (area.y..area.y + area.height)
            .map(|y| {
                (area.x..area.x + area.width)
                    .map(|x| buffer.cell((x, y)).unwrap().symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .filter(|row| !row.is_empty())
            .collect()
    }

    #[test]
    fn the_column_lists_the_primarys_own_items() {
        let (buffer, area) = draw(Primary::Library, Section::Playlists, Focus::Secondary);
        let rows = column(&buffer, area);
        assert_eq!(rows, vec!["  LIKED", "▶ PLAYLISTS", "  HISTORY"]);

        let (buffer, area) = draw(Primary::Queue, Section::Current, Focus::Secondary);
        assert_eq!(column(&buffer, area), vec!["▶ CURRENT", "  UP NEXT"]);

        let (buffer, area) = draw(Primary::Settings, Section::Appearance, Focus::Secondary);
        assert_eq!(
            column(&buffer, area),
            vec!["  PLAYBACK", "▶ APPEARANCE", "  KEYBINDINGS"]
        );
    }

    #[test]
    fn the_longest_item_fits_the_fixed_width() {
        // The widest label plus the cursor and its space; a name that grew past
        // this would be clipped, silently, at every tier.
        use unicode_width::UnicodeWidthStr;
        let longest = Primary::ALL
            .iter()
            .flat_map(|primary| primary.sections())
            .map(|section| section.label())
            .max_by_key(|label| label.width())
            .expect("there is at least one section");
        assert!(
            longest.width() + 2 <= SECONDARY_WIDTH as usize,
            "{longest} ({} cols) does not fit {SECONDARY_WIDTH}",
            longest.width()
        );
    }

    #[test]
    fn the_cursor_is_drawn_only_while_the_column_holds_the_focus() {
        let accent = Theme::default().accent.fg.unwrap();

        // Focused: the cursor marks the chosen item, in the accent colour.
        let (focused, area) = draw(Primary::Library, Section::Playlists, Focus::Secondary);
        let cell = focused.cell((area.x, area.y + 1)).unwrap();
        assert_eq!(cell.symbol(), "▶");
        assert_eq!(cell.fg, accent);

        // Not focused: the marker is gone — it belongs to the main area now —
        // and the item is left dimmed rather than accented.
        let (unfocused, area) = draw(Primary::Library, Section::Playlists, Focus::Main);
        let cell = unfocused.cell((area.x, area.y + 1)).unwrap();
        assert_eq!(cell.symbol(), " ");
        assert_ne!(cell.fg, accent, "the inactive column is still accented");
    }
}
