//! The top-level navigation row.

use super::truncate_spans;
use crate::state::Page;
use crate::theme::Theme;
use niveus_config::Keymap;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub(crate) fn render(frame: &mut Frame, area: Rect, page: Page, keymap: &Keymap, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let mut spans = Vec::new();
    for (index, candidate) in Page::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let active = *candidate == page;
        spans.push(Span::styled(
            candidate.title(),
            if active { theme.heading } else { theme.normal },
        ));

        // Every entry carries its key, taken from the live keymap.
        let key = keymap
            .key_for(candidate.action())
            .map(|key| key.label())
            .unwrap_or_else(|| " ".to_string());
        spans.push(Span::styled(
            format!(" [{key}]"),
            if active { theme.accent } else { theme.muted },
        ));
    }

    // The four entries fit comfortably at the smallest supported width, but the
    // row is shortened rather than allowed to overhang if that ever stops being
    // true — a page the user cannot reach is worse than a shortened label.
    frame.render_widget(
        Paragraph::new(Line::from(truncate_spans(spans, area.width as usize))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use crate::layout::{compute, LayoutTier};
    use crate::state::Page;
    use crate::theme::Theme;
    use niveus_config::{Action, CoverSize, Key, Keymap};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    fn render_to_buffer(page: Page, keymap: &Keymap) -> Buffer {
        let area = Rect::new(0, 0, 120, 40);
        let regions = compute(area, LayoutTier::Standard, CoverSize::Normal);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                super::render(frame, regions.nav, page, keymap, &Theme::default());
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &Buffer, row: u16, width: u16) -> String {
        (0..width)
            .map(|x| buffer.cell((x, row)).unwrap().symbol())
            .collect::<String>()
    }

    #[test]
    fn every_page_is_listed_with_its_bound_key() {
        let text = row_text(
            &render_to_buffer(Page::Playlist, &Keymap::default()),
            0,
            120,
        );
        assert!(text.starts_with("PLAYLIST [1]"), "{text:?}");
        assert!(text.contains("FAVORITES [2]"));
        assert!(text.contains("HISTORY [3]"));
        assert!(text.contains("SETTINGS [4]"));
    }

    #[test]
    fn a_rebound_key_changes_what_is_shown() {
        let mut keymap = Keymap::default();
        keymap.rebind(Action::OpenSettings, Key::Char('s')).unwrap();
        let text = row_text(&render_to_buffer(Page::Settings, &keymap), 0, 120);
        assert!(text.contains("SETTINGS [S]"), "{text:?}");
    }

    /// The row is rendered at a given width rather than the standard one.
    fn render_narrow(width: u16, keymap: &Keymap) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| {
                super::render(frame, area, Page::Playlist, keymap, &Theme::default());
            })
            .unwrap();

        (0..width)
            .map(|x| terminal.backend().buffer().cell((x, 0)).unwrap().symbol())
            .collect()
    }

    #[test]
    fn the_whole_row_fits_the_narrowest_supported_terminal() {
        // Nothing here is user content, so the row is ASCII and characters and
        // columns coincide; what is worth pinning down is that all four pages
        // remain reachable at the smallest width the app allows.
        let text = render_narrow(LayoutTier::MIN_WIDTH, &Keymap::default());
        for page in Page::ALL {
            assert!(
                text.contains(page.title()),
                "{page:?} missing from {text:?}"
            );
        }
        assert!(!text.contains('…'), "the row was shortened: {text:?}");
    }

    #[test]
    fn an_unreasonably_narrow_row_is_shortened_rather_than_overrunning() {
        let text = render_narrow(40, &Keymap::default());
        assert_eq!(super::super::display_width(&text), 40);
        assert!(text.ends_with('…'), "{text:?}");
    }
}
