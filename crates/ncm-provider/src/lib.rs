//! Music service adapter: maps NetEase API data onto Niveus domain types.
//!
//! No `niveus-ncm-api` type appears in this crate's public surface.

mod auth;
mod error;
mod lyric_text;
mod mapping;
mod provider;

pub use auth::{QrLogin, QrLoginState};
pub use error::{ApiFailure, ProviderError, Result};
pub use provider::NcmProvider;

use niveus_music_core::{AudioQuality, AudioSource, Playlist, PlaylistId, RawLyrics, Song, SongId};
use std::future::Future;

/// The music service, as the rest of the program sees it.
///
/// Returning `impl Future + Send` rather than a bare `async fn` keeps callers
/// free to spawn these onto the runtime without knowing the concrete provider.
pub trait MusicProvider: Send + Sync {
    /// Every playlist the user owns or has collected.
    ///
    /// Created and collected playlists are distinguished by
    /// [`Playlist::kind`]; the caller decides which to show where.
    fn get_user_playlists(&self) -> impl Future<Output = Result<Vec<Playlist>>> + Send;

    /// Every track of one playlist, in playlist order.
    fn get_playlist_songs(
        &self,
        playlist_id: PlaylistId,
    ) -> impl Future<Output = Result<Vec<Song>>> + Send;

    /// The user's "liked songs" list.
    fn get_liked_songs(&self) -> impl Future<Output = Result<Vec<Song>>> + Send;

    /// A playable source for a track.
    ///
    /// The returned source reports the tier actually served, which may be lower
    /// than the tier requested.
    fn get_song_url(
        &self,
        song_id: SongId,
        level: AudioQuality,
    ) -> impl Future<Output = Result<AudioSource>> + Send;

    /// Raw lyrics for a track.
    fn get_lyrics(&self, song_id: SongId) -> impl Future<Output = Result<RawLyrics>> + Send;
}
