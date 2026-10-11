//! What has been played, in the order it was heard.
//!
//! The HISTORY page lists this and nothing else derives from it: it is a
//! record of what happened, not a queue and not a source of playback. A track
//! that was heard twice appears once, where it was heard last — the page
//! answers "what have I been listening to", and two entries for one song would
//! make it answer a different question.
//!
//! Like the rest of this crate this is pure data with no IO: the app decides
//! when a track counts as played and when the record is written down.

use crate::id::SongId;
use crate::song::Song;
use serde::{Deserialize, Serialize};

/// One track, and when it was heard.
///
/// The whole track is kept rather than just its id: the page has to draw a
/// title, an artist and an album, and re-fetching them on every start would
/// mean the record could not be shown without the network — which is exactly
/// when a user is most likely to look at it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayedSong {
    pub song: Song,
    /// When playback began, in seconds since the Unix epoch.
    ///
    /// Stored as a plain number rather than a formatted date so the crate needs
    /// no calendar dependency; turning it into `MM-DD` is the renderer's job.
    pub played_at: u64,
}

/// The recently-played list, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct History {
    entries: Vec<PlayedSong>,
}

impl History {
    /// How many tracks are kept.
    ///
    /// A cap rather than an unbounded list: this is written into `state.json` on
    /// every exit, and a list that grows without limit would eventually be the
    /// largest thing in the file.
    pub const MAX: usize = 500;

    pub fn new() -> Self {
        Self::default()
    }

    /// Records a track as played now.
    ///
    /// Recording a track already in the list moves it to the front instead of
    /// adding a second entry, so the list stays a set of tracks ordered by when
    /// each was last heard.
    pub fn record(&mut self, song: Song, played_at: u64) {
        self.entries.retain(|entry| entry.song.id != song.id);
        self.entries.insert(0, PlayedSong { song, played_at });
        self.entries.truncate(Self::MAX);
    }

    /// The entries, newest first.
    pub fn entries(&self) -> &[PlayedSong] {
        &self.entries
    }

    /// The tracks alone, newest first.
    pub fn songs(&self) -> impl Iterator<Item = &Song> + '_ {
        self.entries.iter().map(|entry| &entry.song)
    }

    /// The entry for a track, if it is in the list.
    pub fn entry_for(&self, id: SongId) -> Option<&PlayedSong> {
        self.entries.iter().find(|entry| entry.song.id == id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ArtistId;
    use crate::song::Artist;

    fn song(id: u64) -> Song {
        Song {
            id: SongId(id),
            title: format!("song {id}"),
            artists: vec![Artist {
                id: ArtistId(1),
                name: "Artist".into(),
            }],
            album: None,
            duration_ms: None,
        }
    }

    #[test]
    fn the_newest_track_is_first() {
        let mut history = History::new();
        history.record(song(1), 100);
        history.record(song(2), 200);
        history.record(song(3), 300);

        let ids: Vec<u64> = history.songs().map(|s| s.id.get()).collect();
        assert_eq!(ids, vec![3, 2, 1]);
        assert_eq!(history.entries()[0].played_at, 300);
    }

    #[test]
    fn a_track_heard_again_moves_rather_than_duplicates() {
        let mut history = History::new();
        history.record(song(1), 100);
        history.record(song(2), 200);
        history.record(song(1), 300);

        let ids: Vec<u64> = history.songs().map(|s| s.id.get()).collect();
        assert_eq!(ids, vec![1, 2], "the same track was listed twice");
        assert_eq!(history.len(), 2);
        // And the entry kept is the later one: it answers "when did I last
        // hear this".
        assert_eq!(history.entry_for(SongId(1)).unwrap().played_at, 300);
    }

    #[test]
    fn the_list_stops_growing_at_the_cap_and_drops_the_oldest() {
        let mut history = History::new();
        for id in 1..=(History::MAX as u64 + 5) {
            history.record(song(id), id);
        }

        assert_eq!(history.len(), History::MAX);
        // The newest survived; the very first five did not.
        assert_eq!(
            history.entries()[0].song.id,
            SongId(History::MAX as u64 + 5)
        );
        assert!(
            history.entry_for(SongId(5)).is_none(),
            "an old entry was kept"
        );
        assert!(
            history.entry_for(SongId(6)).is_some(),
            "a recent entry was dropped"
        );
    }

    #[test]
    fn an_empty_history_has_nothing_to_show() {
        let history = History::new();
        assert!(history.is_empty());
        assert_eq!(history.entries().len(), 0);
        assert_eq!(history.entry_for(SongId(1)), None);
    }

    #[test]
    fn the_record_survives_a_round_trip_through_json() {
        let mut history = History::new();
        history.record(song(1), 1_700_000_000);
        history.record(song(2), 1_700_000_100);

        let text = serde_json::to_string(&history).unwrap();
        let read_back: History = serde_json::from_str(&text).unwrap();
        assert_eq!(read_back, history);
        assert_eq!(read_back.entries()[0].played_at, 1_700_000_100);
    }

    #[test]
    fn a_file_written_before_a_field_existed_still_reads() {
        // The list is written by an older release without `album` on the track,
        // say: the record must survive it rather than making the whole file
        // unreadable.
        let raw = r#"[{"song":{"id":7,"title":"T","artists":[]},"played_at":42}]"#;
        let history: History = serde_json::from_str(raw).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history.entries()[0].song.id, SongId(7));
        assert!(history.entries()[0].song.album.is_none());
    }
}
