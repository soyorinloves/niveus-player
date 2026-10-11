//! Isolation layer around the upstream `ncm-api-rs` crate.
//!
//! Nothing outside this crate may name an `ncm-api-rs` type, and the DTOs in
//! [`dto`] stop at `ncm-provider`'s boundary. Callers get plain data plus
//! [`ApiError`].

mod auth;
mod catalog;
mod client;
mod dto;
mod error;

pub use auth::{QrLogin, QrLoginState, SignedInUser};
pub use client::NcmApiClient;
pub use error::{ApiError, Result};

/// Re-exported for the provider's use; see the module documentation.
pub use dto::{
    AlbumDto, ArtistDto, LyricsDto, SongDto, SongUrlDto, UserPlaylistDto, SPECIAL_TYPE_LIKED,
};
