//! Album art rendering to terminal cells.
//!
//! Deliberately does not depend on `ratatui` and does not read environment
//! variables: terminal colour capability is passed in as an argument by the
//! caller.
//!
//! ```text
//! CoverKey → load() → Frame → the caller's own cells
//! ```
//!
//! Two pictures come out of this crate. A cover is a picture shown through
//! characters — `▀` with the upper sample as the foreground colour and the
//! lower as the background, so a cell carries two samples. With no colour to
//! show, that degenerates into a solid block, so a character ramp takes over.
//! The default cover, shown when there is no picture at all, is a drawing made
//! of characters rather than a picture: an infinity sign, drawn from the curve
//! itself at whatever size it is given.

mod cover;
mod default;
mod error;
mod frame;
mod geometry;
mod palette;
mod render;

pub use cover::{load, render_bytes, Cache, CoverKey, Options};
pub use default::render as render_default;
pub use error::ArtworkError;
pub use frame::{Cell, Frame, Ink};
pub use geometry::{Size, MIN_CELLS};
pub use palette::ColourDepth;
pub use render::ArtworkMode;
