//! Songs, artists and albums.

use crate::id::{AlbumId, ArtistId, SongId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub id: ArtistId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    pub id: AlbumId,
    pub name: String,
    /// Cover art URL, already carrying the provider's sizing parameter.
    ///
    /// `None` when the track has no usable cover; artwork falls back to the
    /// default `∞` rendering in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Song {
    pub id: SongId,
    pub title: String,
    pub artists: Vec<Artist>,
    /// `None` when the track has no album information.
    ///
    /// The upstream service can return tracks without album metadata; the
    /// distinction matters because the UI renders `—` rather than an empty
    /// name in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<Album>,
    /// Duration in milliseconds, `None` when unknown. The UI renders `—`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
}

impl Song {
    /// Artists joined for display; `—` when the track carries none.
    pub fn artist_names(&self) -> String {
        if self.artists.is_empty() {
            return String::from("—");
        }
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub fn album_name(&self) -> &str {
        self.album.as_ref().map(|a| a.name.as_str()).unwrap_or("—")
    }

    pub fn cover_url(&self) -> Option<&str> {
        self.album.as_ref().and_then(|a| a.cover_url.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song_with(artists: Vec<Artist>, album: Option<Album>) -> Song {
        Song {
            id: SongId(1),
            title: "Title".into(),
            artists,
            album,
            duration_ms: None,
        }
    }

    #[test]
    fn missing_metadata_falls_back_to_placeholder() {
        let song = song_with(Vec::new(), None);
        assert_eq!(song.artist_names(), "—");
        assert_eq!(song.album_name(), "—");
        assert_eq!(song.cover_url(), None);
    }

    #[test]
    fn multiple_artists_are_joined() {
        let song = song_with(
            vec![
                Artist {
                    id: ArtistId(1),
                    name: "A".into(),
                },
                Artist {
                    id: ArtistId(2),
                    name: "B".into(),
                },
            ],
            None,
        );
        assert_eq!(song.artist_names(), "A / B");
    }

    #[test]
    fn album_metadata_is_optional_through_serde() {
        let raw = r#"{"id":1,"title":"T","artists":[]}"#;
        let song: Song = serde_json::from_str(raw).unwrap();
        assert!(song.album.is_none());
        assert!(song.duration_ms.is_none());
    }
}
