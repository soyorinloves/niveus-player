//! Raw lyric payload as fetched by a provider.
//!
//! Parsing into timed lines is the lyrics crate's job; this type only carries
//! what the provider found, plus the one judgement the provider has to make
//! before anything can be parsed (SPEC §13.4, §13.7).

/// What a provider got back when it asked for a track's lyrics.
///
/// The three cases are separate variants rather than an `Option` because the
/// interface shows them differently: a translation with no original is a normal
/// outcome that says so, while nothing at all means an instrumental. Collapsing
/// them into `None` would lose the distinction at the provider, where it is
/// still known, and no later layer could recover it.
///
/// "Everything the service returned was placeholder credits" is deliberately
/// folded into [`RawLyrics::Unavailable`]: the interface has one state for it
/// (SPEC §13.4), so the fold happens where the judgement is made — in the
/// provider, which is the only layer that knows what this service's placeholders
/// look like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawLyrics {
    /// The words to sing, as an LRC body.
    ///
    /// The provider drops lines that are metadata rather than lyrics — credits
    /// and the like — before handing the body over (SPEC §13.7). What is left
    /// is untouched: filtering removes lines, it never rewrites them (§13.8).
    Original(String),
    /// The service returned a translation but no original text.
    ///
    /// Translations are never displayed (SPEC §13.7); this variant exists so the
    /// panel can say that the original is missing rather than that the track has
    /// no lyrics.
    TranslationOnly,
    /// Nothing to display: the service returned nothing, or everything it
    /// returned was placeholder content such as `纯音乐，请欣赏`.
    Unavailable,
}

impl RawLyrics {
    /// Wraps a body of lyrics.
    pub fn original(body: impl Into<String>) -> Self {
        RawLyrics::Original(body.into())
    }

    /// The original LRC body, when there is one.
    pub fn body(&self) -> Option<&str> {
        match self {
            RawLyrics::Original(body) => Some(body),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_is_kept_exactly_as_it_arrived() {
        let body = "[00:01.00]晴天\n[00:05.50]故事的小黄花";
        assert_eq!(RawLyrics::original(body).body(), Some(body));
    }

    #[test]
    fn the_cases_without_an_original_have_no_body() {
        assert_eq!(RawLyrics::TranslationOnly.body(), None);
        assert_eq!(RawLyrics::Unavailable.body(), None);
    }

    #[test]
    fn the_three_cases_are_distinguishable() {
        // The point of the enum: these must not compare equal, or the panel
        // cannot tell "only a translation came back" from "no lyrics at all".
        assert_ne!(RawLyrics::Unavailable, RawLyrics::TranslationOnly);
        assert_ne!(RawLyrics::Unavailable, RawLyrics::original(""));
        assert_ne!(RawLyrics::TranslationOnly, RawLyrics::original(""));
    }
}
