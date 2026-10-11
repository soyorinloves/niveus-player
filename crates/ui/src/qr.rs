//! Turning a QR payload into something drawable in a terminal.
//!
//! Encoding is a pure transformation, done once when a sign-in starts; drawing
//! the result is the renderer's job.

use qrcode::{EcLevel, QrCode};
use thiserror::Error;

/// Blank modules kept around the code, as scanners expect.
const QUIET_ZONE: usize = 4;

/// Error-correction level.
///
/// The lowest level is deliberate: a code shown on a clean screen suffers no
/// smudging or wear, so spending a third of the symbol on redundancy would only
/// make the code bigger — and size is the binding constraint on a terminal,
/// where the code has to fit above the instructions in eighty columns and
/// twenty-four rows.
const ERROR_CORRECTION: EcLevel = EcLevel::L;

#[derive(Debug, Error)]
pub enum QrError {
    #[error("cannot render the QR code: {0}")]
    Encode(#[from] qrcode::types::QrError),
}

/// A QR code as a square grid of modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrGrid {
    /// Side length in modules, quiet zone included.
    size: usize,
    /// Row-major; `true` is a dark module.
    modules: Vec<bool>,
}

impl QrGrid {
    /// Side length in modules.
    pub fn size(&self) -> usize {
        self.size
    }

    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x]
    }

    /// Columns needed when one cell holds one module.
    pub fn cell_width(&self) -> usize {
        self.size
    }

    /// Rows needed when one cell holds two modules.
    ///
    /// The upper half-block carries two samples per cell, so the grid halves
    /// vertically. An odd module count rounds up, leaving a light half.
    pub fn cell_height(&self) -> usize {
        self.size.div_ceil(2)
    }
}

/// Encodes `payload` into a grid, quiet zone included.
pub fn encode(payload: &str) -> Result<QrGrid, QrError> {
    let code = QrCode::with_error_correction_level(payload.as_bytes(), ERROR_CORRECTION)?;
    let width = code.width();
    let size = width + QUIET_ZONE * 2;
    let mut modules = vec![false; size * size];

    for (index, colour) in code.to_colors().into_iter().enumerate() {
        let x = index % width;
        let y = index / width;
        if colour == qrcode::Color::Dark {
            let (px, py) = (x + QUIET_ZONE, y + QUIET_ZONE);
            modules[py * size + px] = true;
        }
    }

    Ok(QrGrid { size, modules })
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://music.163.com/login?codekey=899d9f4c-c55f-4a01-a10d-6229f4570edb";

    #[test]
    fn the_grid_includes_a_quiet_zone_on_all_four_sides() {
        let grid = encode(URL).unwrap();
        // The quiet zone is light all the way round.
        for i in 0..grid.size() {
            for edge in [0, 1, QUIET_ZONE - 1] {
                assert!(!grid.is_dark(i, edge), "top row {edge} is not blank");
                assert!(!grid.is_dark(edge, i), "left column {edge} is not blank");
                assert!(!grid.is_dark(i, grid.size() - 1 - edge));
                assert!(!grid.is_dark(grid.size() - 1 - edge, i));
            }
        }
    }

    #[test]
    fn the_quiet_zone_size_matches_the_standard() {
        let grid = encode(URL).unwrap();
        let code_width = QrCode::with_error_correction_level(URL.as_bytes(), ERROR_CORRECTION)
            .unwrap()
            .width();
        assert_eq!(grid.size(), code_width + QUIET_ZONE * 2);
    }

    #[test]
    fn a_login_payload_stays_small_enough_for_the_smallest_terminal() {
        // The row budget is the tightest constraint: the code is drawn two
        // module rows per cell, and the sign-in screen must still fit above the
        // instructions.
        let grid = encode(URL).unwrap();
        assert!(
            grid.cell_height() <= 21,
            "a login code needs {} rows",
            grid.cell_height()
        );
        assert!(grid.size() <= 41, "a login code is {} columns", grid.size());
    }

    #[test]
    fn the_grid_is_not_blank() {
        let grid = encode(URL).unwrap();
        let dark = (0..grid.size())
            .flat_map(|y| (0..grid.size()).map(move |x| (x, y)))
            .filter(|(x, y)| grid.is_dark(*x, *y))
            .count();
        assert!(dark > 0, "every module came back light");
        assert!(
            dark < grid.size() * grid.size(),
            "every module came back dark"
        );
    }

    #[test]
    fn out_of_range_lookups_are_light_rather_than_a_panic() {
        let grid = encode(URL).unwrap();
        assert!(!grid.is_dark(grid.size(), 0));
        assert!(!grid.is_dark(0, grid.size()));
    }

    #[test]
    fn a_cell_holds_two_module_rows() {
        let grid = encode(URL).unwrap();
        assert_eq!(grid.cell_width(), grid.size());
        assert_eq!(grid.cell_height(), grid.size().div_ceil(2));
    }

    #[test]
    fn encoding_is_deterministic() {
        assert_eq!(encode(URL).unwrap(), encode(URL).unwrap());
    }
}
