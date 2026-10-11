//! Playback state, audio quality and resolved audio sources.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

/// A quality tier Niveus is willing to request.
///
/// The upstream service exposes more tiers (hires and the various spatial
/// formats). Those stay inside the provider and are never offered to the user
/// in the first release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioQuality {
    Standard,
    Higher,
    #[default]
    Exhigh,
    Lossless,
}

impl AudioQuality {
    /// The four tiers selectable in Settings, in ascending order.
    pub const SELECTABLE: [AudioQuality; 4] = [
        AudioQuality::Standard,
        AudioQuality::Higher,
        AudioQuality::Exhigh,
        AudioQuality::Lossless,
    ];

    /// Parameter value expected by `song_url_v1`.
    pub fn as_api_level(self) -> &'static str {
        match self {
            AudioQuality::Standard => "standard",
            AudioQuality::Higher => "higher",
            AudioQuality::Exhigh => "exhigh",
            AudioQuality::Lossless => "lossless",
        }
    }

    /// Parses a level reported by the service.
    ///
    /// Returns `None` for tiers Niveus does not model, so callers can show `—`
    /// instead of inventing a tier that was never requested.
    pub fn from_api_level(level: &str) -> Option<Self> {
        match level {
            "standard" => Some(AudioQuality::Standard),
            "higher" => Some(AudioQuality::Higher),
            "exhigh" => Some(AudioQuality::Exhigh),
            "lossless" => Some(AudioQuality::Lossless),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        self.as_api_level()
    }

    /// The tier `delta` steps away, wrapping round.
    ///
    /// The order the Settings page steps through is the order the tiers are
    /// graded in, so it is kept next to the tiers themselves rather than in
    /// whichever screen happens to change them.
    pub fn step(self, delta: isize) -> Self {
        let all = Self::SELECTABLE;
        let index = all.iter().position(|tier| *tier == self).unwrap_or(0) as isize;
        all[(index + delta).rem_euclid(all.len() as isize) as usize]
    }
}

impl fmt::Display for AudioQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A resolved, playable audio resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSource {
    /// The URL handed to the player.
    pub url: String,
    /// The level the service actually served.
    ///
    /// This is the level of the returned resource, not the level that was
    /// requested: a request for `lossless` by a non-subscriber is served at a
    /// lower tier, and the UI must show what is actually playing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<AudioQuality>,
    /// Container format reported by the service, e.g. `mp3` or `flac`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Bitrate in bits per second, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u32>,
    /// Whether this is a short preview clip rather than the full track.
    ///
    /// The service serves a trial excerpt when the account may not stream the
    /// track in full; the UI has to say so rather than let the track end early
    /// for no visible reason.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_trial: bool,
}

/// Runtime playback state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackState {
    pub status: PlaybackStatus,
    /// Playback position in milliseconds.
    pub position_ms: u64,
    /// Length of the current track in milliseconds, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Output volume, 0–100.
    pub volume: u8,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            position_ms: 0,
            duration_ms: None,
            volume: 72,
        }
    }
}

impl PlaybackState {
    pub fn is_playing(&self) -> bool {
        self.status == PlaybackStatus::Playing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_quality_is_exhigh_and_it_is_selectable() {
        assert_eq!(AudioQuality::default(), AudioQuality::Exhigh);
        assert!(AudioQuality::SELECTABLE.contains(&AudioQuality::default()));
        assert_eq!(AudioQuality::SELECTABLE.len(), 4);
    }

    #[test]
    fn api_levels_round_trip() {
        for quality in AudioQuality::SELECTABLE {
            assert_eq!(
                AudioQuality::from_api_level(quality.as_api_level()),
                Some(quality)
            );
        }
    }

    #[test]
    fn unmodelled_levels_are_reported_as_unknown() {
        assert_eq!(AudioQuality::from_api_level("hires"), None);
        assert_eq!(AudioQuality::from_api_level("jymaster"), None);
        assert_eq!(AudioQuality::from_api_level(""), None);
    }

    #[test]
    fn a_trial_clip_is_still_a_source_without_a_level() {
        let source = AudioSource {
            url: "https://example.invalid/clip.mp3".into(),
            level: None,
            format: Some("mp3".into()),
            bitrate: None,
            is_trial: true,
        };
        let round_tripped: AudioSource =
            serde_json::from_str(&serde_json::to_string(&source).unwrap()).unwrap();
        assert_eq!(round_tripped, source);
        assert!(round_tripped.is_trial);
    }

    #[test]
    fn an_ordinary_source_omits_the_trial_flag_when_serialised() {
        let source = AudioSource {
            url: "https://example.invalid/full.flac".into(),
            level: Some(AudioQuality::Lossless),
            format: Some("flac".into()),
            bitrate: Some(999_000),
            is_trial: false,
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(!json.contains("is_trial"));
        assert_eq!(serde_json::from_str::<AudioSource>(&json).unwrap(), source);
    }

    #[test]
    fn volume_defaults_to_an_audible_level() {
        let state = PlaybackState::default();
        assert!(!state.is_playing());
        assert_eq!(state.position_ms, 0);
        assert_eq!(state.volume, 72);
    }
}
