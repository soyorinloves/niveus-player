//! Helpers shared by the render tests.
//!
//! The interface's hardest-to-spot defects are width mistakes, and they only
//! show up once the content contains Chinese. These helpers exist so every
//! renderer can be checked the same way.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use unicode_width::UnicodeWidthStr;

/// The plain text of a line, styles discarded.
pub fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// Rebuilds the text of one row of a rendered buffer.
///
/// A wide character fills two cells, and the second holds no symbol of its own —
/// it reads back as a blank. Concatenating cell symbols would therefore put a
/// space after every Chinese character, so the cell a wide glyph covers is
/// skipped rather than read.
pub fn row_text(buffer: &Buffer, area: Rect, y: u16) -> String {
    let mut text = String::new();
    let mut x = area.x;
    while x < area.x + area.width {
        let symbol = buffer
            .cell((x, y))
            .expect("cell inside the buffer")
            .symbol();
        text.push_str(symbol);
        // Advance by the glyph's own width, which steps over the covered cell.
        let width = UnicodeWidthStr::width(symbol);
        x += if width >= 2 { width as u16 } else { 1 };
    }
    text
}

/// Display width of a row, in terminal columns.
pub fn row_width(buffer: &Buffer, area: Rect, y: u16) -> usize {
    UnicodeWidthStr::width(row_text(buffer, area, y).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::text::Span;
    use ratatui::widgets::Paragraph;
    use ratatui::Terminal;

    #[test]
    fn a_row_of_chinese_reads_back_without_gaps() {
        let area = Rect::new(0, 0, 20, 1);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new(Line::from(Span::raw("晴天 · 叶惠美"))), area);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // No phantom spaces between the characters: the cell each wide glyph
        // covers is stepped over rather than read back as a blank.
        assert_eq!(row_text(buffer, area, 0).trim_end(), "晴天 · 叶惠美");
        assert_eq!(row_width(buffer, area, 0), area.width as usize);
    }

    #[test]
    fn a_line_is_measured_in_columns() {
        let line = Line::from(vec![Span::raw("▶ "), Span::raw("晴天")]);
        assert_eq!(line_text(&line), "▶ 晴天");
        // Four columns of ideographs plus the marker and its space.
        assert_eq!(line.width(), 6);
    }
}
