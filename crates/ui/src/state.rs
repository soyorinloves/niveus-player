//! The read-only view model.
//!
//! The app projects its internal state into this shape every frame. Borrowed
//! throughout, so building it costs no allocation and the renderer can rely on
//! nothing being mutated underneath it.

use crate::layout::{compute, LayoutTier};
use crate::qr::QrGrid;
use niveus_config::{Action, CoverSize, Keymap};
use niveus_music_core::{
    AudioSource, PlaybackMode, PlaybackStatus, PlayedSong, Playlist, Song, User,
};
use ratatui::layout::Rect;

/// Where each setting sits in that list, so the app knows which one the cursor
/// is on without counting the drawing code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Keybindings,
    AudioQuality,
    PlaybackMode,
    Volume,
    CoverSize,
}

impl Setting {
    /// Every setting, in the order the page lists it, with the name it is
    /// shown under.
    ///
    /// One declaration on purpose: the order, the heading and the number of
    /// rows are all read from here, so adding a setting is one line rather
    /// than a hunt through the renderer and the app for the places that used
    /// to count them (SPEC §35, Phase 10).
    pub const ALL: [(Setting, &'static str); 5] = [
        (Setting::Keybindings, "KEYBINDINGS"),
        (Setting::AudioQuality, "AUDIO QUALITY"),
        (Setting::PlaybackMode, "PLAYBACK MODE"),
        (Setting::Volume, "VOLUME"),
        (Setting::CoverSize, "COVER SIZE"),
    ];

    /// How many settings the page lists.
    pub const COUNT: usize = Self::ALL.len();

    /// The name a setting is listed under.
    pub fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(setting, _)| *setting == self)
            .expect("ALL lists every setting")
            .1
    }

    /// The setting at a position in the list.
    pub fn at(index: usize) -> Option<Setting> {
        Self::ALL.get(index).map(|(setting, _)| *setting)
    }

    /// Where a setting sits in the list.
    pub fn index_of(setting: Setting) -> usize {
        Self::ALL
            .iter()
            .position(|(candidate, _)| *candidate == setting)
            .expect("ALL lists every setting")
    }
}

/// How many entries the settings page lists.
pub const SETTINGS_ROWS: usize = Setting::COUNT;

/// The values the settings page shows on the right of each row.
///
/// A field per setting rather than a list of strings, so a setting that is
/// added without a value to show is a compile error rather than a blank row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsValues<'a> {
    pub quality: &'a str,
    pub mode: &'a str,
    pub volume: u8,
    pub cover_size: &'a str,
}

impl SettingsValues<'_> {
    /// The value shown against a setting.
    pub fn value(&self, setting: Setting) -> String {
        match setting {
            Setting::Keybindings => "›".to_string(),
            Setting::AudioQuality => self.quality.to_string(),
            Setting::PlaybackMode => self.mode.to_string(),
            Setting::Volume => format!("{}%", self.volume),
            Setting::CoverSize => self.cover_size.to_string(),
        }
    }
}

/// A top-level page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Playlist,
    Favorites,
    History,
    Settings,
}

impl Page {
    /// In the order the navigation row lists them.
    pub const ALL: [Page; 4] = [
        Page::Playlist,
        Page::Favorites,
        Page::History,
        Page::Settings,
    ];

    /// The action that switches to this page, so the navigation row can show
    /// the key actually bound to it.
    pub fn action(self) -> Action {
        match self {
            Page::Playlist => Action::ShowPlaylist,
            Page::Favorites => Action::ShowFavorites,
            Page::History => Action::ShowHistory,
            Page::Settings => Action::OpenSettings,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Page::Playlist => "PLAYLIST",
            Page::Favorites => "FAVORITES",
            Page::History => "HISTORY",
            Page::Settings => "SETTINGS",
        }
    }
}

/// The lines the lyric panel is showing.
///
/// An odd number of them, oldest first, with the one being sung in the middle —
/// that is what puts it in the centre of the panel, with the same room above as
/// below. A slot is empty where the song has no line that far away, so the rows
/// stay where they are at the ends of a song instead of shuffling up.
///
/// The window arrives already sized for the room there is: how many lines fit is
/// worked out before it gets here (SPEC §13.1, §13.5).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LyricWindow<'a> {
    lines: Vec<Option<&'a str>>,
}

impl<'a> LyricWindow<'a> {
    /// Wraps the lines of a window, oldest first.
    pub fn new(lines: Vec<Option<&'a str>>) -> Self {
        Self { lines }
    }

    /// The lines, oldest first, with empty slots for what the song does not have.
    pub fn lines(&self) -> &[Option<&'a str>] {
        &self.lines
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The rows this takes drawn: a line each, with a blank row between them.
    pub fn rows(&self) -> usize {
        self.lines.len().saturating_mul(2).saturating_sub(1)
    }

    /// Whether there is anything to draw at all.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The line in a slot, if the song has one there.
    pub fn line(&self, slot: usize) -> Option<&'a str> {
        self.lines.get(slot).copied().flatten()
    }

    /// The slot the line being sung is in: the middle of the window.
    pub fn current_slot(&self) -> usize {
        self.lines.len() / 2
    }

    /// Whether a slot is the line being sung, which is the one drawn at full
    /// weight (SPEC §13.3).
    pub fn is_current(&self, slot: usize) -> bool {
        slot == self.current_slot()
    }
}

/// Whether a session exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionView<'a> {
    Ready { user: &'a User },
    Login(LoginView<'a>),
}

/// The sign-in screen's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginView<'a> {
    /// Checking the session that was stored from last time.
    Checking,
    /// Asking the service for a code.
    Starting,
    /// A code is drawn and being polled.
    Waiting { grid: &'a QrGrid, scanned: bool },
    /// The code was refused, or the service could not be reached.
    Failed { message: &'a str },
}

/// What the browsable pane is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListView<'a> {
    /// First level: playlists, grouped by how the user came to have them.
    Playlists {
        created: &'a [Playlist],
        collected: &'a [Playlist],
        selected: usize,
        scroll: usize,
    },
    /// Second level: the tracks of one playlist.
    Songs {
        playlist_name: &'a str,
        songs: &'a [Song],
        selected: usize,
        scroll: usize,
    },
    /// The user's liked tracks, which have a page of their own rather than
    /// appearing in the grouped playlist list.
    Likes {
        songs: &'a [Song],
        selected: usize,
        scroll: usize,
    },
    History {
        /// The tracks played, newest first, with when each was heard.
        entries: &'a [PlayedSong],
        selected: usize,
        scroll: usize,
    },
    /// The playback queue, which takes the pane over while it is open.
    Queue {
        songs: &'a [Song],
        /// Which of these is playing, when the track playing is one of them.
        ///
        /// A track queued with "play next" is not in the queue, so nothing here
        /// is marked while it plays.
        playing: Option<usize>,
        selected: usize,
        scroll: usize,
    },
    /// The settings group list. Values are shown on the right.
    Settings {
        values: SettingsValues<'a>,
        selected: usize,
        /// The setting being edited right now, drawn so the user can see which
        /// row the arrow keys are changing.
        editing: Option<Setting>,
    },
    /// The keybindings, opened from `KEYBINDINGS`, with the key each action
    /// currently answers to.
    ///
    /// The bindings themselves are read from the keymap the frame is handed,
    /// not carried here: what is drawn is then necessarily what fires
    /// (SPEC §36, rule 8).
    Keybindings {
        selected: usize,
        /// Index of the first action drawn.
        ///
        /// There are more actions than most panes can show at once — the pane
        /// is 18 rows at the Minimal tier — so this is scrolled like any other
        /// list. Without it the cursor can walk off the bottom of the pane,
        /// taking the "waiting for a key" prompt with it, and the user would
        /// rebind a row they cannot see.
        scroll: usize,
        /// Waiting for the user to press the key to bind. A refusal is said on
        /// the status bar, like every other message this program has for the
        /// user, rather than inside a list that can be scrolled away from it.
        capturing: bool,
    },
    /// A note where a list would be.
    Message(&'a str),
}

impl ListView<'_> {
    /// Rows this view spends on things that are not entries.
    ///
    /// The playlist list heads each of its two groups, and puts a blank line
    /// before the second. Whoever works out how many entries fit has to allow
    /// for these, or the window it computes is taller than the one that gets
    /// drawn and the cursor is scrolled off the bottom.
    pub fn fixed_rows(&self) -> usize {
        match self {
            // `CREATED`, a blank line, `COLLECTED` — the most a window can hold.
            ListView::Playlists { .. } => 3,
            _ => 0,
        }
    }

    /// Rows one entry occupies.
    ///
    /// A playlist's tracks carry their title on one row and the artist, album
    /// and duration on the next; everything else is a single row.
    pub fn rows_per_item(&self) -> usize {
        match self {
            ListView::Songs { .. } | ListView::Likes { .. } => 2,
            ListView::Message(_) => 0,
            _ => 1,
        }
    }

    /// How many entries the user can move between.
    pub fn item_count(&self) -> usize {
        match self {
            ListView::Playlists {
                created, collected, ..
            } => created.len() + collected.len(),
            ListView::Songs { songs, .. }
            | ListView::Likes { songs, .. }
            | ListView::Queue { songs, .. } => songs.len(),
            ListView::History { entries, .. } => entries.len(),
            ListView::Settings { .. } => Setting::COUNT,
            // One row per action, since every action has a binding.
            ListView::Keybindings { .. } => Action::ALL.len(),
            ListView::Message(_) => 0,
        }
    }
}

/// A colour one half of an album art cell is drawn in.
///
/// The naming is the view model's own: the crate that produces the picture is
/// not one this crate depends on, so the app hands the picture over in this
/// shape. The distinction between the classic sixteen colours and the palette's
/// first sixteen entries is kept because they are not the same colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkInk {
    Rgb(u8, u8, u8),
    Indexed(u8),
    /// One of the sixteen classic colours, by index.
    Classic(u8),
    /// Whatever the theme's accent colour is.
    Accent,
    None,
}

/// One cell of album art.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtworkCell {
    pub symbol: char,
    /// The colour of the upper half of the cell.
    pub fg: ArtworkInk,
    /// The colour of the lower half.
    pub bg: ArtworkInk,
}

/// A picture to draw in the album art region.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArtworkView {
    width: u16,
    height: u16,
    cells: Vec<ArtworkCell>,
}

impl ArtworkView {
    /// Builds a picture from its cells, row by row.
    ///
    /// Cells beyond the declared size are dropped rather than wrapping, so a
    /// miscounted picture draws short instead of drawing wrongly.
    pub fn new(width: u16, height: u16, cells: Vec<ArtworkCell>) -> Self {
        let mut all = cells;
        all.truncate(width as usize * height as usize);
        all.resize(
            width as usize * height as usize,
            ArtworkCell {
                symbol: ' ',
                fg: ArtworkInk::None,
                bg: ArtworkInk::None,
            },
        );
        Self {
            width,
            height,
            cells: all,
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn cell(&self, x: u16, y: u16) -> Option<&ArtworkCell> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.cells
            .get(y as usize * self.width as usize + x as usize)
    }

    /// Whether there is anything to draw.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// What the lyrics panel is showing.
///
/// The states the design asks to be told apart are all here, and none of them
/// can be worked out from the others: "there is nothing yet", "there is no
/// original text", "there are no words at all" and "the lyrics could not be
/// used" are four different things to say, and the last one is not an error
/// message — the reason stays in the log (SPEC §13.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LyricsView<'a> {
    /// Nothing to show: no track, or its lyrics are still on their way.
    ///
    /// Distinct from the two states below on purpose — claiming a track is
    /// instrumental before the answer arrives would be a guess.
    #[default]
    Pending,
    /// The lines around the one being sung.
    Window(LyricWindow<'a>),
    /// Only a translation came back; there is no original to show.
    TranslationOnly,
    /// No original text: an instrumental, or a track the service had none for.
    Instrumental,
    /// There are lyrics, but they could not be fetched or parsed.
    Failed,
}

/// The right-hand pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowPlayingView<'a> {
    /// The track being played. `None` when nothing is playing.
    pub song: Option<&'a Song>,
    /// Whether the metadata block is showing along the bottom of the panel.
    ///
    /// A switch rather than a choice between two views: the lyrics stay where
    /// they are and the block joins them (SPEC §12.1).
    pub metadata: bool,
    /// How far into the track playback has got.
    pub position_ms: Option<u64>,
    /// How long the track is, as the player reports it.
    ///
    /// Preferred over the length in the track's own metadata, which is what the
    /// player was told but not necessarily what it found in the file.
    pub duration_ms: Option<u64>,
    /// Album art, absent while it is still being drawn.
    pub artwork: Option<&'a ArtworkView>,
    /// What the lyrics panel has to show.
    pub lyrics: LyricsView<'a>,
    /// The resource being played, for the metadata panel.
    ///
    /// This is what is *actually* playing rather than what was asked for, which
    /// is the only honest thing to report: a request for `lossless` on an
    /// account without a subscription is served at a lower tier (SPEC §14).
    pub source: Option<&'a AudioSource>,
}

/// The bottom row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusView<'a> {
    /// A request is in flight.
    pub loading: bool,
    /// A transient message: a failure, or something the user needs to know.
    pub notice: Option<&'a str>,
    pub volume: u8,
    pub playback_mode: PlaybackMode,
    pub queue_len: usize,
    /// Whether playback is running, paused, or has not started.
    ///
    /// Three states rather than two: "paused" and "stopped" are not the same
    /// thing to a user who has just pressed the pause key.
    pub playback: PlaybackStatus,
    /// The bitrate of the resource playing, in bits per second.
    ///
    /// Absent until something is playing, and for sources that do not report
    /// one; the segment is left out rather than filled with a dash, because a
    /// row of dashes reads as a fault where nothing is wrong.
    pub bitrate: Option<u32>,
}

/// Everything the renderer is allowed to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiState<'a> {
    pub page: Page,
    pub session: SessionView<'a>,
    pub list: ListView<'a>,
    pub now_playing: NowPlayingView<'a>,
    pub status: StatusView<'a>,
    /// Read for key hints, so what is printed is what is bound.
    pub keymap: &'a Keymap,
    pub tier: LayoutTier,
    /// How much of the now-playing pane the cover takes.
    pub cover_size: CoverSize,
}

impl UiState<'_> {
    /// A key hint such as `[1]`, taken from the live keymap.
    ///
    /// Unbound actions print `[ ]` rather than a key that does not work.
    /// Where the cover goes, for the size the user asked for.
    pub fn artwork(&self, area: Rect) -> Option<Rect> {
        compute(area, self.tier, self.cover_size).artwork
    }

    pub fn hint(&self, action: Action) -> String {
        match self.keymap.key_for(action) {
            Some(key) => format!("[{}]", key.label()),
            None => "[  ]".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_declaration_is_the_whole_page() {
        // Order, headings and the row count all come from the one list; this
        // is what would fail if a setting were added to the page without being
        // added to that list.
        assert_eq!(Setting::COUNT, SETTINGS_ROWS);
        assert_eq!(Setting::ALL.len(), SETTINGS_ROWS);

        let mut labels = Vec::new();
        for (index, (setting, label)) in Setting::ALL.iter().enumerate() {
            assert_eq!(Setting::index_of(*setting), index);
            assert_eq!(Setting::at(index), Some(*setting));
            assert_eq!(setting.label(), *label);
            assert!(!labels.contains(label), "{label} is used twice");
            labels.push(label);
        }
        assert_eq!(Setting::at(SETTINGS_ROWS), None);
    }

    #[test]
    fn every_setting_has_a_value_to_show() {
        let values = SettingsValues {
            quality: "exhigh",
            mode: "Sequential",
            volume: 72,
            cover_size: "normal",
        };
        assert_eq!(values.value(Setting::AudioQuality), "exhigh");
        assert_eq!(values.value(Setting::PlaybackMode), "Sequential");
        assert_eq!(values.value(Setting::Volume), "72%");
        assert_eq!(values.value(Setting::CoverSize), "normal");
        // The keybinding row opens rather than showing a value of its own.
        assert_eq!(values.value(Setting::Keybindings), "›");
    }

    #[test]
    fn every_page_has_a_distinct_action_and_title() {
        let mut actions = Vec::new();
        let mut titles = Vec::new();
        for page in Page::ALL {
            assert!(
                !actions.contains(&page.action()),
                "{page:?} reuses an action"
            );
            assert!(!titles.contains(&page.title()), "{page:?} reuses a title");
            actions.push(page.action());
            titles.push(page.title());
        }
    }

    #[test]
    fn hints_come_from_the_live_keymap() {
        let keymap = Keymap::default();
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Login(LoginView::Starting),
            list: ListView::Message(""),
            now_playing: NowPlayingView {
                song: None,
                metadata: false,
                position_ms: None,
                duration_ms: None,
                artwork: None,
                lyrics: LyricsView::Pending,
                source: None,
            },
            status: StatusView {
                loading: false,
                notice: None,
                volume: 72,
                playback_mode: PlaybackMode::Sequential,
                queue_len: 0,
                playback: PlaybackStatus::Stopped,
                bitrate: None,
            },
            keymap: &keymap,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        };

        assert_eq!(state.hint(Action::ShowPlaylist), "[1]");
        assert_eq!(state.hint(Action::PlayPause), "[Space]");
        assert_eq!(state.hint(Action::ToggleQueue), "[U]");
    }

    #[test]
    fn a_rebound_action_changes_its_hint() {
        let mut keymap = Keymap::default();
        keymap
            .rebind(Action::Quit, niveus_config::Key::Char('x'))
            .unwrap();
        let map = keymap;
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Login(LoginView::Starting),
            list: ListView::Message(""),
            now_playing: NowPlayingView {
                song: None,
                metadata: false,
                position_ms: None,
                duration_ms: None,
                artwork: None,
                lyrics: LyricsView::Pending,
                source: None,
            },
            status: StatusView {
                loading: false,
                notice: None,
                volume: 72,
                playback_mode: PlaybackMode::Sequential,
                queue_len: 0,
                playback: PlaybackStatus::Stopped,
                bitrate: None,
            },
            keymap: &map,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        };
        assert_eq!(state.hint(Action::Quit), "[X]");
    }
}
