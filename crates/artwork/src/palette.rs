//! Reducing a picture's colours to what the terminal can show.
//!
//! Terminal colour comes in four depths, and each one reduces a true-colour
//! sample somewhere along the way. The reductions are deliberately separate
//! functions rather than one "pick the nearest colour" call, because the
//! sixteen-colour case is not the first sixteen entries of the 256-colour
//! palette — those are whatever the user's theme says they are.

use crate::frame::Ink;

/// How much colour the terminal can be trusted with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColourDepth {
    /// Full colour; the terminal is asked for exact values.
    #[default]
    TrueColor,
    /// The 256-entry palette.
    Colors256,
    /// The sixteen classic colours, drawn with the classic escape sequences.
    Colors16,
    /// No colour at all.
    Monochrome,
}

impl ColourDepth {
    /// The colour to draw an exact sample in.
    pub fn ink(self, rgb: (u8, u8, u8)) -> Ink {
        match self {
            ColourDepth::TrueColor => Ink::Rgb(rgb.0, rgb.1, rgb.2),
            ColourDepth::Colors256 => Ink::Indexed(to_256(rgb)),
            ColourDepth::Colors16 => Ink::Classic(to_16(rgb)),
            ColourDepth::Monochrome => Ink::None,
        }
    }

    /// Whether colour can be drawn at all.
    pub fn has_colour(self) -> bool {
        self != ColourDepth::Monochrome
    }
}

/// How bright a sample is, 0.0 to 1.0.
///
/// The weights are the usual ones for perceived brightness: the eye is far more
/// sensitive to green than to blue, and the character ramp would look wrong if
/// the three channels counted equally.
pub fn luminance(rgb: (u8, u8, u8)) -> f64 {
    let (r, g, b) = rgb;
    (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) / 255.0
}

/// The six levels each channel of the 256-colour cube is quantised to.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The sixteen classic colours, as the reference values a terminal theme is
/// expected to be recognisably close to.
const CLASSIC: [(u8, u8, u8); 16] = [
    (0, 0, 0),       // black
    (205, 0, 0),     // red
    (0, 205, 0),     // green
    (205, 205, 0),   // yellow
    (0, 0, 238),     // blue
    (205, 0, 205),   // magenta
    (0, 205, 205),   // cyan
    (229, 229, 229), // white
    (127, 127, 127), // bright black
    (255, 0, 0),     // bright red
    (0, 255, 0),     // bright green
    (255, 255, 0),   // bright yellow
    (92, 92, 255),   // bright blue
    (255, 0, 255),   // bright magenta
    (0, 255, 255),   // bright cyan
    (255, 255, 255), // bright white
];

/// The 256-colour palette entry closest to an exact colour.
pub fn to_256(rgb: (u8, u8, u8)) -> u8 {
    let cube = (
        nearest_level(rgb.0),
        nearest_level(rgb.1),
        nearest_level(rgb.2),
    );
    let cube_rgb = (
        CUBE_LEVELS[cube.0 as usize],
        CUBE_LEVELS[cube.1 as usize],
        CUBE_LEVELS[cube.2 as usize],
    );

    // The palette also holds a finer 24-step grey ramp, which reproduces greys
    // far better than the six-level cube can.
    let grey_step = (((luminance(rgb) * 255.0) - 8.0) / 10.0)
        .round()
        .clamp(0.0, 23.0) as u8;
    let grey = 8 + 10 * grey_step;

    if distance(rgb, (grey, grey, grey)) < distance(rgb, cube_rgb) {
        232 + grey_step
    } else {
        16 + 36 * cube.0 + 6 * cube.1 + cube.2
    }
}

/// The classic colour closest to an exact colour.
///
/// The aim in this depth is the overall cast — which hue, how light or dark —
/// rather than an exact match, which sixteen colours cannot give.
pub fn to_16(rgb: (u8, u8, u8)) -> u8 {
    let mut best = 0;
    let mut best_distance = i32::MAX;
    for (index, candidate) in CLASSIC.iter().enumerate() {
        let d = distance(rgb, *candidate);
        if d < best_distance {
            best_distance = d;
            best = index as u8;
        }
    }
    best
}

fn nearest_level(channel: u8) -> u8 {
    let mut best = 0;
    let mut best_distance = i32::MAX;
    for (index, level) in CUBE_LEVELS.iter().enumerate() {
        let d = (i32::from(channel) - i32::from(*level)).abs();
        if d < best_distance {
            best_distance = d;
            best = index as u8;
        }
    }
    best
}

/// Squared distance between two colours, weighted towards green.
fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> i32 {
    let dr = i32::from(a.0) - i32::from(b.0);
    let dg = i32::from(a.1) - i32::from(b.1);
    let db = i32::from(a.2) - i32::from(b.2);
    2 * dr * dr + 4 * dg * dg + db * db
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn true_colour_is_passed_straight_through() {
        assert_eq!(
            ColourDepth::TrueColor.ink((12, 34, 56)),
            Ink::Rgb(12, 34, 56)
        );
    }

    #[test]
    fn monochrome_has_no_colour_at_all() {
        assert!(!ColourDepth::Monochrome.has_colour());
        assert_eq!(ColourDepth::Monochrome.ink((255, 0, 0)), Ink::None);
    }

    #[test]
    fn shades_of_grey_use_the_grey_ramp() {
        // The cube can only make six greys; the ramp makes twenty-four, so a
        // mid grey that is not on the cube must land in the ramp.
        let index = to_256((128, 128, 128));
        assert!(
            (232..=255).contains(&index),
            "{index} is not in the grey ramp"
        );
    }

    #[test]
    fn the_cube_is_used_for_colours_that_are_not_grey() {
        assert_eq!(to_256((255, 0, 0)), 196);
        assert_eq!(to_256((0, 255, 0)), 46);
        assert_eq!(to_256((0, 0, 255)), 21);
        assert_eq!(to_256((0, 0, 0)), 16);
        assert_eq!(to_256((255, 255, 255)), 231);
    }

    #[test]
    fn the_sixteen_colour_palette_keeps_the_hue() {
        // Every one of the classic colours must map to itself, or a picture
        // would shift shade in this depth for no reason.
        for (index, colour) in CLASSIC.iter().enumerate() {
            assert_eq!(to_16(*colour), index as u8, "{colour:?} moved");
        }
    }

    #[test]
    fn the_sixteen_colour_palette_keeps_light_and_dark_apart() {
        // This depth cannot be exact, but it must not invert the picture: a
        // dark colour has to come back darker than a light one.
        let dark = CLASSIC[to_16((20, 20, 60)) as usize];
        let light = CLASSIC[to_16((250, 245, 235)) as usize];
        assert!(
            luminance(dark) < luminance(light),
            "{dark:?} is not darker than {light:?}"
        );
    }

    #[test]
    fn brightness_follows_the_eye_rather_than_the_channels() {
        // Green reads as much brighter than blue at the same level.
        assert!(luminance((0, 255, 0)) > luminance((0, 0, 255)));
        assert_eq!(luminance((0, 0, 0)), 0.0);
        assert!((luminance((255, 255, 255)) - 1.0).abs() < 1e-9);
    }
}
