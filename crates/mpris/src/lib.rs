//! Desktop integration: letting the desktop environment see and drive Niveus.
//!
//! A terminal player is invisible to the desktop. This crate publishes it on
//! the session bus under the name the desktop looks for, so the media card
//! shows the track that is playing, the audio visualiser that waits for a
//! player to appear finds one, and the media keys work.
//!
//! It is another adapter around something outside the program, like the crate
//! that wraps the player, and it follows the same rule:
//!
//! ```text
//! keys, mpv, network  ──►  AppEvent  ──►  App    (the only place state changes)
//! desktop (D-Bus)     ──►  MprisCommand ──┘
//!                    App  ──►  Snapshot  ──►  MprisService::update
//! ```
//!
//! Control arriving from the desktop is a request, not an instruction: it is
//! put on a channel and the application acts on it exactly as it would act on
//! the matching key press. State going the other way is pushed, so nothing
//! here ever reads the program's state.
//!
//! Nothing is fatal: with no session bus — a bare container, a serial console —
//! [`MprisService::start`] says so and the program carries on without it.

mod command;
mod error;
mod service;
mod snapshot;

pub use command::MprisCommand;
pub use error::MprisError;
pub use service::{MprisService, OBJECT_PATH, SERVICE_NAME};
pub use snapshot::Snapshot;
