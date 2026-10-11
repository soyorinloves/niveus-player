//! What the desktop is told about what is playing.

use niveus_music_core::{PlaybackStatus, Song};

/// The identity of the track being played, as the bus wants it.
///
/// An object path rather than a number: the interface says a track id is a
/// D-Bus object path, and a path segment may not begin with a digit, so the id
/// is prefixed.
pub fn track_path(song: Option<&Song>) -> String {
    match song {
        Some(song) => format!("/org/niveus/track/s{}", song.id.get()),
        // The path the interface names for "there is no track".
        None => "/org/mpris/MediaPlayer2/TrackList/NoTrack".to_string(),
    }
}

/// Everything the desktop needs to draw its media card.
///
/// Pushed by the application whenever any of it changes; nothing here is ever
/// read back, which is what keeps the arrow one-way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub status: PlaybackStatus,
    /// The track playing now, if any.
    pub song: Option<Song>,
    /// How far into the track playback has got, in milliseconds.
    pub position_ms: u64,
    /// Output volume, 0–100.
    pub volume: u8,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            song: None,
            position_ms: 0,
            volume: 72,
        }
    }
}

impl Snapshot {
    /// The position in microseconds, which is what the interface asks for.
    pub fn position_us(&self) -> i64 {
        (self.position_ms as i64) * 1000
    }

    /// The volume as a fraction, which is what the interface asks for.
    pub fn volume_fraction(&self) -> f64 {
        f64::from(self.volume.clamp(0, 100)) / 100.0
    }

    /// The name the interface uses for the playback state.
    pub fn playback_status(&self) -> &'static str {
        match self.status {
            PlaybackStatus::Playing => "Playing",
            PlaybackStatus::Paused => "Paused",
            PlaybackStatus::Stopped => "Stopped",
        }
    }

    /// Whether the desktop is told it may seek.
    ///
    /// Seeking is meaningless with nothing loaded, and the interface asks for
    /// this to be honest.
    pub fn can_seek(&self) -> bool {
        self.song.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niveus_music_core::{Artist, ArtistId, SongId};

    fn song(id: u64) -> Song {
        Song {
            id: SongId(id),
            title: format!("song {id}"),
            artists: vec![Artist {
                id: ArtistId(1),
                name: "artist".into(),
            }],
            album: None,
            duration_ms: None,
        }
    }

    #[test]
    fn a_track_id_is_a_path_a_bus_will_accept() {
        // A path segment may hold letters, digits and underscores, but may not
        // begin with a digit — which is why the id is prefixed.
        let path = track_path(Some(&song(12)));
        assert_eq!(path, "/org/niveus/track/s12");
        assert!(path
            .chars()
            .skip(1)
            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_'));
        for segment in path.trim_start_matches('/').split('/') {
            assert!(
                !segment.starts_with(|c: char| c.is_ascii_digit()),
                "{segment} starts with a digit"
            );
        }
    }

    #[test]
    fn no_track_uses_the_path_the_interface_names() {
        assert_eq!(
            track_path(None),
            "/org/mpris/MediaPlayer2/TrackList/NoTrack"
        );
    }

    #[test]
    fn times_are_reported_in_microseconds() {
        // The interface counts microseconds; everything inside Niveus counts
        // milliseconds, and getting this wrong is a factor of a thousand.
        let snapshot = Snapshot {
            position_ms: 97_000,
            ..Snapshot::default()
        };
        assert_eq!(snapshot.position_us(), 97_000_000);
    }

    #[test]
    fn the_volume_is_reported_as_a_fraction() {
        // The interface counts 0.0–1.0; the interface's own readout counts
        // 0–100.
        let volume = |percent| Snapshot {
            volume: percent,
            ..Snapshot::default()
        };
        assert_eq!(volume(0).volume_fraction(), 0.0);
        assert_eq!(volume(72).volume_fraction(), 0.72);
        assert_eq!(volume(100).volume_fraction(), 1.0);
    }

    #[test]
    fn the_playback_state_uses_the_interfaces_own_words() {
        let named = |status| Snapshot {
            status,
            ..Snapshot::default()
        };
        assert_eq!(named(PlaybackStatus::Playing).playback_status(), "Playing");
        assert_eq!(named(PlaybackStatus::Paused).playback_status(), "Paused");
        assert_eq!(named(PlaybackStatus::Stopped).playback_status(), "Stopped");
    }

    #[test]
    fn seeking_is_offered_only_when_there_is_something_to_seek_in() {
        let mut snapshot = Snapshot::default();
        assert!(!snapshot.can_seek());
        snapshot.song = Some(song(1));
        assert!(snapshot.can_seek());
    }
}
