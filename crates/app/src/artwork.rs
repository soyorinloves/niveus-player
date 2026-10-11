//! Between the crate that draws a cover and the crate that draws the screen.
//!
//! The two do not know about each other: the interface depends only on
//! `music-core` and the configuration, so a picture reaches it in the
//! interface's own terms. Translating between them is the coordinator's job,
//! and this is where it is done.

use niveus_artwork::{ArtworkMode, ColourDepth, Frame, Ink, Options, Size};
use niveus_config;
use niveus_ui::{ArtworkCell, ArtworkInk, ArtworkView};

/// How covers should be drawn, from the user's preferences and the terminal.
pub fn options(artwork_mode: niveus_config::ArtworkMode, depth: ColourDepth) -> Options {
    Options {
        mode: match artwork_mode {
            niveus_config::ArtworkMode::Halfblock => ArtworkMode::HalfBlock,
            niveus_config::ArtworkMode::Ascii => ArtworkMode::Characters,
        },
        depth,
    }
}

/// Restates a picture in the interface's own terms.
pub fn view(frame: &Frame) -> ArtworkView {
    let cells = frame
        .cells()
        .iter()
        .map(|cell| ArtworkCell {
            symbol: cell.symbol,
            fg: ink(cell.fg),
            bg: ink(cell.bg),
        })
        .collect();
    ArtworkView::new(frame.width(), frame.height(), cells)
}

/// The region a cover is drawn into, as the drawing crate measures it.
pub fn area(region: ratatui::layout::Rect) -> Size {
    Size::new(region.width, region.height)
}

fn ink(ink: Ink) -> ArtworkInk {
    match ink {
        Ink::Rgb(r, g, b) => ArtworkInk::Rgb(r, g, b),
        Ink::Indexed(index) => ArtworkInk::Indexed(index),
        Ink::Classic(index) => ArtworkInk::Classic(index),
        Ink::Accent => ArtworkInk::Accent,
        Ink::None => ArtworkInk::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_modes_are_the_same_two_modes() {
        let depth = ColourDepth::TrueColor;
        assert_eq!(
            options(niveus_config::ArtworkMode::Halfblock, depth).mode,
            ArtworkMode::HalfBlock
        );
        assert_eq!(
            options(niveus_config::ArtworkMode::Ascii, depth).mode,
            ArtworkMode::Characters
        );
        assert_eq!(
            options(niveus_config::ArtworkMode::Ascii, depth).depth,
            depth
        );
    }

    #[test]
    fn every_colour_survives_the_crossing() {
        // The distinction between the classic sixteen and the palette's first
        // sixteen is the one most easily lost in a translation.
        let frame = Frame::new(1, 1);
        let view = view(&frame);
        assert_eq!(view.width(), 1);
        assert_eq!(view.cell(0, 0).unwrap().symbol, ' ');

        for (source, expected) in [
            (Ink::Rgb(1, 2, 3), ArtworkInk::Rgb(1, 2, 3)),
            (Ink::Indexed(200), ArtworkInk::Indexed(200)),
            (Ink::Classic(4), ArtworkInk::Classic(4)),
            (Ink::Accent, ArtworkInk::Accent),
            (Ink::None, ArtworkInk::None),
        ] {
            assert_eq!(ink(source), expected);
        }
    }

    #[test]
    fn a_picture_keeps_its_shape_across_the_crossing() {
        let mut frame = Frame::new(3, 2);
        frame.set(2, 1, niveus_artwork::Cell::solid('#', Ink::Accent));

        let view = view(&frame);
        assert_eq!((view.width(), view.height()), (3, 2));
        assert_eq!(view.cell(2, 1).unwrap().symbol, '#');
        assert_eq!(view.cell(2, 1).unwrap().fg, ArtworkInk::Accent);
        assert_eq!(view.cell(0, 0).unwrap().symbol, ' ');
    }

    #[test]
    fn a_region_becomes_the_size_the_drawing_crate_measures_in() {
        let area = area(ratatui::layout::Rect::new(4, 9, 24, 12));
        assert_eq!(area, Size::new(24, 12));
    }
}
