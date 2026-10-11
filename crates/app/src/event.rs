//! Everything that can happen to the application, on one channel.
//!
//! Key presses, timer ticks, terminal resizes and every asynchronous reply from
//! the network arrive here as data. There is no other way into the state: this
//! is what makes the loop the single place that mutates it.

use crossterm::event::KeyEvent;
use niveus_artwork::{ArtworkError, CoverKey, Frame};
use niveus_mpris::MprisCommand;
use niveus_music_core::{AudioSource, Playlist, PlaylistId, RawLyrics, Song, SongId, User};
use niveus_ncm_provider::{ProviderError, QrLogin, QrLoginState};
use niveus_player::PlayerEvent;

/// A reply from an asynchronous task, or an input to react to.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    /// A periodic nudge, used to expire transient messages.
    Tick,
    /// The terminal changed size.
    Resize,
    /// SIGINT or SIGTERM arrived rather than a key press.
    ///
    /// The interface normally receives Ctrl+C as a key, but a signal still
    /// arrives when the process is stopped from outside, and must be answered
    /// with the same orderly shutdown.
    Interrupt,

    // ---- sign-in ----
    /// The stored cookie was checked.
    SessionRestored(Result<Option<User>, ProviderError>),
    /// A QR code was requested.
    QrStarted(Result<QrLogin, ProviderError>),
    /// A QR code was polled.
    QrPolled(Result<QrLoginState, ProviderError>),
    /// The account behind a confirmed sign-in.
    QrConfirmed(Result<User, ProviderError>),

    // ---- browsing ----
    Playlists(Result<Vec<Playlist>, ProviderError>),
    /// Tracks for a playlist.
    ///
    /// Carries the generation it was issued under: the reply to a playlist the
    /// user has already navigated away from must not be shown, or a fast
    /// browser ends up looking at one playlist's tracks under another's name.
    Songs {
        generation: u64,
        playlist_id: PlaylistId,
        result: Result<Vec<Song>, ProviderError>,
    },
    Likes(Result<Vec<Song>, ProviderError>),

    // ---- playback ----
    /// Something the player noticed.
    Player(PlayerEvent),
    /// The audio source for a track that was asked to play.
    ///
    /// Carries the generation it was issued under: a source that arrives after
    /// the user has moved to another track must not start playing.
    SongUrl {
        generation: u64,
        song: Song,
        result: Result<AudioSource, ProviderError>,
    },
    /// The lyrics for a track that was asked to play.
    ///
    /// Carries the generation it was issued under, for the same reason an audio
    /// source does: lyrics that arrive after the user has moved on belong to the
    /// track they were asked about, not to the one showing now.
    ///
    /// Only the id travels back, since nothing downstream needs the whole track
    /// — a failed fetch names it in the log and no more.
    Lyrics {
        generation: u64,
        song_id: SongId,
        result: Result<RawLyrics, ProviderError>,
    },
    /// A cover, drawn and ready to be put on screen.
    ///
    /// Carries the key it was drawn from, so a picture for a track or a size
    /// the pane has left behind can be recognised and dropped.
    Cover {
        key: CoverKey,
        result: Result<Frame, ArtworkError>,
    },
    /// A control request from the desktop.
    ///
    /// Handled exactly as the matching key press is: the desktop pressing play
    /// and a listener pressing `Space` take the same path through the program.
    Mpris(MprisCommand),
}
