//! The screen shown when the terminal cannot hold the interface.

use crate::layout::LayoutTier;
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub(crate) fn render(frame: &mut Frame, area: Rect, theme: &Theme) {
    let text = vec![
        Line::from(Span::styled("Terminal size too small", theme.error)),
        Line::default(),
        Line::from(Span::styled(
            format!("current: {}×{}", area.width, area.height),
            theme.muted,
        )),
        Line::from(Span::styled(
            format!(
                "needed:  at least {}×{}",
                LayoutTier::MIN_WIDTH,
                LayoutTier::MIN_HEIGHT
            ),
            theme.muted,
        )),
    ];
    frame.render_widget(Paragraph::new(text).centered(), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::row_width;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render_at(width: u16, height: u16) -> (ratatui::buffer::Buffer, Rect) {
        let area = Rect::new(0, 0, width, height);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, area, &Theme::default()))
            .unwrap();
        (terminal.backend().buffer().clone(), area)
    }

    #[test]
    fn the_explanation_stays_within_the_area_it_is_given() {
        // This screen is shown precisely when there is very little room, so it
        // has to cope with the smallest area the app can be handed.
        for (width, height) in [(80, 24), (60, 16), (30, 6), (12, 3)] {
            let (buffer, area) = render_at(width, height);
            for y in 0..area.height {
                assert_eq!(
                    row_width(&buffer, area, y),
                    area.width as usize,
                    "row {y} at {width}×{height} spills out of its area"
                );
            }
        }
    }

    #[test]
    fn both_sizes_are_reported() {
        let (buffer, area) = render_at(120, 40);
        let text: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer.cell((x, y)).unwrap().symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Terminal size too small"), "{text}");
        assert!(text.contains("120×40"), "{text}");
        assert!(text.contains("80×24"), "{text}");
    }
}
