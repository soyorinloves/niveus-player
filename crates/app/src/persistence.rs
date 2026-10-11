//! `state.json`: what the program was doing, written down.
//!
//! Preferences live in `config.toml` and are the user's to decide; everything
//! here is what the program was doing when it last stopped — which playlist was
//! open, what was playing, where in it, and what has been heard. They are kept
//! apart because they are answered by different questions (SPEC §28, §29):
//! "what should this program do" against "what was it doing".
//!
//! Nothing in this file is authoritative. Every entry is a starting point, and
//! a file that cannot be read, or that names a playlist that no longer exists,
//! is answered by starting from the top rather than by refusing to run.

use niveus_music_core::{History, PlaybackState, PlaylistId, Queue, SongId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

/// Why a state file could not be used.
///
/// Reading is allowed to fail quietly — the program starts fresh — but never
/// silently: the reason goes to the log, because "my queue is gone again" is
/// impossible to diagnose otherwise.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("无法读取状态文件: {0}")]
    Read(#[source] std::io::Error),
    #[error("状态文件解析失败: {0}")]
    Parse(#[source] serde_json::Error),
    #[error("无法写入状态文件: {0}")]
    Write(#[source] std::io::Error),
}

/// Where the cursor was left in a list.
///
/// The same shape the application keeps in memory, written down under a name
/// that says which list it belongs to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorState {
    #[serde(default)]
    pub selected: usize,
    #[serde(default)]
    pub scroll: usize,
}

/// The volume a fresh installation starts at.
///
/// Named rather than defaulted to zero: a missing entry in a hand-edited file
/// must not arrive as silence.
fn default_volume() -> u8 {
    PlaybackState::default().volume
}

/// The whole of `state.json`.
///
/// Every field is optional on the way in, so a file written by an older release
/// — or edited by hand — still yields something usable (SPEC §29).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateFile {
    /// The playlist that was open, if one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_playlist_id: Option<PlaylistId>,
    /// The track that was playing, if one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_song_id: Option<SongId>,
    /// How far into that track playback had got, in milliseconds.
    #[serde(default)]
    pub position_ms: u64,
    /// Output volume, 0–100.
    #[serde(default = "default_volume")]
    pub volume: u8,
    /// The queue itself: its tracks, where it had got to, and its mode.
    #[serde(default)]
    pub queue: Queue,
    /// What has been heard, newest first.
    #[serde(default)]
    pub history: History,
    /// Where the cursor was left in each browsing list, by list name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cursors: BTreeMap<String, CursorState>,
}

/// A fresh start.
///
/// Written out rather than derived, because a derived default would answer
/// "volume" with zero: the two defaults are the same thing whether the value
/// is missing from a file or from a program that has never run, and both have
/// to be the level a new installation listens at.
impl Default for StateFile {
    fn default() -> Self {
        Self {
            current_playlist_id: None,
            current_song_id: None,
            position_ms: 0,
            volume: default_volume(),
            queue: Queue::default(),
            history: History::default(),
            cursors: BTreeMap::new(),
        }
    }
}

impl StateFile {
    /// Reads `state.json`, and says why if it could not be used.
    ///
    /// A file that is not there is the ordinary first run and is not reported.
    /// A file that is there but unreadable is reported and treated as absent:
    /// losing a queue is a small thing, refusing to start is not.
    pub fn load(path: &Path) -> (Self, Option<StateError>) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %path.display(), "no state file, starting fresh");
                return (Self::default(), None);
            }
            Err(error) => return (Self::default(), Some(StateError::Read(error))),
        };

        match serde_json::from_str(&text) {
            Ok(state) => (state, None),
            Err(error) => (Self::default(), Some(StateError::Parse(error))),
        }
    }

    /// Writes the state out.
    ///
    /// Written to a neighbouring temporary file and moved into place, so a
    /// crash part-way through leaves the previous state intact rather than a
    /// truncated file. This is the one file whose whole purpose is to survive a
    /// crash, so half a write is the one outcome it must not have.
    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StateError::Write)?;
        }

        let mut text = serde_json::to_string_pretty(self).map_err(StateError::Parse)?;
        text.push('\n');

        let scratch = path.with_extension("json.tmp");
        std::fs::write(&scratch, text).map_err(StateError::Write)?;
        std::fs::rename(&scratch, path).map_err(|error| {
            // The scratch file is of no use to anyone once the move failed.
            let _ = std::fs::remove_file(&scratch);
            StateError::Write(error)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niveus_music_core::{Artist, ArtistId, PlaybackMode, Song};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("niveus-state-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("state.json")
    }

    fn song(id: u64) -> Song {
        Song {
            id: SongId(id),
            title: format!("song {id}"),
            artists: vec![Artist {
                id: ArtistId(1),
                name: "Artist".into(),
            }],
            album: None,
            duration_ms: Some(200_000),
        }
    }

    #[test]
    fn a_missing_file_is_a_first_run_and_not_an_error() {
        let path = scratch("missing");
        let (state, error) = StateFile::load(&path);
        assert_eq!(state, StateFile::default());
        assert!(error.is_none(), "{error:?}");
        // And the default is not silence.
        assert_eq!(state.volume, 72);
    }

    #[test]
    fn what_was_written_is_what_is_read_back() {
        let path = scratch("round-trip");
        let mut state = StateFile {
            current_playlist_id: Some(PlaylistId(42)),
            current_song_id: Some(SongId(7)),
            position_ms: 61_500,
            volume: 33,
            ..StateFile::default()
        };
        state.queue.replace_items(vec![song(1), song(2), song(3)]);
        state.queue.select(1);
        state.queue.playback_mode = PlaybackMode::Shuffle;
        state.queue.push_next(song(9));
        state.history.record(song(1), 1_700_000_000);
        state.cursors.insert(
            "playlist:42".into(),
            CursorState {
                selected: 5,
                scroll: 3,
            },
        );

        state.save(&path).unwrap();
        let (read_back, error) = StateFile::load(&path);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(read_back, state);

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_file_on_disk_is_readable_json() {
        // It is named `.json` and users do look at it.
        let path = scratch("readable");
        let mut state = StateFile::default();
        state.queue.replace_items(vec![song(1)]);
        state.save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["position_ms"], 0);
        assert_eq!(parsed["queue"]["items"][0]["title"], "song 1");
        assert!(text.ends_with('\n'), "the file does not end in a newline");
        // Nothing written means nothing in the file: an absent playlist must
        // not appear as a null the reader has to know about.
        assert!(parsed.get("current_playlist_id").is_none(), "{text}");

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_damaged_file_starts_fresh_and_says_why() {
        let path = scratch("damaged");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ this is not json").unwrap();

        let (state, error) = StateFile::load(&path);
        assert_eq!(state, StateFile::default());
        assert!(matches!(error, Some(StateError::Parse(_))), "{error:?}");

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_file_with_only_some_of_the_fields_still_reads() {
        // An older release wrote less, or a user deleted a line. What is there
        // is used and the rest falls back rather than the whole file failing.
        let path = scratch("partial");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"current_playlist_id": 5, "queue": {"items": [{"id": 3, "title": "T", "artists": []}]}}"#,
        )
        .unwrap();

        let (state, error) = StateFile::load(&path);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(state.current_playlist_id, Some(PlaylistId(5)));
        assert_eq!(state.queue.items.len(), 1);
        assert_eq!(state.queue.playback_mode, PlaybackMode::Sequential);
        assert_eq!(state.volume, 72, "a missing volume became silence");
        assert!(state.history.is_empty());

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn writing_leaves_no_scratch_file_behind() {
        let path = scratch("scratch");
        StateFile::default().save(&path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());

        // A second write replaces the first rather than merging with it.
        let mut state = StateFile::default();
        state.queue.replace_items(vec![song(1)]);
        state.save(&path).unwrap();
        let (read_back, _) = StateFile::load(&path);
        assert_eq!(read_back.queue.items.len(), 1);

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_playlist_id_is_not_written_when_there_is_none() {
        // Otherwise every file carries a `null` that a reader has to know to
        // ignore, and "which playlist was open" reads as "none was".
        let path = scratch("no-playlist");
        StateFile::default().save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("current_playlist_id"), "{text}");
        assert!(!text.contains("current_song_id"), "{text}");
        assert!(!text.contains("cursors"), "{text}");

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_fresh_start_listens_at_the_default_level() {
        // A derived `Default` would answer zero, and a missing entry in a
        // hand-edited file would arrive as silence.
        assert_eq!(StateFile::default().volume, 72);
        assert_eq!(StateFile::default().volume, PlaybackState::default().volume);
    }
}
