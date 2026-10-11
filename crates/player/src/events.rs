//! What the player reports back.

/// Something the player noticed, sent to the application as it happens.
///
/// These are the only way playback state changes reach the application: the
/// player never writes application state itself.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// A track was loaded and playback is about to begin.
    Loaded,
    /// The track reached its natural end.
    ///
    /// Only a genuine end of file produces this. Stopping, seeking away,
    /// replacing the track and quitting the player are all silent, so that
    /// nothing can trigger an unintended advance to the next track.
    Ended,
    /// Playback stopped because the file could not be played.
    Failed(String),
    /// Playback position of the current track, in seconds.
    Position(f64),
    /// Duration of the current track, in seconds.
    Duration(f64),
    /// Whether the current track is paused.
    Paused(bool),
    /// Output volume, 0–100.
    Volume(f64),
    /// The player process ended.
    Exited,
}
