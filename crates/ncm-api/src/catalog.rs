//! Catalogue and playback endpoints: playlists, songs, audio URLs, lyrics.

use crate::client::NcmApiClient;
use crate::dto::*;
use crate::error::{ApiError, Result};
use serde::Deserialize;

/// Page size used when walking a paginated endpoint.
const PAGE_SIZE: u32 = 1000;
/// Upper bound on pages walked, so a misbehaving service cannot spin forever.
const MAX_PAGES: u32 = 20;
/// The song-detail endpoint takes ids as one comma-separated parameter, so
/// large requests are split into batches.
const SONG_ID_BATCH: usize = 200;

#[derive(Deserialize)]
struct PlaylistPage {
    #[serde(default)]
    playlist: Vec<UserPlaylistDto>,
}

#[derive(Deserialize)]
struct SongList {
    #[serde(default)]
    songs: Vec<SongDto>,
}

#[derive(Deserialize)]
struct SongUrlList {
    #[serde(default)]
    data: Vec<SongUrlDto>,
}

impl NcmApiClient {
    /// Every playlist owned by or subscribed to by `uid`.
    ///
    /// The endpoint paginates, so this walks pages until the service returns a
    /// short one.
    pub async fn user_playlists(&self, uid: u64) -> Result<Vec<UserPlaylistDto>> {
        let mut all = Vec::new();
        for page in 0..MAX_PAGES {
            let offset = page * PAGE_SIZE;
            let query = self
                .query()
                .param("uid", &uid.to_string())
                .param("limit", &PAGE_SIZE.to_string())
                .param("offset", &offset.to_string());

            let response = self
                .inner
                .user_playlist(&query)
                .await
                .map_err(|e| Self::fail("user_playlist", e))?;
            self.record(&response);

            let parsed: PlaylistPage = Self::decode("user_playlist", response.body)?;
            let received = parsed.playlist.len();
            all.extend(parsed.playlist);

            if received < PAGE_SIZE as usize {
                break;
            }
        }
        Ok(all)
    }

    /// Songs of a playlist, in playlist order.
    ///
    /// `playlist_detail` only returns the first few tracks, so the track-list
    /// endpoint is used instead.
    pub async fn playlist_tracks(
        &self,
        playlist_id: u64,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SongDto>> {
        let query = self
            .query()
            .param("id", &playlist_id.to_string())
            .param("limit", &limit.to_string())
            .param("offset", &offset.to_string());

        let response = self
            .inner
            .playlist_track_all(&query)
            .await
            .map_err(|e| Self::fail("playlist_track_all", e))?;
        self.record(&response);

        let parsed: SongList = Self::decode("playlist_track_all", response.body)?;
        Ok(parsed.songs)
    }

    /// Details for specific songs, batched to keep the request URL sane.
    pub async fn song_details(&self, ids: &[u64]) -> Result<Vec<SongDto>> {
        let mut all = Vec::with_capacity(ids.len());
        for batch in ids.chunks(SONG_ID_BATCH) {
            let joined = batch
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let query = self.query().param("ids", &joined);

            let response = self
                .inner
                .song_detail(&query)
                .await
                .map_err(|e| Self::fail("song_detail", e))?;
            self.record(&response);

            let parsed: SongList = Self::decode("song_detail", response.body)?;
            all.extend(parsed.songs);
        }
        Ok(all)
    }

    /// Ids of the tracks in the user's "liked songs" list.
    pub async fn liked_song_ids(&self, uid: u64) -> Result<Vec<u64>> {
        let query = self.query().param("uid", &uid.to_string());
        let response = self
            .inner
            .likelist(&query)
            .await
            .map_err(|e| Self::fail("likelist", e))?;
        self.record(&response);

        let parsed: LikelistDto = Self::decode("likelist", response.body)?;
        Ok(parsed.ids)
    }

    /// Resolves a playable URL.
    ///
    /// Returns `Ok(None)` when the service answers normally but has nothing to
    /// play — a track this account may not stream comes back as HTTP 200 with a
    /// null URL. That is a different situation from a malformed response, and
    /// the caller needs to tell them apart to choose between "cannot play this
    /// track" and "something is wrong".
    pub async fn song_url(&self, song_id: u64, level: &str) -> Result<Option<SongUrlDto>> {
        let query = self
            .query()
            .param("id", &song_id.to_string())
            .param("level", level);

        let response = self
            .inner
            .song_url_v1(&query)
            .await
            .map_err(|e| Self::fail("song_url_v1", e))?;
        self.record(&response);

        let parsed: SongUrlList = Self::decode("song_url_v1", response.body)?;
        let entry = parsed.data.into_iter().next().ok_or_else(|| {
            ApiError::InvalidResponse("song_url_v1 returned no audio source".to_string())
        })?;

        if entry.url.as_deref().unwrap_or("").trim().is_empty() {
            tracing::debug!(
                song_id,
                code = ?entry.code,
                "service reports no playable source for this track"
            );
            return Ok(None);
        }
        Ok(Some(entry))
    }

    /// Raw lyrics for a song.
    pub async fn lyrics(&self, song_id: u64) -> Result<LyricsDto> {
        let query = self.query().param("id", &song_id.to_string());
        let response = self
            .inner
            .lyric(&query)
            .await
            .map_err(|e| Self::fail("lyric", e))?;
        self.record(&response);

        Self::decode("lyric", response.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playlist_page_parses_and_tolerates_an_empty_body() {
        let raw = r#"{"code":200,"playlist":[{"id":1,"name":"a","userId":7,"specialType":0}]}"#;
        let page: PlaylistPage = serde_json::from_str(raw).unwrap();
        assert_eq!(page.playlist.len(), 1);
        assert_eq!(page.playlist[0].user_id, Some(7));

        let page: PlaylistPage = serde_json::from_str(r#"{"code":200}"#).unwrap();
        assert!(page.playlist.is_empty());
    }

    #[test]
    fn a_missing_playlist_key_is_not_a_parse_error() {
        // The service returns a bare error payload for an unknown id; the empty
        // default keeps that from looking like a malformed response.
        let page: PlaylistPage = serde_json::from_str(r#"{"code":404,"msg":"nope"}"#).unwrap();
        assert!(page.playlist.is_empty());
    }

    #[test]
    fn a_song_list_parses_from_the_shared_wire_shape() {
        let raw = r#"{"code":200,"songs":[{"id":1,"name":"x","ar":[],"dt":1000}]}"#;
        let list: SongList = serde_json::from_str(raw).unwrap();
        assert_eq!(list.songs.len(), 1);
        assert_eq!(list.songs[0].dt, Some(1000));
    }

    #[test]
    fn a_null_url_entry_is_recognised_as_unplayable() {
        let raw = r#"{"code":200,"data":[{"id":1,"url":null,"code":404}]}"#;
        let list: SongUrlList = serde_json::from_str(raw).unwrap();
        assert!(list.data[0].url.is_none());
    }
}
