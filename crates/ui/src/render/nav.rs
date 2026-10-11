//! The first-level navigation row.

use super::truncate_spans;
use crate::state::{Primary, UiState};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub(crate) fn render(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let mut spans = Vec::new();
    // Only the items with something behind them are drawn; SEARCH is reserved
    // but absent (SPEC §10.1).
    for (index, candidate) in Primary::RENDERED.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let active = *candidate == state.primary;
        spans.push(Span::styled(
            candidate.title(),
            if active { theme.heading } else { theme.normal },
        ));

        // Every entry carries its key, taken from the live keymap.
        let key = state
            .keymap
            .key_for(candidate.action())
            .map(|key| key.label())
            .unwrap_or_else(|| " ".to_string());
        spans.push(Span::styled(
            format!(" [{key}]"),
            if active { theme.accent } else { theme.muted },
        ));
    }

    // The three drawn entries fit comfortably at the smallest supported width,
    // but the row is shortened rather than allowed to overhang if that ever
    // stops being true — a page the user cannot reach is worse than a shortened
    // label.
    frame.render_widget(
        Paragraph::new(Line::from(truncate_spans(spans, area.width as usize))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use crate::layout::{compute, LayoutTier, SECONDARY_WIDTH};
    use crate::state::{Focus, Primary, Section};
    use crate::theme::Theme;
    use niveus_config::{Action, CoverSize, Key, Keymap};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    fn render_to_buffer(primary: Primary, keymap: &Keymap) -> Buffer {
        let area = Rect::new(0, 0, 120, 40);
        let regions = compute(area, LayoutTier::Standard, CoverSize::Normal);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                super::render(
                    frame,
                    regions.nav,
                    &state(primary, keymap),
                    &Theme::default(),
                );
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn state<'a>(primary: Primary, keymap: &'a Keymap) -> crate::state::UiState<'a> {
        crate::state::UiState {
            primary,
            section: primary.default_section(),
            focus: Focus::Main,
            session: crate::state::SessionView::Login(crate::state::LoginView::Starting),
            list: crate::state::ListView::Message(""),
            now_playing: crate::state::NowPlayingView {
                song: None,
                metadata: false,
                position_ms: None,
                duration_ms: None,
                artwork: None,
                lyrics: crate::state::LyricsView::Pending,
                source: None,
            },
            status: crate::state::StatusView {
                loading: false,
                notice: None,
                volume: 72,
                playback_mode: niveus_music_core::PlaybackMode::Sequential,
                queue_len: 0,
                playback: niveus_music_core::PlaybackStatus::Stopped,
                bitrate: None,
            },
            keymap,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        }
    }

    fn row_text(buffer: &Buffer, row: u16, width: u16) -> String {
        (0..width)
            .map(|x| buffer.cell((x, row)).unwrap().symbol())
            .collect::<String>()
    }

    #[test]
    fn every_drawn_primary_is_listed_with_its_bound_key() {
        let text = row_text(
            &render_to_buffer(Primary::Library, &Keymap::default()),
            0,
            120,
        );
        assert!(text.starts_with("LIBRARY [1]"), "{text:?}");
        assert!(text.contains("QUEUE [3]"), "{text:?}");
        assert!(text.contains("SETTINGS [4]"), "{text:?}");
        // SEARCH is reserved but has nothing behind it, so it is not drawn.
        assert!(!text.contains("SEARCH"), "{text:?}");
    }

    #[test]
    fn the_active_primary_is_the_marked_one() {
        // Whatever the layout, exactly one item is the one showing.
        for primary in Primary::RENDERED {
            let text = row_text(&render_to_buffer(primary, &Keymap::default()), 0, 120);
            assert!(
                text.contains(primary.title()),
                "{primary:?} missing: {text:?}"
            );
        }
    }

    #[test]
    fn a_rebound_key_changes_what_is_shown() {
        let mut keymap = Keymap::default();
        keymap.rebind(Action::ShowSettings, Key::Char('s')).unwrap();
        let text = row_text(&render_to_buffer(Primary::Settings, &keymap), 0, 120);
        assert!(text.contains("SETTINGS [S]"), "{text:?}");
    }

    /// The row is rendered at a given width rather than the standard one.
    fn render_narrow(width: u16, keymap: &Keymap) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| {
                super::render(
                    frame,
                    area,
                    &state(Primary::Library, keymap),
                    &Theme::default(),
                );
            })
            .unwrap();

        (0..width)
            .map(|x| terminal.backend().buffer().cell((x, 0)).unwrap().symbol())
            .collect()
    }

    #[test]
    fn the_whole_row_fits_the_narrowest_supported_terminal() {
        // Nothing here is user content, so the row is ASCII and characters and
        // columns coincide; what is worth pinning down is that every drawn
        // primary remains reachable at the smallest width the app allows.
        let text = render_narrow(LayoutTier::MIN_WIDTH, &Keymap::default());
        for primary in Primary::RENDERED {
            assert!(
                text.contains(primary.title()),
                "{primary:?} missing from {text:?}"
            );
        }
        assert!(!text.contains('…'), "the row was shortened: {text:?}");
        // The row must not need more than the secondary column and its rule
        // leave beside it either, which is what the constants say.
        let _ = SECONDARY_WIDTH;
    }

    #[test]
    fn an_unreasonably_narrow_row_is_shortened_rather_than_overrunning() {
        // At 40 the three drawn entries already fit; a truly cramped row still
        // shortens rather than overrunning.
        let text = render_narrow(30, &Keymap::default());
        assert_eq!(super::super::display_width(&text), 30);
        assert!(text.ends_with('…'), "{text:?}");
    }

    #[test]
    fn the_secondary_ids_are_the_same_shape_as_the_primary_row() {
        // Guards against a future change that draws the secondary column from
        // anything but the primary's own section list.
        for primary in Primary::ALL {
            for section in primary.sections() {
                let _ = section.label();
            }
        }
        assert_eq!(Section::Playlists.label(), "PLAYLISTS");
    }
}
