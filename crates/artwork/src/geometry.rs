//! How a picture is fitted into a region of terminal cells.
//!
//! The one thing that must not be got wrong here is the cell's shape. A
//! terminal cell is about twice as tall as it is wide, so a region of cells is
//! not a rectangle of square pixels: fitting a square picture into it without
//! compensating leaves the picture stretched to twice its height.

/// A rectangle of terminal cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Size {
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }

    /// How many vertical samples a cell of this region holds, by mode.
    ///
    /// Half-block drawing uses both halves of a cell, so it gets two; the
    /// character ramp uses the cell as one sample.
    pub fn samples(self, per_cell: u16) -> (u16, u16) {
        (self.cols, self.rows.saturating_mul(per_cell))
    }

    /// Whether this size is at least `other` in both directions.
    pub const fn fits(self, other: Size) -> bool {
        self.cols >= other.cols && self.rows >= other.rows
    }
}

/// How many cells tall an area must be for a picture to be worth drawing.
///
/// Below this a cover degenerates into a few coloured blocks that say less than
/// the empty space would.
pub const MIN_CELLS: u16 = 4;

/// How many vertical samples one terminal cell holds.
///
/// The cell is about twice as tall as it is wide, which is exactly what the
/// half-block character relies on: its two halves are square samples.
pub const CELL_ASPECT: f64 = 2.0;

/// The cells a picture of `pixels` should occupy inside `area`.
///
/// The result keeps the picture's proportions and never exceeds `area`. `None`
/// when the area is too small to draw anything worth drawing.
///
/// This answers a question about *cells*, and the answer is the same whichever
/// drawing mode is used: a cell is one column wide and two rows tall either
/// way, so a picture covers the same cells and the same physical area in both.
/// What differs is the *sample* grid inside those cells — two samples per cell
/// for half-blocks, one for the character ramp, as [`Size::samples`] gives.
///
/// Conflating the two is the mistake to avoid: the half-block sample grid is
/// twice as tall as the cell rectangle, and using it as a cell rectangle for
/// the character ramp draws the picture twice its proper height.
pub fn fit(pixels: (u32, u32), area: Size) -> Option<Size> {
    if area.cols < MIN_CELLS || area.rows < MIN_CELLS {
        return None;
    }
    let (width, height) = pixels;
    if width == 0 || height == 0 {
        return None;
    }

    let aspect = width as f64 / height as f64;

    // A cell is one wide and two tall, so the widest a picture can be drawn in
    // `rows` cells is `rows * 2 * aspect`, and the tallest it can be drawn in
    // `cols` cells is `cols / (2 * aspect)`.
    let sample_rows = (area.cols as f64 / (CELL_ASPECT * aspect)).floor();
    let rows = (sample_rows.min(f64::from(area.rows)) as u16).max(1);
    let cols = ((rows as f64 * CELL_ASPECT * aspect).round() as u16)
        .min(area.cols)
        .max(1);

    // Rounding can leave room for one more row; take it, so a picture is not
    // drawn a cell smaller than it needs to be.
    let rows = (((cols as f64 / (CELL_ASPECT * aspect)).floor()) as u16)
        .min(area.rows)
        .max(1);

    Some(Size::new(cols, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference case from the design: a square cover in a 40×12 region
    /// occupies about 24×12 cells. Rendering it 40 wide would be a 40×24-cell
    /// picture — a square stretched to more than three times its height.
    #[test]
    fn a_square_cover_fills_the_height_of_its_region() {
        let fitted = fit((600, 600), Size::new(40, 12)).unwrap();
        assert_eq!(fitted, Size::new(24, 12));
    }

    #[test]
    fn the_two_modes_share_a_cell_rectangle_but_not_a_sample_grid() {
        // The cell rectangle is the same either way — a cell is the same shape
        // whichever way it is drawn.
        let cells = fit((600, 600), Size::new(40, 12)).unwrap();
        assert_eq!(cells, Size::new(24, 12));

        // The sample grids are not. Drawing the ramp on the half-block grid
        // would make it 24 cells tall instead of 12, twice its proper height.
        assert_eq!(cells.samples(2), (24, 24));
        assert_eq!(cells.samples(1), (24, 12));
    }

    #[test]
    fn a_wide_cover_leaves_the_bottom_of_its_region_empty() {
        // 3:1 into 40×12: six cells tall is all the height there is, and at
        // that height the picture is 6 * 2 * 3 = 36 columns wide — three fifths
        // of the width would be the picture stretched, not fitted.
        let fitted = fit((900, 300), Size::new(40, 12)).unwrap();
        assert_eq!(fitted, Size::new(36, 6));
    }

    #[test]
    fn a_tall_cover_leaves_the_sides_of_its_region_empty() {
        // 1:2 into 40×12: twelve cells tall, and half that wide.
        let fitted = fit((300, 600), Size::new(40, 12)).unwrap();
        assert_eq!(fitted, Size::new(12, 12));
    }

    #[test]
    fn the_fitted_picture_never_exceeds_its_region() {
        for (w, h) in [(1, 1), (600, 600), (1920, 1080), (300, 1200), (1200, 300)] {
            for (cols, rows) in [(4, 4), (10, 4), (40, 12), (80, 24), (17, 9)] {
                let area = Size::new(cols, rows);
                let fitted = fit((w, h), area).expect("a drawable region");
                assert!(
                    fitted.cols <= cols && fitted.rows <= rows,
                    "{w}x{h} did not fit in {cols}x{rows}: {fitted:?}"
                );
                assert!(fitted.cols > 0 && fitted.rows > 0);
            }
        }
    }

    #[test]
    fn a_region_too_small_to_be_worth_drawing_is_refused() {
        assert_eq!(fit((600, 600), Size::new(3, 12)), None);
        assert_eq!(fit((600, 600), Size::new(40, 3)), None);
        assert_eq!(fit((600, 600), Size::new(0, 0)), None);
    }

    #[test]
    fn a_picture_with_no_pixels_is_refused() {
        assert_eq!(fit((0, 600), Size::new(40, 12)), None);
        assert_eq!(fit((600, 0), Size::new(40, 12)), None);
    }
}
