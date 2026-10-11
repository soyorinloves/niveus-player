//! The one hard-coded colour scheme.
//!
//! The first release has no theme system; these styles are shared by every view
//! so the interface reads as one piece. Only the standard sixteen terminal
//! colours are used, so the result stays legible on both dark and light
//! terminals.
//!
//! Album art is the deliberate exception and brings its own colour-capability
//! ladder; nothing here constrains it.

use ratatui::style::{Color, Modifier, Style};

/// Semantic roles rather than colour names, so views ask for "emphasis" or
/// "muted" and the mapping stays in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Unstyled body text.
    pub normal: Style,
    /// Highest visual weight: the line being sung, the selected row's label.
    pub emphasis: Style,
    /// Clearly de-emphasised: adjacent lyric lines, secondary metadata.
    pub muted: Style,
    /// Identity colour for the current item, the progress bar, and accents.
    pub accent: Style,
    /// Section labels such as `CREATED` and `NOW PLAYING`.
    pub heading: Style,
    /// Horizontal rules between sections.
    pub divider: Style,
    /// Failure notices.
    pub error: Style,
}

impl Default for Theme {
    fn default() -> Self {
        let accent = Style::default().fg(Color::Cyan);
        Self {
            normal: Style::default(),
            emphasis: Style::default().add_modifier(Modifier::BOLD),
            muted: Style::default().add_modifier(Modifier::DIM),
            accent,
            heading: accent.add_modifier(Modifier::BOLD),
            divider: Style::default().add_modifier(Modifier::DIM),
            error: Style::default().fg(Color::Red),
        }
    }
}

impl Theme {
    /// Style for a row the cursor is on.
    pub fn selected(&self) -> Style {
        self.accent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_standard_colours_are_used() {
        let theme = Theme::default();
        for style in [
            theme.normal,
            theme.emphasis,
            theme.muted,
            theme.accent,
            theme.heading,
            theme.divider,
            theme.error,
        ] {
            if let Some(colour) = style.fg {
                assert!(
                    matches!(
                        colour,
                        Color::Black
                            | Color::Red
                            | Color::Green
                            | Color::Yellow
                            | Color::Blue
                            | Color::Magenta
                            | Color::Cyan
                            | Color::Gray
                            | Color::DarkGray
                            | Color::LightRed
                            | Color::LightGreen
                            | Color::LightYellow
                            | Color::LightBlue
                            | Color::LightMagenta
                            | Color::LightCyan
                            | Color::White
                            | Color::Reset
                    ),
                    "colour {colour:?} is outside the sixteen-colour palette"
                );
            }
        }
    }

    #[test]
    fn emphasis_and_muted_are_visually_distinct() {
        let theme = Theme::default();
        assert_ne!(theme.emphasis, theme.muted);
        assert_eq!(theme.heading.fg, theme.accent.fg);
    }
}
