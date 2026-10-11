//! Niveus domain model.
//!
//! This crate is the dependency leaf of the workspace: it must not depend on any
//! other Niveus crate, and it contains no IO, no async and no network code.

pub mod history;
pub mod id;
pub mod lyrics;
pub mod playback;
pub mod playlist;
pub mod queue;
pub mod song;

pub use history::{History, PlayedSong};
pub use id::{AlbumId, ArtistId, PlaylistId, SongId, UserId};
pub use lyrics::RawLyrics;
pub use playback::{AudioQuality, AudioSource, PlaybackState, PlaybackStatus};
pub use playlist::{Playlist, PlaylistKind, User};
pub use queue::{PlaybackMode, Queue};
pub use song::{Album, Artist, Song};
