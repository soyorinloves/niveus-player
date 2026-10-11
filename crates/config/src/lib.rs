//! User preferences: `config.toml` reading and keymap.
//!
//! Configuration is read once at startup into an immutable [`Config`] and then
//! handed to the other modules; modules never read the config file themselves.

pub mod config;
pub mod keymap;

pub use config::{ArtworkMode, ColorMode, Config, ConfigError, ConfigIssue, CoverSize};
pub use keymap::{Action, Key, Keymap, KeymapError};
