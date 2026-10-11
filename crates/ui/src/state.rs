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
use std::collections::VecDeque;

/// One of the settings a page can change.
///
/// Keybindings are not here: they are a list rather than a value, and a
/// secondary item of their own (SPEC §10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    AudioQuality,
    PlaybackMode,
    Volume,
    CoverSize,
}

impl Setting {
    /// Every setting, in the order the pages list them, with the name each is
    /// shown under.
    ///
    /// One declaration on purpose: the heading and the label are both read
    /// from here, so adding a setting is one line rather than a hunt through
    /// the renderer and the app for the places that used to count them
    /// (SPEC §35, Phase 10).
    pub const ALL: [(Setting, &'static str); 4] = [
        (Setting::AudioQuality, "AUDIO QUALITY"),
        (Setting::PlaybackMode, "PLAYBACK MODE"),
        (Setting::Volume, "VOLUME"),
        (Setting::CoverSize, "COVER SIZE"),
    ];

    /// The name a setting is listed under.
    pub fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(setting, _)| *setting == self)
            .expect("ALL lists every setting")
            .1
    }
}

/// How the settings are grouped in the secondary column (SPEC §10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingGroup {
    Playback,
    Appearance,
}

impl SettingGroup {
    /// Every group, in the order SETTINGS lists them.
    pub const ALL: [SettingGroup; 2] = [SettingGroup::Playback, SettingGroup::Appearance];

    /// The settings this group lists, in the order it lists them.
    pub fn settings(self) -> &'static [Setting] {
        match self {
            SettingGroup::Playback => &[
                Setting::AudioQuality,
                Setting::PlaybackMode,
                Setting::Volume,
            ],
            SettingGroup::Appearance => &[Setting::CoverSize],
        }
    }

    /// How many settings the group lists.
    pub fn count(self) -> usize {
        self.settings().len()
    }

    /// The setting at a position in this group's list.
    pub fn at(self, index: usize) -> Option<Setting> {
        self.settings().get(index).copied()
    }

    /// The group a setting belongs to.
    pub fn of(setting: Setting) -> SettingGroup {
        match setting {
            Setting::CoverSize => SettingGroup::Appearance,
            Setting::AudioQuality | Setting::PlaybackMode | Setting::Volume => {
                SettingGroup::Playback
            }
        }
    }

    /// The heading the group's page carries.
    pub fn label(self) -> &'static str {
        match self {
            SettingGroup::Playback => crate::strings::nav::PLAYBACK,
            SettingGroup::Appearance => crate::strings::nav::APPEARANCE,
        }
    }
}

/// The values the settings pages show on the right of each row.
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
            Setting::AudioQuality => self.quality.to_string(),
            Setting::PlaybackMode => self.mode.to_string(),
            Setting::Volume => format!("{}%", self.volume),
            Setting::CoverSize => self.cover_size.to_string(),
        }
    }
}

/// A first-level navigation item (SPEC §10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primary {
    Library,
    Search,
    Queue,
    Settings,
}

impl Primary {
    /// Every item, including the one not drawn yet.
    pub const ALL: [Primary; 4] = [
        Primary::Library,
        Primary::Search,
        Primary::Queue,
        Primary::Settings,
    ];

    /// The items the navigation row draws.
    ///
    /// `Search` is reserved but has nothing behind it, and an entry that leads
    /// nowhere is worse than no entry — the structure keeps its place, the row
    /// does not (SPEC §10.1).
    pub const RENDERED: [Primary; 3] = [Primary::Library, Primary::Queue, Primary::Settings];

    pub fn title(self) -> &'static str {
        use crate::strings::nav;
        match self {
            Primary::Library => nav::LIBRARY,
            Primary::Search => nav::SEARCH,
            Primary::Queue => nav::QUEUE,
            Primary::Settings => nav::SETTINGS,
        }
    }

    /// The action that switches to this item, so the navigation row can show
    /// the key actually bound to it.
    pub fn action(self) -> Action {
        match self {
            Primary::Library => Action::ShowLibrary,
            Primary::Search => Action::ShowSearch,
            Primary::Queue => Action::ShowQueue,
            Primary::Settings => Action::ShowSettings,
        }
    }

    /// The secondary items this primary shows.
    pub fn sections(self) -> &'static [Section] {
        match self {
            Primary::Library => &[Section::Liked, Section::Playlists, Section::History],
            // Nothing is built behind SEARCH yet (SPEC §10.2).
            Primary::Search => &[],
            Primary::Queue => &[Section::Current, Section::UpNext],
            Primary::Settings => &[Section::Playback, Section::Appearance, Section::Keybindings],
        }
    }

    /// The secondary item selected when the primary is first reached.
    pub fn default_section(self) -> Section {
        match self {
            // The playlists, not the liked list, are what the browser is for.
            Primary::Library => Section::Playlists,
            Primary::Search => Section::Liked,
            Primary::Queue => Section::Current,
            Primary::Settings => Section::Playback,
        }
    }
}

/// A secondary item, chosen in the column beside the main area (SPEC §10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Liked,
    Playlists,
    History,
    Current,
    UpNext,
    Playback,
    Appearance,
    Keybindings,
}

impl Section {
    /// The name the secondary column lists it under.
    pub fn label(self) -> &'static str {
        use crate::strings::nav;
        match self {
            Section::Liked => nav::LIKED,
            Section::Playlists => nav::PLAYLISTS,
            Section::History => nav::HISTORY,
            Section::Current => nav::CURRENT,
            Section::UpNext => nav::UP_NEXT,
            Section::Playback => nav::PLAYBACK,
            Section::Appearance => nav::APPEARANCE,
            Section::Keybindings => nav::KEYBINDINGS,
        }
    }
}

/// Which of the two focusable regions holds the focus (SPEC §10.3).
///
/// The arrow keys act on whichever this is, so it is also the thing the
/// interface has to make visible: an explicit scope the user cannot see is the
/// implicit one §23 forbids by another name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Secondary,
    Main,
}

impl Focus {
    pub fn toggled(self) -> Self {
        match self {
            Focus::Secondary => Focus::Main,
            Focus::Main => Focus::Secondary,
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
    /// The playback queue, which takes the main area over while it is shown.
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
    /// The tracks queued to play next, ahead of the rest of the queue.
    ///
    /// Read-only: reordering and deleting act on the queue's own items, not on
    /// what was queued in front of them (SPEC §17.1). The songs are held as the
    /// queue's own deque rather than copied out of it.
    UpNext {
        songs: &'a VecDeque<Song>,
        selected: usize,
        scroll: usize,
    },
    /// One group of the settings. Values are shown on the right.
    Settings {
        group: SettingGroup,
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
            ListView::UpNext { songs, .. } => songs.len(),
            ListView::History { entries, .. } => entries.len(),
            // Only the rows of the group showing, not every setting there is.
            ListView::Settings { group, .. } => group.count(),
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
    /// Which first-level item is showing.
    pub primary: Primary,
    /// Which of its secondary items is selected.
    pub section: Section,
    /// Which of the two focusable regions the arrow keys act on.
    pub focus: Focus,
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

    fn state<'a>(keymap: &'a Keymap) -> UiState<'a> {
        UiState {
            primary: Primary::Library,
            section: Section::Playlists,
            focus: Focus::Main,
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
            keymap,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        }
    }

    #[test]
    fn the_settings_declaration_is_the_whole_page() {
        // Every setting carries a label, and no label is used twice.
        let mut labels = Vec::new();
        for (setting, label) in Setting::ALL {
            assert_eq!(setting.label(), label);
            assert!(!labels.contains(&label), "{label} is used twice");
            labels.push(label);
        }
    }

    #[test]
    fn every_setting_is_listed_by_exactly_one_group() {
        // The grouping is what the secondary column shows: a setting that
        // belongs to no group would be unreachable, and one in two groups
        // would appear twice.
        let mut seen = Vec::new();
        for group in SettingGroup::ALL {
            assert!(group.count() > 0, "{group:?} lists nothing");
            for (index, setting) in group.settings().iter().enumerate() {
                assert_eq!(group.at(index), Some(*setting));
                assert_eq!(SettingGroup::of(*setting), group);
                assert!(!seen.contains(setting), "{setting:?} is listed twice");
                seen.push(*setting);
            }
            assert_eq!(group.at(group.count()), None);
        }
        assert_eq!(seen.len(), Setting::ALL.len(), "a setting is in no group");
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
    }

    #[test]
    fn every_primary_has_a_distinct_action_and_title() {
        let mut actions = Vec::new();
        let mut titles = Vec::new();
        for primary in Primary::ALL {
            assert!(
                !actions.contains(&primary.action()),
                "{primary:?} reuses an action"
            );
            assert!(
                !titles.contains(&primary.title()),
                "{primary:?} reuses a title"
            );
            actions.push(primary.action());
            titles.push(primary.title());
        }
    }

    #[test]
    fn the_reserved_primary_is_not_drawn_but_keeps_its_key() {
        // SEARCH has no content yet, so it is absent from the row — but its
        // action and binding exist, so adding it later moves nothing (SPEC
        // §10.1).
        assert!(!Primary::RENDERED.contains(&Primary::Search));
        assert!(Primary::Search.sections().is_empty());
        assert_eq!(Primary::Search.action(), Action::ShowSearch);
        assert_eq!(
            Keymap::default().key_for(Action::ShowSearch),
            Some(niveus_config::Key::Char('2'))
        );
    }

    #[test]
    fn the_secondary_items_of_each_primary_are_named_and_distinct() {
        for primary in Primary::ALL {
            let sections = primary.sections();
            assert!(
                sections.contains(&primary.default_section()) || sections.is_empty(),
                "{primary:?} defaults to an item it does not list"
            );
            let mut labels = Vec::new();
            for section in sections {
                let label = section.label();
                assert!(!label.is_empty(), "{section:?} has no label");
                assert!(!labels.contains(&label), "{label} is used twice");
                labels.push(label);
            }
        }
    }

    #[test]
    fn focus_toggles_between_the_two_regions() {
        assert_eq!(Focus::Secondary.toggled(), Focus::Main);
        assert_eq!(Focus::Main.toggled(), Focus::Secondary);
    }

    #[test]
    fn hints_come_from_the_live_keymap() {
        let keymap = Keymap::default();
        let state = state(&keymap);

        assert_eq!(state.hint(Action::ShowLibrary), "[1]");
        assert_eq!(state.hint(Action::ShowQueue), "[3]");
        assert_eq!(state.hint(Action::FocusNext), "[Tab]");
        assert_eq!(state.hint(Action::PlayPause), "[Space]");
    }

    #[test]
    fn a_rebound_action_changes_its_hint() {
        let mut keymap = Keymap::default();
        keymap
            .rebind(Action::Quit, niveus_config::Key::Char('x'))
            .unwrap();
        let state = state(&keymap);
        assert_eq!(state.hint(Action::Quit), "[X]");
    }
}
