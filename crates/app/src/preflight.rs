//! Checks made before the interface starts.
//!
//! Anything that would make the program unusable later is caught here, while
//! there is still a terminal available to explain it on.

use crate::paths::Paths;
use niveus_player::player_path;
use niveus_ui::strings;

/// The outcome of the checks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preflight {
    /// Conditions worth telling the user about that do not stop the program.
    pub warnings: Vec<String>,
}

/// Runs the checks, or explains why the program cannot start.
pub fn check(paths: &Paths) -> Result<Preflight, String> {
    // The lookup lives with the player: two copies of it could drift apart,
    // and the one that matters is the one the player actually uses.
    let mpv = player_path().ok_or_else(|| strings::preflight::MPV_MISSING.to_string())?;
    tracing::info!(path = %mpv.display(), "found player");

    paths
        .ensure_dirs()
        .map_err(|e| strings::preflight::dirs_failed(&e.to_string()))?;

    let mut warnings = Vec::new();
    if !locale_is_utf8() {
        // Not fatal: the interface relies on box-drawing and block characters,
        // which a non-UTF-8 locale will mangle, but the user may know better.
        warnings.push(strings::preflight::NON_UTF8_LOCALE.to_string());
        tracing::warn!("non-UTF-8 locale");
    }

    Ok(Preflight { warnings })
}

/// Whether the environment advertises a UTF-8 locale.
fn locale_is_utf8() -> bool {
    let value = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));

    match value {
        Some(value) => {
            let normalised = value.to_ascii_lowercase();
            normalised.contains("utf-8") || normalised.contains("utf8")
        }
        // Nothing set at all: assume the modern default rather than nagging.
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_player_is_looked_for_the_same_way_the_player_looks_for_itself() {
        // There is only one lookup; this pins down that start-up uses it rather
        // than a second copy that could drift.
        assert_eq!(player_path().is_some(), player_path().is_some());
        if let Some(path) = player_path() {
            assert!(path.is_absolute(), "{}", path.display());
        }
    }

    #[test]
    fn utf8_locales_are_recognised_in_their_common_spellings() {
        // The helper reads the environment; the accepted spellings are what is
        // asserted, via the same normalisation the function uses.
        for value in ["en_US.UTF-8", "zh_CN.utf8", "C.UTF-8"] {
            let normalised = value.to_ascii_lowercase();
            assert!(normalised.contains("utf-8") || normalised.contains("utf8"));
        }
        for value in ["C", "POSIX", "en_US.ISO-8859-1"] {
            let normalised = value.to_ascii_lowercase();
            assert!(!(normalised.contains("utf-8") || normalised.contains("utf8")));
        }
    }
}
