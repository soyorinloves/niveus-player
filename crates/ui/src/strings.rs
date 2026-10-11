//! Every word the interface puts on screen, in one place.
//!
//! The interface is written in English for the first release; Chinese returns
//! later, and doing it by translation means these strings are the only thing
//! that has to move (SPEC §39.2). They are grouped by where they are read, not
//! by crate: navigation, the list pane, the now-playing pane, the status row,
//! sign-in, the size warning, and the messages the application raises while it
//! works.
//!
//! What does **not** belong here (SPEC §39.3):
//!
//! * log lines — for developers, not users;
//! * the `thiserror` variants of the lower crates — internal, and turned into
//!   user copy only when the application wraps one in one of the sentences
//!   below;
//! * test fixtures — data, kept verbatim (SPEC §39.4);
//! * the strings a service returns — data, shown as they came (SPEC §39.5).

/// Top-level navigation and its secondary items.
pub mod nav {
    pub const LIBRARY: &str = "LIBRARY";
    pub const SEARCH: &str = "SEARCH";
    pub const QUEUE: &str = "QUEUE";
    pub const SETTINGS: &str = "SETTINGS";

    pub const LIKED: &str = "LIKED";
    pub const PLAYLISTS: &str = "PLAYLISTS";
    pub const HISTORY: &str = "HISTORY";
    pub const CURRENT: &str = "CURRENT";
    pub const UP_NEXT: &str = "UP NEXT";
    pub const PLAYBACK: &str = "PLAYBACK";
    pub const APPEARANCE: &str = "APPEARANCE";
    pub const KEYBINDINGS: &str = "KEYBINDINGS";
}

/// The browsable pane: headings, empty notes and the rows that are not entries.
pub mod list {
    /// The heading over the tracks of an opened playlist.
    pub const CURRENT_PLAYLIST: &str = "CURRENT PLAYLIST";
    /// The groups the playlist list is split into.
    pub const GROUP_CREATED: &str = "CREATED";
    pub const GROUP_COLLECTED: &str = "COLLECTED";

    /// Shown in place of a list that has nothing in it and is not loading.
    pub const EMPTY_PLAYLISTS: &str = "No playlists yet";
    pub const EMPTY_SONGS: &str = "This playlist has no songs";
    pub const EMPTY_LIKED: &str = "No liked songs yet";
    pub const EMPTY_HISTORY: &str = "Nothing has been played yet";
    pub const EMPTY_QUEUE: &str = "The queue is empty";
    pub const EMPTY_UP_NEXT: &str = "Nothing is queued to play next";

    /// Marks the row of the track playing now in the queue.
    pub const PLAYING: &str = "PLAYING";

    /// The prompt a row shows while it waits for the key to bind.
    pub const BINDING_KEY: &str = "Press a new key…";
}

/// The right-hand pane.
pub mod now_playing {
    pub const TITLE: &str = "NOW PLAYING";
    /// A caption, not a control: turning the metadata off is what shows them.
    pub const LYRICS: &str = "LYRICS";
    /// The switch, drawn with the key that works it.
    pub const METADATA: &str = "METADATA";

    /// The five metadata rows (SPEC §14).
    pub const METADATA_ALBUM: &str = "ALBUM";
    pub const METADATA_DURATION: &str = "DURATION";
    pub const METADATA_QUALITY: &str = "QUALITY";
    pub const METADATA_FORMAT: &str = "FORMAT";
    pub const METADATA_SONG_ID: &str = "SONG ID";

    /// Only a translation came back; there is no original text.
    pub const TRANSLATION_ONLY: &str = "No original lyrics";
    /// No original text at all: an instrumental, or a track with none.
    pub const INSTRUMENTAL: &str = "♪ Instrumental";
    /// There are lyrics, but they could not be fetched or parsed.
    pub const FAILED: &str = "Lyrics unavailable";
}

/// The bottom row.
pub mod status {
    pub const PLAYING: &str = "PLAYING";
    pub const PAUSED: &str = "PAUSED";
    pub const STOPPED: &str = "STOPPED";

    pub const VOLUME: &str = "VOL";
    pub const KBPS: &str = "KBPS";
    pub const QUEUE: &str = "QUEUE";
    pub const PAUSE: &str = "PAUSE";
    pub const PLAY: &str = "PLAY";
    pub const LOADING: &str = "LOADING…";

    /// Short reminders of what the current view responds to.
    pub const FOCUS: &str = "FOCUS";
    pub const MOVE: &str = "MOVE";
    pub const OPEN: &str = "OPEN";
    pub const BACK: &str = "BACK";
    pub const ORDER: &str = "ORDER";
    pub const DELETE: &str = "DEL";
    pub const EDIT: &str = "EDIT";
    pub const REBIND: &str = "REBIND";
    pub const QUIT: &str = "QUIT";
}

/// The sign-in screen.
pub mod login {
    pub const HEADING: &str = "SIGN IN";
    pub const INSTRUCTION: &str = "Scan the QR code with the NetEase Cloud Music app";
    pub const CHECKING: &str = "Checking your session…";
    pub const STARTING: &str = "Requesting a QR code…";
    pub const WAITING: &str = "Waiting for scan…";
    pub const SCANNED: &str = "Scanned — confirm on your phone";
    pub const TOO_SMALL: &str = "Terminal too small to show the QR code; enlarge the window";

    /// The way out, with the key that quits appending to it.
    pub fn quit(key: &str) -> String {
        format!("{key} Quit")
    }
}

/// The warning shown when the terminal cannot hold the interface.
pub mod usage {
    pub const TITLE: &str = "Terminal size too small";

    pub fn current(width: u16, height: u16) -> String {
        format!("current: {width}×{height}")
    }

    pub fn needed(width: u16, height: u16) -> String {
        format!("needed:  at least {width}×{height}")
    }
}

/// The sentences the application raises as it works.
///
/// One clause each, with the failure's own words appended where there is one:
/// the message from the lower crate is internal and is never the whole story on
/// its own.
pub mod notice {
    /// A failure that happened before the interface was up, said on stderr.
    pub fn terminal_init(error: &str) -> String {
        format!(
            "Cannot initialize the terminal: {error}. \
             Run niveus directly in a terminal; if it is started from a script, \
             give it a pseudo-terminal (pty)."
        )
    }

    pub fn state_restore_failed(error: &str) -> String {
        format!("Cannot restore the previous session: {error}")
    }

    pub fn desktop_unavailable(error: &str) -> String {
        format!("Desktop integration unavailable: {error}")
    }

    pub fn player_start_failed(error: &str) -> String {
        format!("Cannot start the player: {error}")
    }

    pub const STATE_SAVE_FAILED: &str =
        "The state could not be saved; it will not be restored next time";

    pub const NOT_SIGNED_IN: &str = "Not signed in; cannot play";
    pub const PLAYER_UNAVAILABLE: &str = "The player is unavailable";

    pub fn playback_failed(error: &str) -> String {
        format!("Playback failed: {error}")
    }

    pub fn playback_control_failed(error: &str) -> String {
        format!("Playback control failed: {error}")
    }

    pub fn stop_failed(error: &str) -> String {
        format!("Cannot stop playback: {error}")
    }

    pub fn seek_failed(error: &str) -> String {
        format!("Cannot seek: {error}")
    }

    pub fn volume_failed(error: &str) -> String {
        format!("Cannot change the volume: {error}")
    }

    pub fn cannot_play(title: &str, error: &str) -> String {
        format!("Cannot play \"{title}\": {error}")
    }

    pub const TRIAL_ONLY: &str = "This account can only preview a clip of this track";
    pub const SELECT_A_SONG: &str = "Choose a song first";
    pub const QUEUE_ENDED: &str = "The queue has finished";
    pub const QUEUE_FIRST: &str = "Already at the first track";

    pub fn playback_mode(mode: &str) -> String {
        format!("Playback mode: {mode}")
    }

    pub fn playing_next(title: &str) -> String {
        format!("Playing next: {title}")
    }

    pub const NOTHING_TO_QUEUE: &str = "There is nothing to add to the queue";

    pub const BIND_PROMPT: &str = "Press a key to bind, Esc to cancel";
    pub const BIND_CANCELLED: &str = "Rebinding cancelled";
    pub const BIND_INVALID: &str = "That key cannot be bound; try another";

    /// Announced when a setting's value is changed, naming it and its new
    /// value.
    pub fn setting(label: &str, value: &str) -> String {
        format!("{label}: {value}")
    }

    pub fn bound(action: &str, key: &str) -> String {
        format!("{action} bound to {key}")
    }

    pub fn bind_failed(error: &str) -> String {
        format!("Cannot bind: {error}")
    }

    pub const CONFIG_SAVE_FAILED: &str =
        "The settings could not be saved; the old values will return next time";

    pub fn session_check_failed(error: &str) -> String {
        format!("Cannot verify the session: {error}")
    }

    pub fn qr_encode_failed(error: &str) -> String {
        format!("Cannot draw the QR code: {error}")
    }

    pub fn qr_fetch_failed(error: &str) -> String {
        format!("Cannot fetch the QR code: {error}")
    }

    pub fn qr_scan_failed(error: &str) -> String {
        format!("Scan failed: {error}")
    }

    pub fn sign_in_confirm_failed(error: &str) -> String {
        format!("Sign-in could not be confirmed: {error}")
    }

    /// The three browsing lists, named in the order the failure is reported.
    pub const PLAYLISTS_FAILED: &str = "Cannot load playlists";
    pub const SONGS_FAILED: &str = "Cannot load songs";
    pub const LIKES_FAILED: &str = "Cannot load liked songs";

    pub fn browse_failed(what: &str, error: &str) -> String {
        format!("{what}: {error}")
    }

    pub const SESSION_EXPIRED: &str = "Your session has expired; scan the QR code again";
    pub const PLAYLIST_GONE: &str = "The playlist from last time no longer exists";

    pub const PLAYER_DEAD: &str =
        "The player keeps exiting; playback is disabled. Restart Niveus to restore it";
    pub const PLAYER_RESTARTED: &str = "The player exited and was restarted; press Space to resume";

    pub fn player_restart_failed(error: &str) -> String {
        format!("The player exited and could not be restarted: {error}")
    }

    /// A first-level item that is reserved but not built yet (SPEC §10.1).
    pub const SEARCH_UNIMPLEMENTED: &str = "Search is not available yet";
}

/// What was wrong with `config.toml`, said once the interface is up.
///
/// `config` itself reports these as structured values rather than sentences:
/// the sentence is the interface's, and this is where it is kept.
pub mod config {
    pub fn unreadable(error: &str) -> String {
        format!("Cannot read the config file; using defaults: {error}")
    }

    pub fn unparsable(error: &str) -> String {
        format!("The config file could not be parsed; using defaults: {error}")
    }

    pub fn keybinding_ignored(error: &str) -> String {
        format!("Ignoring a keybinding: {error}")
    }

    pub fn unknown_quality(raw: &str, used: &str) -> String {
        format!("Unknown audio quality \"{raw}\"; using {used}")
    }
}

/// What the start-up checks say, before the interface takes the screen.
pub mod preflight {
    pub const MPV_MISSING: &str = "mpv was not found. Niveus uses mpv to play audio; \
        install it with your distribution's package manager.";

    pub fn dirs_failed(error: &str) -> String {
        format!(
            "Cannot create the config/state directories: {error}. \
             Check that XDG_CONFIG_HOME / XDG_STATE_HOME are writable."
        )
    }

    pub const NON_UTF8_LOCALE: &str =
        "The current locale is not UTF-8; characters such as ─ │ ● ▶ ∞ may not render. \
         Set LANG=en_US.UTF-8.";
}

/// Said on stderr when the executable cannot even get as far as the interface.
pub mod startup {
    pub fn logging_failed(error: &str) -> String {
        format!("niveus: cannot initialize logging: {error}")
    }

    pub fn runtime_failed(error: &str) -> String {
        format!("niveus: cannot start the async runtime: {error}")
    }
}
