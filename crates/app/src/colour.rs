//! Working out how much colour the terminal can show.
//!
//! This lives here rather than in the crate that draws the cover because
//! deciding it means reading the environment, and that crate deliberately does
//! not: it is handed the answer instead.

use niveus_artwork::ColourDepth;
use niveus_config::ColorMode;

/// What the terminal can be trusted with, given the user's preference and how
/// the terminal describes itself.
pub fn detect(setting: ColorMode) -> ColourDepth {
    resolve(
        setting,
        std::env::var("NO_COLOR").is_ok_and(|value| !value.is_empty()),
        std::env::var("COLORTERM").ok(),
        std::env::var("TERM").ok(),
    )
}

/// The rule, with the environment passed in so it can be checked directly.
///
/// The first thing that matches decides: an explicit setting first, then the
/// environment, and the most specific of the terminal's own claims before the
/// least.
fn resolve(
    setting: ColorMode,
    no_color: bool,
    colorterm: Option<String>,
    term: Option<String>,
) -> ColourDepth {
    match setting {
        ColorMode::Truecolor => return ColourDepth::TrueColor,
        ColorMode::Colors256 => return ColourDepth::Colors256,
        ColorMode::Colors16 => return ColourDepth::Colors16,
        ColorMode::Monochrome => return ColourDepth::Monochrome,
        // An override is exactly that; the user asked for this.
        ColorMode::Auto => {}
    }

    if no_color {
        return ColourDepth::Monochrome;
    }

    if let Some(value) = colorterm {
        let lowered = value.to_ascii_lowercase();
        if lowered == "truecolor" || lowered == "24bit" {
            return ColourDepth::TrueColor;
        }
    }

    match term.as_deref() {
        Some(value) if value.contains("256color") => ColourDepth::Colors256,
        // A terminal that says it is dumb, or says nothing at all, is not to be
        // trusted with colour.
        Some("dumb") | None | Some("") => ColourDepth::Monochrome,
        Some(_) => ColourDepth::Colors16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto() -> ColorMode {
        ColorMode::Auto
    }

    #[test]
    fn an_explicit_setting_wins_over_everything_the_terminal_says() {
        // Even a terminal claiming true colour cannot talk the user out of a
        // setting they made on purpose.
        for (setting, expected) in [
            (ColorMode::Truecolor, ColourDepth::TrueColor),
            (ColorMode::Colors256, ColourDepth::Colors256),
            (ColorMode::Colors16, ColourDepth::Colors16),
            (ColorMode::Monochrome, ColourDepth::Monochrome),
        ] {
            assert_eq!(
                resolve(
                    setting,
                    true,
                    Some("truecolor".into()),
                    Some("xterm-256color".into())
                ),
                expected,
                "{setting:?}"
            );
        }
    }

    #[test]
    fn no_colour_is_asked_for_is_no_colour_given() {
        assert_eq!(
            resolve(auto(), true, Some("truecolor".into()), Some("xterm".into())),
            ColourDepth::Monochrome
        );
    }

    #[test]
    fn a_terminal_that_claims_true_colour_gets_it() {
        for value in ["truecolor", "24bit", "TRUECOLOR"] {
            assert_eq!(
                resolve(auto(), false, Some(value.into()), Some("xterm".into())),
                ColourDepth::TrueColor,
                "COLORTERM={value}"
            );
        }
    }

    #[test]
    fn the_palette_is_the_next_thing_to_look_for() {
        assert_eq!(
            resolve(auto(), false, None, Some("xterm-256color".into())),
            ColourDepth::Colors256
        );
        // COLORTERM without true colour does not upgrade a 256-colour terminal.
        assert_eq!(
            resolve(
                auto(),
                false,
                Some("yes".into()),
                Some("screen-256color".into())
            ),
            ColourDepth::Colors256
        );
    }

    #[test]
    fn anything_else_gets_the_sixteen_classic_colours() {
        assert_eq!(
            resolve(auto(), false, None, Some("xterm".into())),
            ColourDepth::Colors16
        );
        assert_eq!(
            resolve(auto(), false, None, Some("linux".into())),
            ColourDepth::Colors16
        );
    }

    #[test]
    fn a_terminal_with_nothing_to_say_gets_no_colour() {
        assert_eq!(resolve(auto(), false, None, None), ColourDepth::Monochrome);
        assert_eq!(
            resolve(auto(), false, None, Some("dumb".into())),
            ColourDepth::Monochrome
        );
        // An empty TERM is a variable that was set and never filled in, which
        // is no more of a claim than not setting it.
        assert_eq!(
            resolve(auto(), false, None, Some(String::new())),
            ColourDepth::Monochrome
        );
    }

    #[test]
    fn the_rule_agrees_with_whatever_this_machine_says() {
        // The environment decides the answer; what is worth pinning down is
        // that asking twice gives the same thing and nothing panics.
        assert_eq!(detect(ColorMode::Auto), detect(ColorMode::Auto));
        assert_eq!(detect(ColorMode::Monochrome), ColourDepth::Monochrome);
    }
}
