//! Application coordination: state ownership and the event loop.
//!
//! This crate is the only place that mutates application state. It projects a
//! read-only view model for the UI and turns every asynchronous source into an
//! event on a single loop.
//!
//! The UI depends on this crate for nothing: the view model and the actions it
//! emits live in `niveus-ui`, so the arrow between the two runs from here to
//! there. What matters is the contract either way — the UI never sees
//! `AppState`, and only this crate changes it.

pub mod app;
pub mod artwork;
pub mod colour;
pub mod event;
pub mod logging;
pub mod paths;
pub mod persistence;
pub mod preflight;
pub mod state;
