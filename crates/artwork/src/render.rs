//! Turning sampled colours into cells.
//!
//! Two ways of drawing, chosen by configuration:
//!
//! * half-block — `▀` with the upper sample as the foreground colour and the
//!   lower as the background, which puts two vertical samples in every cell;
//! * character ramp — a brightness mapped onto a ramp of characters, one sample
//!   per cell, coloured if the terminal can show colour.
//!
//! The first is a picture shown through characters rather than a drawing made
//! of them, which is why it is the default.

use crate::frame::{Cell, Frame, Ink};
use crate::geometry::Size;
use crate::palette::{luminance, ColourDepth};

/// The character a half-block cell is drawn with, always.
const HALF_BLOCK: char = '▀';

/// The ramp used when characters stand in for shading, brightest last.
const RAMP: [char; 9] = ['.', ':', '-', '=', '+', '*', '#', '%', '@'];

/// How a cover is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtworkMode {
    /// Two vertically stacked samples per cell, using both colours.
    #[default]
    HalfBlock,
    /// Brightness mapped onto a character ramp.
    Characters,
}

impl ArtworkMode {
    /// The mode that will actually be used for this colour depth.
    ///
    /// Half-blocks are two colours in one cell; with no colour to show, that
    /// leaves nothing but a solid block, so the ramp takes over.
    pub fn resolve(self, depth: ColourDepth) -> ArtworkMode {
        match (self, depth.has_colour()) {
            (ArtworkMode::HalfBlock, false) => ArtworkMode::Characters,
            (mode, _) => mode,
        }
    }

    /// Vertical samples taken from each cell.
    pub fn samples_per_cell(self) -> u16 {
        match self {
            ArtworkMode::HalfBlock => 2,
            ArtworkMode::Characters => 1,
        }
    }
}

/// A picture sampled down to one colour per sample point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Samples {
    width: u16,
    height: u16,
    /// Row-major, top to bottom.
    colours: Vec<(u8, u8, u8)>,
}

impl Samples {
    pub fn new(width: u16, height: u16, colours: Vec<(u8, u8, u8)>) -> Self {
        debug_assert_eq!(colours.len(), width as usize * height as usize);
        Self {
            width,
            height,
            colours,
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    /// The colour at a sample point, holding the edge when asked beyond it.
    pub fn at(&self, x: u16, y: u16) -> (u8, u8, u8) {
        if self.width == 0 || self.height == 0 {
            return (0, 0, 0);
        }
        let x = x.min(self.width - 1) as usize;
        let y = y.min(self.height - 1) as usize;
        self.colours[y * self.width as usize + x]
    }
}

/// Draws a picture as half-block cells.
///
/// The samples are expected to be twice as tall as the frame: each cell takes
/// one sample for its upper half and one for its lower.
pub fn half_block(samples: &Samples, depth: ColourDepth) -> Frame {
    let width = samples.width();
    let height = samples.height() / 2;
    let mut frame = Frame::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let upper = samples.at(x, y * 2);
            let lower = samples.at(x, y * 2 + 1);
            frame.set(
                x,
                y,
                Cell {
                    symbol: HALF_BLOCK,
                    fg: depth.ink(upper),
                    bg: depth.ink(lower),
                },
            );
        }
    }
    frame
}

/// Draws a picture as characters, one sample per cell.
pub fn characters(samples: &Samples, depth: ColourDepth) -> Frame {
    let mut frame = Frame::new(samples.width(), samples.height());

    for y in 0..samples.height() {
        for x in 0..samples.width() {
            let colour = samples.at(x, y);
            frame.set(
                x,
                y,
                Cell {
                    symbol: ramp_char(colour),
                    fg: depth.ink(colour),
                    bg: Ink::None,
                },
            );
        }
    }
    frame
}

/// The ramp character for a colour's brightness.
pub fn ramp_char(colour: (u8, u8, u8)) -> char {
    let brightness = luminance(colour).clamp(0.0, 1.0);
    let step = (brightness * (RAMP.len() - 1) as f64).round() as usize;
    RAMP[step.min(RAMP.len() - 1)]
}

/// Draws `samples` in `mode`, over the cells they were taken for.
pub fn draw(samples: &Samples, cells: Size, mode: ArtworkMode, depth: ColourDepth) -> Frame {
    match mode {
        ArtworkMode::HalfBlock => {
            let frame = half_block(samples, depth);
            debug_assert_eq!((frame.width(), frame.height()), (cells.cols, cells.rows));
            frame
        }
        ArtworkMode::Characters => characters(samples, depth),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame-sized picture of one colour.
    fn flat(colour: (u8, u8, u8), width: u16, height: u16) -> Samples {
        Samples::new(
            width,
            height,
            vec![colour; width as usize * height as usize],
        )
    }

    #[test]
    fn half_blocks_stack_two_samples_in_every_cell() {
        // Top half red, bottom half blue.
        let samples = Samples::new(1, 2, vec![(255, 0, 0), (0, 0, 255)]);
        let frame = half_block(&samples, ColourDepth::TrueColor);

        assert_eq!((frame.width(), frame.height()), (1, 1));
        let cell = frame.cell(0, 0).unwrap();
        assert_eq!(cell.symbol, '▀');
        assert_eq!(
            cell.fg,
            Ink::Rgb(255, 0, 0),
            "the top sample is not the foreground"
        );
        assert_eq!(
            cell.bg,
            Ink::Rgb(0, 0, 255),
            "the bottom sample is not the background"
        );
    }

    #[test]
    fn a_character_cell_takes_one_sample() {
        let samples = flat((255, 255, 255), 2, 1);
        let frame = characters(&samples, ColourDepth::TrueColor);
        assert_eq!((frame.width(), frame.height()), (2, 1));
        assert_eq!(frame.cell(0, 0).unwrap().symbol, '@');
    }

    #[test]
    fn the_ramp_runs_from_dark_to_bright() {
        assert_eq!(ramp_char((0, 0, 0)), '.');
        assert_eq!(ramp_char((255, 255, 255)), '@');
        // Monotonic: brighter is never a lighter-on-the-page character.
        let mut previous = 0;
        for level in 0..=255u8 {
            let index = RAMP
                .iter()
                .position(|c| *c == ramp_char((level, level, level)))
                .unwrap();
            assert!(index >= previous, "the ramp went backwards at {level}");
            previous = index;
        }
    }

    #[test]
    fn characters_are_drawn_in_their_own_colour() {
        let samples = flat((10, 200, 30), 1, 1);
        let frame = characters(&samples, ColourDepth::TrueColor);
        assert_eq!(frame.cell(0, 0).unwrap().fg, Ink::Rgb(10, 200, 30));
        // A character has no second colour: the lower half is the cell's own
        // background, not something this crate draws.
        assert_eq!(frame.cell(0, 0).unwrap().bg, Ink::None);
    }

    #[test]
    fn without_colour_the_ramp_loses_its_colour_too() {
        let samples = flat((10, 200, 30), 1, 1);
        let frame = characters(&samples, ColourDepth::Monochrome);
        assert_eq!(frame.cell(0, 0).unwrap().fg, Ink::None);
        assert_ne!(frame.cell(0, 0).unwrap().symbol, ' ');
    }

    #[test]
    fn half_blocks_give_way_to_characters_when_there_is_no_colour() {
        // A half-block cell with no colour is a solid block: the shape of the
        // picture is lost entirely.
        assert_eq!(
            ArtworkMode::HalfBlock.resolve(ColourDepth::Monochrome),
            ArtworkMode::Characters
        );
        // With colour, and when asked for characters, the choice stands.
        assert_eq!(
            ArtworkMode::HalfBlock.resolve(ColourDepth::TrueColor),
            ArtworkMode::HalfBlock
        );
        assert_eq!(
            ArtworkMode::Characters.resolve(ColourDepth::TrueColor),
            ArtworkMode::Characters
        );
    }

    #[test]
    fn samples_hold_their_edge_when_asked_beyond_it() {
        let samples = Samples::new(2, 1, vec![(1, 1, 1), (2, 2, 2)]);
        assert_eq!(samples.at(0, 0), (1, 1, 1));
        assert_eq!(samples.at(1, 0), (2, 2, 2));
        assert_eq!(samples.at(9, 0), (2, 2, 2));
        assert_eq!(samples.at(0, 9), (1, 1, 1));
    }

    #[test]
    fn an_empty_sample_set_draws_nothing_rather_than_panicking() {
        let samples = Samples::new(0, 0, Vec::new());
        assert_eq!(samples.at(0, 0), (0, 0, 0));
        assert!(half_block(&samples, ColourDepth::TrueColor).is_empty());
        assert!(characters(&samples, ColourDepth::TrueColor).is_empty());
    }
}
