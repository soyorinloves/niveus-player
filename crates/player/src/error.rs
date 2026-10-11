//! Failure modes of playback control.
//!
//! Deliberately free of any mention of mpv, sockets or JSON: the caller answers
//! a `PlayerError`, not an IPC failure, and swapping the player must not change
//! what callers match on.

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("mpv was not found; playback needs mpv, so install it with your distribution's package manager")]
    MpvNotFound,
    #[error("cannot start the player: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("cannot connect to the player: {0}")]
    IpcConnection(#[source] std::io::Error),
    #[error("player communication failed: {0}")]
    IpcProtocol(String),
    #[error("the player refused the command: {0}")]
    Rejected(String),
    #[error("the player has exited")]
    Closed,
}
