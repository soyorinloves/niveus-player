//! The abstract picture this crate produces.
//!
//! Deliberately not a ratatui type and not a terminal escape sequence: a frame
//! is a grid of characters with a colour for each half of each cell, and it is
//! the caller's job to put it on a terminal.

use std::fmt;

/// A colour one half of a cell is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    /// True colour, as three channels.
    Rgb(u8, u8, u8),
    /// An entry in the 256-colour palette.
    Indexed(u8),
    /// One of the sixteen classic colours, by index.
    ///
    /// Kept apart from [`Ink::Indexed`] because the first sixteen entries of
    /// the 256-colour palette are not the classic sixteen: they are whatever
    /// the terminal's theme says they are, and drawing them with the palette
    /// escape would lose the distinction.
    Classic(u8),
    /// The caller's accent colour.
    ///
    /// Used by the default cover, which is a glyph rather than a picture and
    /// so has no colours of its own.
    Accent,
    /// Nothing drawn here.
    None,
}

impl Ink {
    /// Whether anything is drawn in this colour.
    pub fn is_none(self) -> bool {
        self == Ink::None
    }
}

/// One cell: a character, and the colours of its two halves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The character drawn in the cell.
    pub symbol: char,
    /// The colour of the upper half of the cell.
    pub fg: Ink,
    /// The colour of the lower half of the cell.
    pub bg: Ink,
}

impl Cell {
    /// A cell with nothing in it.
    pub const BLANK: Cell = Cell {
        symbol: ' ',
        fg: Ink::None,
        bg: Ink::None,
    };

    /// A cell filled entirely with one colour.
    pub fn solid(symbol: char, ink: Ink) -> Cell {
        Cell {
            symbol,
            fg: ink,
            bg: ink,
        }
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}

/// A picture, ready to be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Frame {
    /// An empty frame of the given size.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            cells: vec![Cell::BLANK; width as usize * height as usize],
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    /// Whether there is anything to draw.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).map(|index| &self.cells[index])
    }

    pub fn set(&mut self, x: u16, y: u16, cell: Cell) {
        if let Some(index) = self.index(x, y) {
            self.cells[index] = cell;
        }
    }

    /// The cells, row by row.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(y as usize * self.width as usize + x as usize)
    }
}

impl fmt::Display for Frame {
    /// The characters alone, one row per line.
    ///
    /// Colours are left out on purpose: this is the form that is worth
    /// asserting on in a test, where escape sequences would only be noise.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for y in 0..self.height {
            for x in 0..self.width {
                let symbol = self.cell(x, y).map(|cell| cell.symbol).unwrap_or(' ');
                write!(f, "{symbol}")?;
            }
            if y + 1 < self.height {
                writeln!(f)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_frame_is_blank() {
        let frame = Frame::new(3, 2);
        assert_eq!(frame.width(), 3);
        assert_eq!(frame.height(), 2);
        assert_eq!(frame.cells().len(), 6);
        assert!(frame.cells().iter().all(|cell| *cell == Cell::BLANK));
    }

    #[test]
    fn writing_outside_the_frame_is_ignored_rather_than_panicking() {
        let mut frame = Frame::new(2, 2);
        frame.set(5, 0, Cell::solid('#', Ink::Accent));
        frame.set(0, 5, Cell::solid('#', Ink::Accent));
        assert!(frame.cells().iter().all(|cell| *cell == Cell::BLANK));
        assert!(frame.cell(9, 9).is_none());
    }

    #[test]
    fn cells_are_addressed_row_by_row() {
        let mut frame = Frame::new(3, 2);
        frame.set(2, 1, Cell::solid('#', Ink::Classic(1)));
        assert_eq!(frame.cells()[5].symbol, '#');
        assert_eq!(frame.cell(2, 1).unwrap().symbol, '#');
    }

    #[test]
    fn an_empty_frame_prints_as_nothing() {
        assert_eq!(Frame::new(0, 0).to_string(), "");
        assert!(Frame::new(0, 0).is_empty());
    }
}
