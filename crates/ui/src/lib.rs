//! Rendering and key mapping.
//!
//! The UI is a pure function of the read-only view model it is handed: it never
//! reads application state, executes business logic, or mutates anything. It
//! knows nothing about the music service, the player, or lyrics.
//!
//! ```
//! use niveus_ui::{layout, state::Page};
//!
//! assert_eq!(layout::LayoutTier::for_size(120, 40).name(), "Standard");
//! assert_eq!(Page::Playlist.title(), "PLAYLIST");
//! ```

pub mod layout;
pub mod qr;
pub mod render;
pub mod state;
pub mod theme;

pub use input::{map_key, to_key};
pub use layout::{artwork_rows, compute, pane_rows, panel_blocks, LayoutTier, Regions};
pub use qr::{QrError, QrGrid};
pub use render::render;
pub use state::{
    ArtworkCell, ArtworkInk, ArtworkView, ListView, LoginView, LyricWindow, LyricsView,
    NowPlayingView, Page, SessionView, Setting, SettingsValues, StatusView, UiState, SETTINGS_ROWS,
};
pub use theme::Theme;

mod input;

#[cfg(test)]
pub(crate) mod testing;
