//! Playback control backed by an external `mpv` process over JSON IPC.
//!
//! mpv, sockets and the IPC wire protocol are implementation details of this
//! crate and must not leak through its public surface.
//!
//! The shape of a session:
//!
//! ```text
//! Player::launch(socket, options, events)
//!     ↓
//! player.play(url) / pause() / seek_to(…) / set_volume(…)
//!     ↓
//! events: PlayerEvent::Loaded · Position · Duration · Paused · Ended · Exited
//! ```
//!
//! Events arrive on the channel handed to `launch`, which is what keeps this
//! crate free of application state: it reports, it never decides.

mod error;
mod events;
mod ipc;
mod player;

pub use error::PlayerError;
pub use events::PlayerEvent;
pub use player::{player_path, LaunchOptions, Player};
