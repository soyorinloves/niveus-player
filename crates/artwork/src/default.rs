//! The cover shown when there is no picture: Niveus's own infinity sign.
//!
//! It is a glyph, not a picture, so it is drawn from the curve itself rather
//! than from a font or a bitmap: two rings with a single crossing in the
//! middle, which is what an infinity sign is.

use crate::frame::{Cell, Frame, Ink};
use crate::geometry::Size;

/// How wide the figure is for its height, in cells.
///
/// A cell is twice as tall as it is wide, so this is not the aspect of the
/// curve — it is the aspect the curve looks right at once a cell's shape has
/// been accounted for.
const WIDTH_PER_ROW: f64 = 24.0 / 7.0;

/// Below this the curve stops resolving into two rings.
const MIN_FULL: Size = Size::new(15, 4);

/// Below that, a hand-drawn stand-in, and below *that*, one character.
const SMALL: [&str; 3] = ["▄██▄ ▄██▄", "█  ███  █", "▀██▀ ▀██▀"];

/// The largest half-height of the curve, `1 / (2√2)`.
///
/// The figure is normalised by this so it fills whatever box it is drawn into.
const Y_MAX: f64 = 0.353_553_390_593_273_8;

/// How many points are walked around the curve.
///
/// Far more than there are cells: the curve must come out joined, not dotted.
const STEPS: usize = 4096;

/// How far the stroke spreads from the curve, in cells and half-rows.
const STROKE: [(f64, f64); 9] = [
    (-0.45, -0.225),
    (-0.45, 0.0),
    (-0.45, 0.225),
    (0.0, -0.225),
    (0.0, 0.0),
    (0.0, 0.225),
    (0.45, -0.225),
    (0.45, 0.0),
    (0.45, 0.225),
];

/// Draws the default cover to fill as much of `area` as its shape allows.
pub fn render(area: Size) -> Frame {
    let (cols, rows) = glyph_size(area);

    if Size::new(cols, rows).fits(MIN_FULL) {
        return curve(cols, rows);
    }
    if area.cols >= SMALL[0].chars().count() as u16 && area.rows >= SMALL.len() as u16 {
        return preset();
    }
    if area.cols >= 1 && area.rows >= 1 {
        // Not even the stand-in will fit: one character still says what this is.
        let mut frame = Frame::new(1, 1);
        frame.set(0, 0, Cell::solid('∞', Ink::Accent));
        return frame;
    }
    Frame::new(0, 0)
}

/// How far the figure may be drawn from its true proportions.
///
/// A few per cent either way is invisible; what is not invisible is falling
/// back to the stand-in because the ideal width happened to round down a
/// column short of the room there was.
const TOLERANCE: (f64, f64) = (0.8, 1.25);

/// The size the figure is drawn at: the tallest box that fits the area and is
/// still recognisably the right shape.
fn glyph_size(area: Size) -> (u16, u16) {
    for rows in (1..=area.rows).rev() {
        let cols = ((f64::from(rows) * WIDTH_PER_ROW).round() as u16).min(area.cols);
        let ratio = f64::from(cols) / f64::from(rows);
        if (WIDTH_PER_ROW * TOLERANCE.0..=WIDTH_PER_ROW * TOLERANCE.1).contains(&ratio) {
            return (cols, rows);
        }
    }
    (0, 0)
}

/// The stand-in used when the curve would not resolve.
fn preset() -> Frame {
    let mut frame = Frame::new(SMALL[0].chars().count() as u16, SMALL.len() as u16);
    for (y, line) in SMALL.iter().enumerate() {
        for (x, symbol) in line.chars().enumerate() {
            if symbol != ' ' {
                frame.set(x as u16, y as u16, Cell::solid(symbol, Ink::Accent));
            }
        }
    }
    frame
}

/// Walks the curve and marks every cell its stroke passes through.
///
/// A figure eight on its side, which in polar form is `r² = cos 2θ`. Walking it
/// and marking where it lands is what keeps the two rings joined at exactly one
/// crossing however large it is drawn — a character-by-character drawing would
/// have to be redone for every size.
fn curve(cols: u16, rows: u16) -> Frame {
    let width = cols as usize;
    // Two samples per cell, because a cell is drawn in two halves.
    let half_rows = rows as usize * 2;
    let mut marked = vec![false; width * half_rows];

    for step in 0..STEPS {
        let angle = std::f64::consts::TAU * step as f64 / STEPS as f64;
        let (sin, cos) = angle.sin_cos();
        let denominator = 1.0 + sin * sin;
        // The curve in Cartesian form, spanning x in [-1, 1].
        let x = cos / denominator;
        let y = sin * cos / denominator;

        let column = (x + 1.0) / 2.0 * cols as f64;
        // Rows count downwards, the curve's y counts up.
        let row = (1.0 - (y / Y_MAX + 1.0) / 2.0) * half_rows as f64;

        for (dx, dy) in STROKE {
            let (marked_x, marked_y) = (column + dx, row + dy);
            if marked_x < 0.0 || marked_y < 0.0 {
                continue;
            }
            let (marked_x, marked_y) = (marked_x.floor(), marked_y.floor());
            if marked_x >= f64::from(cols) || marked_y >= half_rows as f64 {
                continue;
            }
            marked[marked_y as usize * width + marked_x as usize] = true;
        }
    }

    let mut frame = Frame::new(cols, rows);
    for y in 0..rows as usize {
        for x in 0..width {
            // Which halves of the cell the stroke passes through decides the
            // character: both, the top, the bottom, or neither.
            let upper = marked[(y * 2) * width + x];
            let lower = marked[(y * 2 + 1) * width + x];
            let symbol = match (upper, lower) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => continue,
            };
            frame.set(x as u16, y as u16, Cell::solid(symbol, Ink::Accent));
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_of(frame: &Frame) -> Vec<String> {
        frame.to_string().lines().map(str::to_owned).collect()
    }

    #[test]
    fn the_sign_is_two_rings_with_one_crossing() {
        let frame = curve(24, 7);
        let rows = rows_of(&frame);

        // Two rings: the top row has two solid runs, with a gap between them.
        let top = &rows[0];
        assert!(top.contains("   "), "the rings are not separated: {top:?}");
        // One crossing: the middle row is joined, and the stroke there is
        // narrow — a wide bar would mean the rings met along a seam instead.
        let middle = &rows[3];
        let run = middle.chars().filter(|c| *c != ' ').count();
        assert!(run <= 6, "the crossing is not a crossing: {middle:?}");
        assert!(middle.trim().starts_with('█') || middle.trim().starts_with('▄'));
    }

    #[test]
    fn the_sign_is_symmetric_both_ways() {
        let frame = curve(24, 7);
        let rows = rows_of(&frame);

        for (index, row) in rows.iter().enumerate() {
            let mirrored: String = row.chars().rev().collect();
            assert_eq!(*row, mirrored, "row {index} is not mirrored left to right");
        }

        for (top, bottom) in rows.iter().zip(rows.iter().rev()) {
            let flipped: String = bottom
                .chars()
                .map(|c| match c {
                    '▀' => '▄',
                    '▄' => '▀',
                    other => other,
                })
                .collect();
            assert_eq!(*top, flipped, "the sign is not symmetric top to bottom");
        }
    }

    #[test]
    fn every_drawn_cell_asks_for_the_accent_colour() {
        let frame = curve(24, 7);
        for cell in frame.cells() {
            if cell.symbol == ' ' {
                assert_eq!(cell.fg, Ink::None, "a blank cell was given a colour");
            } else {
                assert_eq!(cell.fg, Ink::Accent);
                assert_eq!(cell.bg, Ink::Accent);
            }
        }
    }

    #[test]
    fn the_sign_fills_a_roomy_area_without_overrunning_it() {
        let frame = render(Size::new(53, 12));
        assert!(frame.width() <= 53 && frame.height() <= 12);
        // As large as its shape allows: 53 / (24/7) = 15.5, so 12 rows.
        assert_eq!(frame.height(), 12);
        assert!(frame.width() >= 24, "the sign is too small: {frame}");
    }

    #[test]
    fn a_small_area_gives_the_stand_in() {
        let frame = render(Size::new(11, 3));
        assert_eq!((frame.width(), frame.height()), (9, 3));
        assert_eq!(frame.to_string().lines().next().unwrap(), "▄██▄ ▄██▄");
    }

    #[test]
    fn an_awkward_area_still_gets_the_real_sign() {
        // 16 / 5 is a little wider than the sign's proportions, but far closer
        // to them than the stand-in is. Rounding the ideal width down must not
        // cost the real thing.
        let frame = render(Size::new(16, 5));
        assert_eq!((frame.width(), frame.height()), (16, 5));
        assert!(frame.to_string().lines().next().unwrap().contains(' '));
    }

    #[test]
    fn a_single_line_gives_a_single_character() {
        let frame = render(Size::new(4, 1));
        assert_eq!((frame.width(), frame.height()), (1, 1));
        assert_eq!(frame.to_string(), "∞");
    }

    #[test]
    fn nowhere_to_draw_gives_nothing() {
        assert!(render(Size::new(0, 0)).is_empty());
        assert!(render(Size::new(4, 0)).is_empty());
    }

    #[test]
    fn the_sign_never_overruns_the_area_it_was_given() {
        for cols in 0..80u16 {
            for rows in 0..30u16 {
                let area = Size::new(cols, rows);
                let frame = render(area);
                assert!(
                    frame.width() <= cols && frame.height() <= rows,
                    "the sign overran a {cols}×{rows} area with {frame}"
                );
            }
        }
    }
}
