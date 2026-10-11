//! The application's own state, and the only place it changes.

use crate::artwork as cover;
use crate::colour;
use crate::event::AppEvent;
use crate::paths::Paths;
use crate::persistence::{CursorState, StateFile};
use niveus_artwork::{ArtworkError, Cache as CoverCache};
use niveus_artwork::{ColourDepth, CoverKey, Frame};
use niveus_config::{Action, Config};
use niveus_mpris::{MprisCommand, MprisService, Snapshot};
use niveus_music_core::{
    AudioSource, History, PlaybackMode, PlaybackState, PlaybackStatus, Playlist, PlaylistId,
    PlaylistKind, Queue, RawLyrics, Song, User,
};
use niveus_ncm_provider::{MusicProvider, NcmProvider, ProviderError, QrLoginState};
use niveus_player::{LaunchOptions, Player, PlayerError, PlayerEvent};
use niveus_ui::{
    ArtworkView, ListView, LoginView, LyricWindow, LyricsView, Page, QrGrid, SessionView, Setting,
    SettingsValues, StatusView, UiState,
};
use ratatui::layout::Rect;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self, Sender};
use tokio_util::sync::CancellationToken;

/// How long a transient message stays on the status bar.
const NOTICE_LIFETIME: Duration = Duration::from_secs(5);
/// How often a QR code is polled.
const QR_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// How far one press of either seek key moves the position.
const SEEK_STEP: f64 = 5.0;
/// How much one press of either volume key changes the volume.
const VOLUME_STEP: i16 = 5;
/// How soon after it started a player has to die for the death to count as a
/// failed start rather than bad luck.
const PLAYER_RESTART_WINDOW: Duration = Duration::from_secs(5);
/// How many of those in a row are tolerated before starting another one is
/// given up on as pointless.
const MAX_PLAYER_RESTARTS: u32 = 3;

/// Where sign-in has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginPhase {
    /// The stored cookie is being checked.
    Restoring,
    /// A QR code is being requested.
    Starting,
    /// A code is displayed and being polled.
    Waiting,
    /// The code was scanned; the phone is awaiting confirmation.
    Scanned,
    /// Sign-in cannot proceed.
    Failed(String),
}

/// Which list a cursor position belongs to.
///
/// Each playlist counts as a list of its own: where the cursor was left in one
/// has nothing to do with where it was left in another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ListId {
    Playlists,
    Playlist(PlaylistId),
    Likes,
    History,
    Queue,
    Settings,
    Keybindings,
}

impl ListId {
    /// The name this list's cursor is written under in `state.json`.
    ///
    /// `None` for the lists that are not places to come back to: the queue is
    /// rebuilt by whatever is playing, and the settings pages are short enough
    /// to start at the top.
    fn persist_key(self) -> Option<String> {
        match self {
            ListId::Playlists => Some("playlists".to_string()),
            ListId::Likes => Some("likes".to_string()),
            ListId::History => Some("history".to_string()),
            ListId::Playlist(id) => Some(format!("playlist:{id}")),
            ListId::Queue | ListId::Settings | ListId::Keybindings => None,
        }
    }

    /// The list a name in `state.json` refers to.
    ///
    /// A name this version does not know is skipped rather than treated as an
    /// error: a state file written by a later release must not stop this one.
    fn from_persist_key(key: &str) -> Option<ListId> {
        match key {
            "playlists" => Some(ListId::Playlists),
            "likes" => Some(ListId::Likes),
            "history" => Some(ListId::History),
            other => other
                .strip_prefix("playlist:")
                .and_then(|id| id.parse().ok())
                .map(|id| ListId::Playlist(PlaylistId(id))),
        }
    }
}

/// Where the cursor was left in a list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Cursor {
    selected: usize,
    scroll: usize,
}

/// What the settings page is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum SettingsMode {
    /// Moving between the settings.
    #[default]
    Browse,
    /// Changing the value of one of them: the arrows adjust it and Enter puts
    /// it back down.
    Editing(Setting),
    /// The keybinding list is open.
    Keybindings,
    /// The keybinding list is open and the next key pressed will be bound.
    Capturing,
}

/// What the lyrics panel has to work with for the current track.
///
/// The lyrics themselves are held parsed, not raw: the position they are read
/// at changes several times a second, and parsing on every frame to answer a
/// question about the same lines would be wasted work.
#[derive(Debug, Default)]
enum LyricsState {
    /// Nothing is playing, or the answer has not arrived yet.
    #[default]
    Pending,
    /// Parsed, ready to be windowed at whatever position playback is at.
    Ready(niveus_lyrics::Lyrics),
    /// The service sent a translation but no original text.
    TranslationOnly,
    /// No original text at all: an instrumental, or a track with no lyrics.
    Instrumental,
    /// There are lyrics, but they could not be fetched or parsed.
    Failed,
}

/// Which list the browsable pane is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Playlists,
    Songs,
    Likes,
    History,
    Queue,
    Settings,
}

#[derive(Debug, Clone)]
struct Notice {
    text: String,
    until: Instant,
}

/// Everything the application knows.
pub struct AppState {
    pub paths: Paths,
    pub config: Config,
    pub provider: Arc<NcmProvider>,

    pub user: Option<User>,
    pub login: LoginPhase,
    /// The displayed code, kept so the renderer can borrow it each frame.
    pub qr: Option<QrGrid>,

    pub page: Page,
    /// The playlist whose tracks are on screen, when the pane is showing any.
    pub opened_playlist: Option<Playlist>,
    pub created: Vec<Playlist>,
    pub collected: Vec<Playlist>,
    pub songs: Vec<Song>,
    pub likes: Vec<Song>,
    pub likes_loaded: bool,

    /// Index of the highlighted entry within the current pane.
    pub selected: usize,
    /// Index of the first entry drawn, so the selection stays on screen.
    pub scroll: usize,
    /// Which list `selected` and `scroll` are about.
    current_list: ListId,
    /// Where the cursor was left in each list, so going back to one finds it
    /// where it was rather than at the top.
    cursors: HashMap<ListId, Cursor>,

    /// Whether the metadata block is showing along the bottom of the panel.
    ///
    /// A switch, not a choice between two views: the lyrics stay up and the
    /// block joins them (SPEC §12.1).
    pub metadata: bool,
    pub queue: Queue,
    pub playback: PlaybackState,
    /// The player, once it is up. Playback is unavailable until then, and
    /// browsing is unaffected either way.
    pub player: Option<Player>,
    /// The track playing now, with the resource it is playing from.
    pub current_song: Option<Song>,
    pub source: Option<AudioSource>,
    /// Whether the queue has taken the pane over.
    pub queue_view: bool,
    /// What the lyrics panel has for the track playing now.
    lyrics: LyricsState,
    /// The picture showing in the now-playing pane, once one is ready.
    pub artwork: Option<ArtworkView>,
    /// The cover the pane is currently showing, or waiting for.
    ///
    /// A picture for anything else — another track, another size — is not the
    /// one that was asked for and is dropped when it arrives.
    pub artwork_key: Option<CoverKey>,
    artwork_cache: CoverCache,
    /// What the terminal can be trusted with, worked out once at start-up.
    pub colour_depth: ColourDepth,
    /// When the terminal last changed size, so the redraw can be waited out.
    resize_at: Option<Instant>,
    /// The desktop's view of the program, when there is a desktop listening.
    pub mpris: Option<MprisService>,
    /// What the desktop was last told, so it is only told about changes.
    published: Snapshot,
    /// Bumped for every track that is asked to play.
    ///
    /// A source that was resolved for a track the user has since left is
    /// discarded, which is what keeps a slow reply from overriding a fast one.
    pub play_generation: u64,
    /// The generation of the track the player was last handed, if it has been
    /// handed one at all.
    ///
    /// The player's own reports — a position, an end of file — describe
    /// whatever it was last given. Comparing this with `play_generation` is
    /// what tells an event about the track being replaced from one about the
    /// track playing now, and stops a natural end that was already on its way
    /// from skipping a track the user has just chosen.
    ///
    /// `None` until something has actually been handed over, and the difference
    /// matters: a player that has been given nothing still reports its own
    /// state — mpv answers for `pause` the moment it is asked to watch it —
    /// and a session restored from `state.json` shows a track it has never
    /// played. Holding the generation rather than a separate "handed over"
    /// flag keeps the two from drifting apart.
    pub loaded_generation: Option<u64>,

    /// When the player process was last started.
    ///
    /// A player that dies seconds after starting is failing to start, and
    /// starting another one will not help; one that dies an hour later is
    /// simply an interruption, and a fresh one is the fix.
    player_started_at: Option<Instant>,
    /// How many players have died shortly after starting, in a row.
    player_restarts: u32,

    /// Requests in flight, which is what the status bar reports as loading.
    pub pending: u32,
    notice: Option<Notice>,

    /// Bumped whenever the browsing context changes.
    ///
    /// Replies tagged with an older generation are discarded, which is what
    /// keeps a slow request from overwriting a newer one.
    pub generation: u64,
    /// Cancels the requests belonging to the current context.
    cancel: CancellationToken,

    /// What has been heard, newest first (§30). Written to `state.json` so the
    /// HISTORY page survives a restart.
    pub history: History,
    /// The playlist a previous session had open, until it can be opened again.
    ///
    /// Only its id is written down: which playlists exist is the service's to
    /// say, so the playlist itself is waited for rather than invented.
    pending_playlist: Option<PlaylistId>,
    /// Set when a track came back from `state.json` and was never handed to the
    /// player. Pressing play starts it instead of doing nothing.
    resume_on_play: bool,
    /// What the settings page is doing, if it is the page showing.
    settings_mode: SettingsMode,

    pub quit: bool,
    pub size: (u16, u16),
    /// Things to tell the user once the screen is up, one per idle tick.
    ///
    /// A queue rather than one message: a configuration with two problems in it
    /// should be able to report both, and the interface is the only place with
    /// room to say anything at all.
    warnings: VecDeque<String>,
}

impl AppState {
    pub fn new(
        paths: Paths,
        config: Config,
        provider: Arc<NcmProvider>,
        warnings: Vec<String>,
    ) -> Self {
        let playback = PlaybackState::default();
        let queue = Queue::new(PlaybackMode::Sequential);
        // Worked out once, from the preference and the terminal: the terminal
        // does not change its abilities mid-session, and asking repeatedly
        // would only risk answering differently.
        let colour_depth = colour::detect(config.color_mode);
        Self {
            paths,
            config,
            provider,
            user: None,
            login: LoginPhase::Restoring,
            qr: None,
            page: Page::Playlist,
            opened_playlist: None,
            created: Vec::new(),
            collected: Vec::new(),
            songs: Vec::new(),
            likes: Vec::new(),
            likes_loaded: false,
            selected: 0,
            scroll: 0,
            current_list: ListId::Playlists,
            cursors: HashMap::new(),
            metadata: false,
            queue,
            playback,
            player: None,
            current_song: None,
            source: None,
            queue_view: false,
            lyrics: LyricsState::default(),
            artwork: None,
            artwork_key: None,
            artwork_cache: CoverCache::default(),
            colour_depth,
            resize_at: None,
            mpris: None,
            published: Snapshot::default(),
            play_generation: 0,
            loaded_generation: None,
            player_started_at: None,
            player_restarts: 0,
            pending: 0,
            notice: None,
            generation: 0,
            cancel: CancellationToken::new(),
            history: History::new(),
            pending_playlist: None,
            resume_on_play: false,
            settings_mode: SettingsMode::default(),
            quit: false,
            size: (0, 0),
            warnings: warnings.into(),
        }
    }

    /// Takes on what the last session left behind (§29).
    ///
    /// Called before the player is started, so the volume the interface shows
    /// is the volume the player is given. Nothing here is trusted: a queue
    /// whose playlist no longer exists, or a track that cannot be played any
    /// more, is discovered when it is asked for and answered then.
    pub fn restore(&mut self, state: StateFile) {
        self.queue = state.queue;
        self.playback.volume = state.volume;
        self.playback.position_ms = state.position_ms;
        self.history = state.history;
        self.pending_playlist = state.current_playlist_id;

        for (key, cursor) in state.cursors {
            if let Some(list) = ListId::from_persist_key(&key) {
                self.cursors.insert(
                    list,
                    Cursor {
                        selected: cursor.selected,
                        scroll: cursor.scroll,
                    },
                );
            }
        }
        // The list being looked at when the program stopped is the playlist
        // list, unless a playlist was open — whose cursor is picked up when it
        // opens. Either way the remembered position is where the cursor starts.
        if let Some(cursor) = self.cursors.get(&ListId::Playlists).copied() {
            self.selected = cursor.selected;
            self.scroll = cursor.scroll;
        }

        // A restored track shows in the pane and is marked in the queue, but
        // nothing claims it is playing: no sound came out of this process.
        //
        // The queue holds the whole track; the id on its own is what the file
        // also carries, and it is used when the queue has no current entry —
        // the track is then still pointed at, if it is still in the list.
        let song_id = state.current_song_id;
        let restored = self.queue.playing.clone().or_else(|| {
            let id = song_id?;
            self.queue.items.iter().find(|song| song.id == id).cloned()
        });
        if let Some(song) = restored {
            tracing::info!(song = %song.title, "restored the track that was playing");
            self.playback.duration_ms = song.duration_ms.map(u64::from);
            self.current_song = Some(song);
            self.playback.status = PlaybackStatus::Stopped;
            self.resume_on_play = true;
        }
    }

    /// Writes what the program was doing, so it can be picked up next time.
    ///
    /// Called on the way out and whenever a setting that belongs in this file
    /// changes (§26). A failure is reported rather than swallowed: the user
    /// would otherwise find their queue gone with nothing to explain it.
    pub fn persist_state(&mut self) {
        let state = StateFile {
            current_playlist_id: self.current_playlist_id(),
            current_song_id: self.current_song.as_ref().map(|song| song.id),
            position_ms: self.playback.position_ms,
            volume: self.playback.volume,
            queue: self.queue.clone(),
            history: self.history.clone(),
            cursors: self.cursors_for_persistence(),
        };

        let path = self.paths.state_file();
        match state.save(&path) {
            Ok(()) => tracing::debug!(path = %path.display(), "state saved"),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "cannot save state");
                self.notify("状态无法保存，下次启动将无法恢复");
            }
        }
    }

    /// The playlist being browsed, if any.
    fn current_playlist_id(&self) -> Option<PlaylistId> {
        self.opened_playlist
            .as_ref()
            .map(|playlist| playlist.id)
            .or(self.pending_playlist)
    }

    /// Every browsing list's cursor, with the one being looked at now put away
    /// first — it is not written to the map until the user leaves it.
    fn cursors_for_persistence(&self) -> std::collections::BTreeMap<String, CursorState> {
        let mut saved = std::collections::BTreeMap::new();
        for (list, cursor) in &self.cursors {
            if let Some(key) = list.persist_key() {
                saved.insert(
                    key,
                    CursorState {
                        selected: cursor.selected,
                        scroll: cursor.scroll,
                    },
                );
            }
        }
        if let Some(key) = self.current_list.persist_key() {
            saved.insert(
                key,
                CursorState {
                    selected: self.selected,
                    scroll: self.scroll,
                },
            );
        }
        saved
    }

    // -----------------------------------------------------------------
    // Projection
    // -----------------------------------------------------------------

    pub fn tier(&self) -> niveus_ui::LayoutTier {
        niveus_ui::LayoutTier::for_size(self.size.0, self.size.1)
    }

    /// The read-only view the renderer consumes.
    pub fn ui_state(&self) -> UiState<'_> {
        UiState {
            page: self.page,
            session: self.session_view(),
            list: self.list_view(),
            now_playing: niveus_ui::state::NowPlayingView {
                song: self.current_song.as_ref(),
                metadata: self.metadata,
                position_ms: Some(self.playback.position_ms),
                // The player's own figure wins: it is what it found in the
                // file, rather than what it was told to expect.
                duration_ms: self
                    .playback
                    .duration_ms
                    .or_else(|| self.current_song.as_ref()?.duration_ms.map(u64::from)),
                artwork: self.artwork.as_ref(),
                lyrics: self.lyrics_view(),
                source: self.source.as_ref(),
            },
            status: StatusView {
                loading: self.pending > 0,
                notice: self.notice.as_ref().map(|notice| notice.text.as_str()),
                volume: self.playback.volume,
                playback_mode: self.queue.playback_mode,
                queue_len: self.queue.len(),
                playback: self.playback.status,
                bitrate: self.source.as_ref().and_then(|source| source.bitrate),
            },
            keymap: &self.config.keymap,
            tier: self.tier(),
            cover_size: self.config.cover_size,
        }
    }

    /// The lyric window the panel should draw, read at the current position.
    ///
    /// Which line is being sung, and how many of its neighbours fit, are worked
    /// out here once per frame rather than in the renderer: the UI is handed the
    /// lines and draws them (SPEC §13.5).
    fn lyrics_view(&self) -> LyricsView<'_> {
        // A track that is not playing has no lyrics to show, whatever the last
        // one's answer was.
        if self.current_song.is_none() {
            return LyricsView::Pending;
        }
        match &self.lyrics {
            LyricsState::Pending => LyricsView::Pending,
            LyricsState::Ready(lyrics) => {
                let window = lyrics.window_at(self.playback.position_ms, self.lyric_rows());
                LyricsView::Window(LyricWindow::new(window.lines().to_vec()))
            }
            LyricsState::TranslationOnly => LyricsView::TranslationOnly,
            LyricsState::Instrumental => LyricsView::Instrumental,
            LyricsState::Failed => LyricsView::Failed,
        }
    }

    /// How many rows the panel can give the lyric window right now.
    ///
    /// Asked of the layout rather than worked out here, so the window that gets
    /// built is the size of the space the renderer will draw it in: a switch of
    /// the metadata block, a resize or another cover size all change this, and
    /// all of them are answered by the same call.
    fn lyric_rows(&self) -> usize {
        let area = Rect::new(0, 0, self.size.0.max(1), self.size.1.max(1));
        let regions = niveus_ui::compute(area, self.tier(), self.config.cover_size);
        let Some(panel) = regions.panel else {
            return 0;
        };
        let lyrics = niveus_ui::panel_blocks(panel, self.metadata).lyrics;
        lyrics.height as usize
    }

    fn session_view(&self) -> SessionView<'_> {
        match &self.user {
            Some(user) => SessionView::Ready { user },
            None => SessionView::Login(match (&self.login, &self.qr) {
                (LoginPhase::Failed(message), _) => LoginView::Failed { message },
                // The stored session is being checked; saying a code is being
                // fetched would describe something that is not happening.
                (LoginPhase::Restoring, _) => LoginView::Checking,
                (LoginPhase::Waiting, Some(grid)) => LoginView::Waiting {
                    grid,
                    scanned: false,
                },
                (LoginPhase::Scanned, Some(grid)) => LoginView::Waiting {
                    grid,
                    scanned: true,
                },
                _ => LoginView::Starting,
            }),
        }
    }

    fn pane(&self) -> Pane {
        // The queue replaces whatever list was showing, and gives it back when
        // it is closed.
        if self.queue_view {
            return Pane::Queue;
        }
        match self.page {
            Page::Playlist if self.opened_playlist.is_some() => Pane::Songs,
            Page::Playlist => Pane::Playlists,
            Page::Favorites => Pane::Likes,
            Page::History => Pane::History,
            Page::Settings => Pane::Settings,
        }
    }

    fn list_view(&self) -> ListView<'_> {
        match self.pane() {
            Pane::Playlists => ListView::Playlists {
                created: &self.created,
                collected: &self.collected,
                selected: self.selected,
                scroll: self.scroll,
            },
            Pane::Songs => ListView::Songs {
                playlist_name: self
                    .opened_playlist
                    .as_ref()
                    .map(|p| p.name.as_str())
                    .unwrap_or(""),
                songs: &self.songs,
                selected: self.selected,
                scroll: self.scroll,
            },
            Pane::Likes => ListView::Likes {
                songs: &self.likes,
                selected: self.selected,
                scroll: self.scroll,
            },
            Pane::History => ListView::History {
                entries: self.history.entries(),
                selected: self.selected,
                scroll: self.scroll,
            },
            Pane::Queue => ListView::Queue {
                songs: &self.queue.items,
                // The track playing is marked only when it is one of the
                // queue's own: a "play next" track is not in the list.
                playing: self.current_song.as_ref().and_then(|playing| {
                    self.queue
                        .items
                        .iter()
                        .position(|song| song.id == playing.id)
                }),
                selected: self.selected,
                scroll: self.scroll,
            },
            Pane::Settings
                if self.settings_mode == SettingsMode::Keybindings
                    || self.settings_mode == SettingsMode::Capturing =>
            {
                ListView::Keybindings {
                    selected: self.selected,
                    scroll: self.scroll,
                    capturing: self.settings_mode == SettingsMode::Capturing,
                }
            }
            Pane::Settings => ListView::Settings {
                values: SettingsValues {
                    quality: self.config.audio_quality.label(),
                    mode: self.queue.playback_mode.label(),
                    volume: self.playback.volume,
                    cover_size: self.config.cover_size.name(),
                },
                selected: self.selected,
                editing: match self.settings_mode {
                    SettingsMode::Editing(setting) => Some(setting),
                    _ => None,
                },
            },
        }
    }

    /// How many entries fit on screen right now.
    ///
    /// The pane spends some of its rows on things that are not entries — the
    /// group headings in the playlist list — and those rows have to come off
    /// the top, or scrolling stops one or three entries too late and takes the
    /// cursor off the bottom with it.
    fn visible_items(&self) -> usize {
        let regions = niveus_ui::compute(
            Rect::new(0, 0, self.size.0.max(1), self.size.1.max(1)),
            self.tier(),
            self.config.cover_size,
        );
        let list = self.list_view();
        let per_item = list.rows_per_item().max(1);
        let rows = regions.list_viewport().saturating_sub(list.fixed_rows());
        (rows / per_item).max(1)
    }

    // -----------------------------------------------------------------
    // Context
    // -----------------------------------------------------------------

    /// Marks a new browsing context: earlier replies become stale, and the
    /// requests still running for the old one are cancelled.
    fn begin_context(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
        self.pending = 0;
    }

    /// Which list the cursor is currently about.
    fn list_id(&self) -> ListId {
        match self.pane() {
            Pane::Playlists => ListId::Playlists,
            Pane::Songs => self
                .opened_playlist
                .as_ref()
                .map(|playlist| ListId::Playlist(playlist.id))
                .unwrap_or(ListId::Playlists),
            Pane::Likes => ListId::Likes,
            Pane::History => ListId::History,
            Pane::Queue => ListId::Queue,
            // The keybinding list is a list of its own: it remembers its own
            // position, so reopening it does not land on the wrong row.
            Pane::Settings => match self.settings_mode {
                SettingsMode::Keybindings | SettingsMode::Capturing => ListId::Keybindings,
                _ => ListId::Settings,
            },
        }
    }

    /// Puts the cursor where this list was left, remembering where the last one
    /// was.
    ///
    /// Called after anything that changes which list is showing, so that
    /// stepping out of a list and back into it is not the same as never having
    /// been there.
    fn sync_cursor(&mut self) {
        let next = self.list_id();
        if next == self.current_list {
            return;
        }
        self.cursors.insert(
            self.current_list,
            Cursor {
                selected: self.selected,
                scroll: self.scroll,
            },
        );
        let saved = self.cursors.get(&next).copied().unwrap_or_default();
        self.selected = saved.selected;
        self.scroll = saved.scroll;
        self.current_list = next;
        // A remembered position can now be past the end of a list that has
        // shrunk. A playlist's tracks are still being fetched at this point, so
        // theirs is clamped when they arrive instead — measuring it here would
        // mean measuring it against whatever the previous playlist left behind.
        if !matches!(self.pane(), Pane::Songs) {
            self.clamp_selection();
        }
    }

    /// Queues something to tell the user once the screen is up.
    ///
    /// For anything known before the terminal is initialized: a notice cannot
    /// be shown yet, and a message the user never sees is not a message.
    pub fn warn(&mut self, text: impl Into<String>) {
        self.warnings.push_back(text.into());
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            until: Instant::now() + NOTICE_LIFETIME,
        });
    }

    pub fn expire_notices(&mut self) {
        if let Some(notice) = &self.notice {
            if Instant::now() >= notice.until {
                self.notice = None;
            }
        }
    }

    // -----------------------------------------------------------------
    // Requests
    // -----------------------------------------------------------------

    /// Checks the stored cookie.
    pub fn request_session(&mut self, tx: &Sender<AppEvent>) {
        let provider = self.provider.clone();
        let tx = tx.clone();
        self.pending += 1;
        tokio::spawn(async move {
            let result = provider.restore_session().await;
            let _ = tx.send(AppEvent::SessionRestored(result)).await;
        });
    }

    /// Asks for a QR code and starts polling it.
    pub fn request_qr(&mut self, tx: &Sender<AppEvent>) {
        self.qr = None;
        self.login = LoginPhase::Starting;
        let provider = self.provider.clone();
        let tx = tx.clone();
        self.pending += 1;
        tokio::spawn(async move {
            let result = provider.start_qr_login().await;
            let _ = tx.send(AppEvent::QrStarted(result)).await;
        });
    }

    fn spawn_qr_poll(&self, tx: &Sender<AppEvent>, key: String) {
        let provider = self.provider.clone();
        let tx = tx.clone();
        let token = self.cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(QR_POLL_INTERVAL) => {}
                }
                let state = provider.poll_qr_login(&key).await;
                let finished = matches!(
                    state,
                    Ok(QrLoginState::Confirmed) | Ok(QrLoginState::Expired) | Err(_)
                );
                if tx.send(AppEvent::QrPolled(state)).await.is_err() || finished {
                    return;
                }
            }
        });
    }

    fn spawn_qr_finish(&mut self, tx: &Sender<AppEvent>) {
        let provider = self.provider.clone();
        let tx = tx.clone();
        self.pending += 1;
        tokio::spawn(async move {
            let result = provider.finish_qr_login().await;
            let _ = tx.send(AppEvent::QrConfirmed(result)).await;
        });
    }

    fn request_playlists(&mut self, tx: &Sender<AppEvent>) {
        let provider = self.provider.clone();
        let tx = tx.clone();
        let token = self.cancel.clone();
        self.pending += 1;
        tokio::spawn(async move {
            let result = tokio::select! {
                _ = token.cancelled() => return,
                result = provider.get_user_playlists() => result,
            };
            let _ = tx.send(AppEvent::Playlists(result)).await;
        });
    }

    fn request_songs(&mut self, tx: &Sender<AppEvent>, playlist_id: PlaylistId) {
        let provider = self.provider.clone();
        let tx = tx.clone();
        let token = self.cancel.clone();
        let generation = self.generation;
        self.pending += 1;
        tokio::spawn(async move {
            let result = tokio::select! {
                _ = token.cancelled() => return,
                result = provider.get_playlist_songs(playlist_id) => result,
            };
            let _ = tx
                .send(AppEvent::Songs {
                    generation,
                    playlist_id,
                    result,
                })
                .await;
        });
    }

    fn request_likes(&mut self, tx: &Sender<AppEvent>) {
        let provider = self.provider.clone();
        let tx = tx.clone();
        let token = self.cancel.clone();
        self.pending += 1;
        tokio::spawn(async move {
            let result = tokio::select! {
                _ = token.cancelled() => return,
                result = provider.get_liked_songs() => result,
            };
            let _ = tx.send(AppEvent::Likes(result)).await;
        });
    }

    /// Loads whatever the current page needs, if it is not loaded already.
    pub fn ensure_current_page(&mut self, tx: &Sender<AppEvent>) {
        match self.page {
            Page::Playlist if self.created.is_empty() && self.collected.is_empty() => {
                self.request_playlists(tx)
            }
            Page::Favorites if !self.likes_loaded => self.request_likes(tx),
            _ => {}
        }
    }

    // -----------------------------------------------------------------
    // Input
    // -----------------------------------------------------------------

    pub async fn handle_action(&mut self, action: Action, tx: &Sender<AppEvent>) {
        match action {
            Action::Quit => self.quit = true,
            Action::ShowPlaylist => self.go_to_page(Page::Playlist, tx),
            Action::ShowFavorites => self.go_to_page(Page::Favorites, tx),
            Action::ShowHistory => self.go_to_page(Page::History, tx),
            Action::OpenSettings => self.go_to_page(Page::Settings, tx),
            Action::Back => self.go_back(),
            // The arrows change a setting only while one is being edited;
            // otherwise they always mean "move the cursor".
            Action::MoveUp if self.editing_setting().is_some() => self.adjust_setting(-1, tx).await,
            Action::MoveDown if self.editing_setting().is_some() => {
                self.adjust_setting(1, tx).await
            }
            Action::MoveUp => self.move_selection(-1),
            Action::MoveDown => self.move_selection(1),
            Action::Select => self.activate(tx).await,
            Action::ToggleMetadata => self.metadata = !self.metadata,
            Action::ToggleQueue => self.toggle_queue_view(),
            Action::CyclePlaybackMode => self.cycle_playback_mode(),
            Action::VolumeUp => self.change_volume(VOLUME_STEP).await,
            Action::VolumeDown => self.change_volume(-VOLUME_STEP).await,
            Action::PlayPause => self.start_or_pause(tx).await,
            Action::Next => self.skip_forward(tx).await,
            Action::Previous => self.skip_back(tx).await,
            Action::SeekForward => self.seek(SEEK_STEP).await,
            Action::SeekBackward => self.seek(-SEEK_STEP).await,
            Action::PlayNext => self.queue_selected_next(),
            Action::DeleteSong => self.delete_from_queue(),
            Action::ReorderUp => self.reorder_selected(-1),
            Action::ReorderDown => self.reorder_selected(1),
        }
    }

    fn go_to_page(&mut self, page: Page, tx: &Sender<AppEvent>) {
        // Changing page while the queue is open also closes it; asking again
        // for the page already showing only closes the queue.
        if self.page == page && !self.queue_view {
            return;
        }
        // Leaving the queue view is part of leaving the page it was opened on.
        self.queue_view = false;
        self.page = page;
        // An edit in progress belongs to the page it was started on.
        self.settings_mode = SettingsMode::Browse;
        // The playlist that was open stays open: coming back to PLAYLIST means
        // coming back to where the user was, not to the top of the list. Going
        // up a level is what `Back` is for.
        self.begin_context();
        self.sync_cursor();
        self.ensure_current_page(tx);
    }

    fn go_back(&mut self) {
        // Out of the innermost thing first: an edit, then the list opened from
        // the page, then the queue, then a level of browsing.
        if let SettingsMode::Editing(setting) = self.settings_mode {
            self.settings_mode = SettingsMode::Browse;
            self.notify(format!(
                "{}：{}",
                setting.label(),
                self.setting_value(setting)
            ));
            return;
        }
        if self.settings_mode != SettingsMode::Browse {
            self.settings_mode = SettingsMode::Browse;
            self.sync_cursor();
            return;
        }
        if self.queue_view {
            self.queue_view = false;
            self.sync_cursor();
            return;
        }
        // The only nesting in the browser is playlist -> tracks.
        if self.pane() == Pane::Songs {
            self.opened_playlist = None;
            self.begin_context();
            self.songs.clear();
            self.sync_cursor();
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.list_view().item_count();
        if count == 0 {
            return;
        }
        let last = count as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        self.keep_selection_visible();
    }

    fn keep_selection_visible(&mut self) {
        let visible = self.visible_items();
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected + 1 - visible;
        }
    }

    async fn activate(&mut self, tx: &Sender<AppEvent>) {
        match self.pane() {
            Pane::Playlists => {
                let index = self.selected;
                let playlist = if index < self.created.len() {
                    self.created.get(index).cloned()
                } else {
                    self.collected.get(index - self.created.len()).cloned()
                };
                if let Some(playlist) = playlist {
                    self.open_playlist(playlist, tx);
                }
            }
            Pane::Songs | Pane::Likes | Pane::History => {
                let songs = self.listed_songs();
                self.play_from(&songs, self.selected, tx).await;
            }
            Pane::Queue => {
                let index = self.selected;
                if let Some(song) = self.queue.select(index) {
                    self.start_track(song, tx).await;
                }
            }
            Pane::Settings => self.activate_setting(),
        }
    }

    /// The songs the current pane is listing, if it is listing any.
    fn visible_songs(&self) -> &[Song] {
        match self.pane() {
            Pane::Songs => &self.songs,
            Pane::Likes => &self.likes,
            _ => &[],
        }
    }

    /// Every track the current pane is listing, gathered into one list.
    ///
    /// The history page keeps its tracks with the day each was heard, so it is
    /// the one pane whose rows are not already a plain list of songs.
    fn listed_songs(&self) -> Vec<Song> {
        match self.pane() {
            Pane::History => self.history.songs().cloned().collect(),
            _ => self.visible_songs().to_vec(),
        }
    }

    /// The track the cursor is on, whichever pane is showing.
    fn selected_song(&self) -> Option<Song> {
        match self.pane() {
            Pane::Queue => self.queue.items.get(self.selected).cloned(),
            Pane::History => self.history.songs().nth(self.selected).cloned(),
            _ => self.visible_songs().get(self.selected).cloned(),
        }
    }

    // -----------------------------------------------------------------
    // Playback
    // -----------------------------------------------------------------

    /// Plays the track at `index` of `songs`, making that list the queue.
    ///
    /// Choosing a track out of a list means the queue is that list from that
    /// track on, which is what makes "next" mean what the user sees.
    async fn play_from(&mut self, songs: &[Song], index: usize, tx: &Sender<AppEvent>) {
        if songs.is_empty() {
            return;
        }
        // Only rebuild the queue when it is a different list: playing another
        // track of the same playlist must not throw away what was queued to
        // play next.
        if !self.queue_holds(songs) {
            self.queue.replace_items(songs.to_vec());
        }
        if let Some(song) = self.queue.select(index) {
            self.start_track(song, tx).await;
        }
    }

    fn queue_holds(&self, songs: &[Song]) -> bool {
        self.queue.items.len() == songs.len()
            && self
                .queue
                .items
                .iter()
                .zip(songs)
                .all(|(queued, song)| queued.id == song.id)
    }

    /// Asks the player to start `song`.
    ///
    /// The source has to be resolved first, which is asynchronous; the result
    /// comes back as an event and is only acted on if this is still the track
    /// the user wants.
    async fn start_track(&mut self, song: Song, tx: &Sender<AppEvent>) {
        tracing::info!(song = %song.title, "starting track");
        // Bumped before anything else, so a source still being resolved for the
        // previous track is already stale by the time this one is asked for.
        self.play_generation = self.play_generation.wrapping_add(1);
        let generation = self.play_generation;

        self.current_song = Some(song.clone());
        // Whatever was restored has now been asked for in earnest, so there is
        // nothing left for the play key to resume.
        self.resume_on_play = false;
        self.source = None;
        // Whatever was showing belonged to the track being left behind. The
        // panel says nothing until the new track's lyrics arrive, rather than
        // showing the old song's words under the new song's name.
        self.lyrics = LyricsState::Pending;
        self.playback.position_ms = 0;
        self.playback.duration_ms = song.duration_ms.map(u64::from);
        self.playback.status = PlaybackStatus::Stopped;
        // The pane shows the track that was chosen straight away; its cover
        // arrives when it arrives.
        self.refresh_artwork(tx);

        if self.user.is_none() {
            // Resolving a source needs a session, and asking without one only
            // produces a pointless request. Nothing is playing, so the pane
            // must go back to showing nothing.
            self.notify("尚未登录，无法播放");
            self.current_song = None;
            self.playback.status = PlaybackStatus::Stopped;
            return;
        }

        let provider = self.provider.clone();
        let url_tx = tx.clone();
        let quality = self.config.audio_quality;
        let song_id = song.id;
        self.pending += 1;
        tokio::spawn(async move {
            let result = provider.get_song_url(song_id, quality).await;
            let _ = url_tx
                .send(AppEvent::SongUrl {
                    generation,
                    song,
                    result,
                })
                .await;
        });

        // Lyrics come from an endpoint of their own and do not depend on the
        // source resolving, so they are asked for at the same time rather than
        // once playback starts. A failure here is not fatal and is not announced
        // on the status bar — it happens on every track that has no lyrics.
        let provider = self.provider.clone();
        let lyrics_tx = tx.clone();
        self.pending += 1;
        tokio::spawn(async move {
            let result = provider.get_lyrics(song_id).await;
            let _ = lyrics_tx
                .send(AppEvent::Lyrics {
                    generation,
                    song_id,
                    result,
                })
                .await;
        });
    }

    /// Folds a fetched lyric payload into what the panel can show.
    fn set_lyrics(&mut self, raw: RawLyrics) {
        self.lyrics = match raw {
            // Parsing is the only part that can fail once a body is here, and
            // the body is only parsed once: the position it is read at changes
            // far more often than the words do.
            RawLyrics::Original(body) => match niveus_lyrics::parse(&body) {
                Ok(lyrics) => LyricsState::Ready(lyrics),
                Err(error) => {
                    tracing::warn!(%error, "cannot parse lyrics");
                    LyricsState::Failed
                }
            },
            RawLyrics::TranslationOnly => LyricsState::TranslationOnly,
            RawLyrics::Unavailable => LyricsState::Instrumental,
        };
    }

    /// Hands a resolved source to the player.
    ///
    /// The track itself is already showing: it was set the moment it was
    /// chosen, so the pane does not sit blank while the source is resolved.
    async fn begin_playback(&mut self, source: AudioSource) {
        let Some(player) = &self.player else {
            self.notify("播放器不可用，无法播放");
            self.current_song = None;
            self.playback.status = PlaybackStatus::Stopped;
            return;
        };

        if let Err(error) = player.play(&source.url).await {
            tracing::warn!(%error, "cannot start playback");
            self.notify(format!("播放失败：{error}"));
            self.playback.status = PlaybackStatus::Stopped;
            return;
        }

        // From here on, the player's reports are about this track.
        self.loaded_generation = Some(self.play_generation);

        // The service may have served a short preview rather than the track;
        // saying so is better than letting it end early for no visible reason.
        if source.is_trial {
            self.notify("当前账号只能试听该歌曲的片段");
        }
        self.source = Some(source);
        self.playback.status = PlaybackStatus::Playing;

        // This is the moment a track has been heard (§30): the source resolved
        // and the player took it. Recording it any earlier would list tracks
        // that never sounded.
        if let Some(song) = self.current_song.clone() {
            self.history.record(song, now_seconds());
        }
    }

    /// The play/pause key.
    ///
    /// A track can also be sitting in the pane without ever having been handed
    /// to the player — that is how a session comes back after a restart. For
    /// that one case the key starts it, which is what the user is asking for;
    /// otherwise it means pause.
    async fn start_or_pause(&mut self, tx: &Sender<AppEvent>) {
        if self.resume_on_play {
            if let Some(song) = self.current_song.clone() {
                self.start_track(song, tx).await;
                return;
            }
            self.resume_on_play = false;
        }
        self.toggle_pause().await;
    }

    async fn toggle_pause(&mut self) {
        if self.current_song.is_none() {
            // Nothing has been chosen yet: the key starts the queue instead of
            // doing nothing at all.
            self.notify("请先选择一首歌曲");
            return;
        }
        let Some(player) = &self.player else {
            self.notify("播放器不可用，无法播放");
            return;
        };
        if let Err(error) = player.toggle_pause().await {
            tracing::warn!(%error, "cannot toggle playback");
            self.notify(format!("播放控制失败：{error}"));
        }
    }

    async fn skip_forward(&mut self, tx: &Sender<AppEvent>) {
        match self.queue.advance() {
            Some(song) => self.start_track(song, tx).await,
            None => {
                // The end of the queue is the end of playback.
                self.stop_playback().await;
                self.notify("队列已播放完毕");
            }
        }
    }

    async fn skip_back(&mut self, tx: &Sender<AppEvent>) {
        match self.queue.previous() {
            Some(song) => self.start_track(song, tx).await,
            None => self.notify("已经是队列的第一首"),
        }
    }

    async fn stop_playback(&mut self) {
        // Bumping the generation is also what stops the player's reports from
        // being taken as news about a track: from here on they describe either
        // the track just stopped or one that was never ours.
        self.play_generation = self.play_generation.wrapping_add(1);
        self.playback.status = PlaybackStatus::Stopped;
        self.playback.position_ms = 0;
        self.current_song = None;
        self.source = None;
        self.lyrics = LyricsState::Pending;
        self.artwork = None;
        self.artwork_key = None;
        if let Some(player) = &self.player {
            if let Err(error) = player.stop().await {
                // The pane and the status bar now say nothing is playing; if
                // the player disagrees, only saying so keeps the two honest.
                tracing::warn!(%error, "cannot stop the player");
                self.notify(format!("停止播放失败：{error}"));
            }
        }
    }

    async fn seek(&mut self, seconds: f64) {
        self.move_position(Move::By(seconds)).await
    }

    /// Moves the position relative to where it is.
    async fn jump(&mut self, seconds: f64) {
        self.move_position(Move::By(seconds)).await
    }

    /// Moves the position to a point in the track.
    async fn jump_to(&mut self, seconds: f64) {
        self.move_position(Move::To(seconds)).await
    }

    async fn move_position(&mut self, movement: Move) {
        let Some(player) = &self.player else {
            self.notify("播放器不可用，无法跳转");
            return;
        };
        if self.current_song.is_none() {
            self.notify("请先选择一首歌曲");
            return;
        }

        let result = match movement {
            Move::By(seconds) => player.seek_by(seconds).await,
            Move::To(seconds) => player.seek_to(seconds).await,
        };
        if let Err(error) = result {
            tracing::warn!(%error, "cannot seek");
            self.notify(format!("跳转失败：{error}"));
            return;
        }

        // The position jumped rather than ran on, which the interface asks to
        // be announced differently from the progress it does not announce at
        // all.
        if let Move::To(seconds) = movement {
            self.playback.position_ms = (seconds.max(0.0) * 1000.0) as u64;
        }
        if let Some(service) = &self.mpris {
            if let Err(error) = service.seeked(self.playback.position_ms).await {
                tracing::warn!(%error, "cannot announce a seek");
            }
        }
    }

    /// Sets the volume to a particular level, as the desktop asks.
    async fn set_volume(&mut self, percent: u8) {
        let wanted = percent.min(100);
        let delta = i16::from(wanted) - i16::from(self.playback.volume);
        if delta != 0 {
            self.change_volume(delta).await;
        }
    }

    fn cycle_playback_mode(&mut self) {
        self.queue.playback_mode = self.queue.playback_mode.cycle();
        self.notify(format!("播放模式：{}", self.queue.playback_mode.label()));
    }

    /// Puts the highlighted track next in line, ahead of the rest of the queue.
    fn queue_selected_next(&mut self) {
        match self.selected_song() {
            Some(song) => {
                self.queue.push_next(song.clone());
                self.notify(format!("已加入下一首播放：{}", song.title));
            }
            None => self.notify("没有可加入队列的歌曲"),
        }
    }

    // -----------------------------------------------------------------
    // Queue editing
    // -----------------------------------------------------------------

    fn toggle_queue_view(&mut self) {
        // Opening and closing are the same gesture: the queue replaces the
        // list that is showing, and giving it back is all closing has to do.
        self.queue_view = !self.queue_view;
        self.sync_cursor();
    }

    fn delete_from_queue(&mut self) {
        if self.pane() != Pane::Queue {
            return;
        }
        if self.queue.remove(self.selected).is_some() {
            self.clamp_selection();
        }
    }

    fn reorder_selected(&mut self, delta: isize) {
        if self.pane() != Pane::Queue {
            return;
        }
        let moved = if delta < 0 {
            self.queue.move_up(self.selected)
        } else {
            self.queue.move_down(self.selected)
        };
        if moved {
            // The cursor travels with the track it is holding.
            self.selected = (self.selected as isize + delta) as usize;
            self.keep_selection_visible();
        }
    }

    // -----------------------------------------------------------------
    // Settings
    // -----------------------------------------------------------------

    /// Which setting the arrows are changing, if any.
    ///
    /// Only while the settings page is what is showing: the queue takes the
    /// pane over, and an edit left open underneath it would have the arrows
    /// changing a value the user cannot see.
    fn editing_setting(&self) -> Option<Setting> {
        if self.queue_view {
            return None;
        }
        match self.settings_mode {
            SettingsMode::Editing(setting) => Some(setting),
            _ => None,
        }
    }

    /// What `Enter` does on the settings page.
    ///
    /// The four value rows are edited in place: `Enter` picks one up, the
    /// arrows change it and `Enter` (or `Esc`) puts it down. Keybindings are
    /// not a value but a list, so `Enter` goes into it instead.
    fn activate_setting(&mut self) {
        if matches!(
            self.settings_mode,
            SettingsMode::Keybindings | SettingsMode::Capturing
        ) {
            self.settings_mode = SettingsMode::Capturing;
            self.notify("按下要绑定的按键，Esc 取消");
            return;
        }
        let Some(setting) = Setting::at(self.selected) else {
            return;
        };
        match setting {
            Setting::Keybindings => {
                self.settings_mode = SettingsMode::Keybindings;
                self.sync_cursor();
            }
            other => {
                // Enter again puts the value down: one key, both ways, so
                // there is no mode to be stuck in.
                if self.editing_setting() == Some(other) {
                    self.settings_mode = SettingsMode::Browse;
                    self.notify(format!("{}：{}", other.label(), self.setting_value(other)));
                } else {
                    self.settings_mode = SettingsMode::Editing(other);
                }
            }
        }
    }

    /// The value a setting is showing, in the words the screen uses.
    ///
    /// This is the app's answer to the same question [`SettingsValues`] answers
    /// for the renderer, and the two are asked at different moments: the
    /// renderer wants it every frame, this one when a change is announced.
    fn setting_value(&self, setting: Setting) -> String {
        match setting {
            Setting::AudioQuality => self.config.audio_quality.label().to_string(),
            Setting::PlaybackMode => self.queue.playback_mode.label().to_string(),
            Setting::Volume => format!("{}%", self.playback.volume),
            Setting::CoverSize => self.config.cover_size.name().to_string(),
            // Not a value but a list of keys, and it is never edited in place,
            // so nothing ever asks for its value.
            Setting::Keybindings => String::new(),
        }
    }

    /// Steps the setting being edited by `delta`.
    ///
    /// Each setting is written where it belongs the moment it changes: the two
    /// that are preferences go to `config.toml` and the two that are runtime
    /// state go to `state.json` (§26). Nothing here writes the other one.
    async fn adjust_setting(&mut self, delta: isize, tx: &Sender<AppEvent>) {
        let Some(setting) = self.editing_setting() else {
            return;
        };
        match setting {
            Setting::AudioQuality => {
                let next = self.config.audio_quality.step(delta);
                if next != self.config.audio_quality {
                    self.config.audio_quality = next;
                    self.persist_config();
                }
            }
            Setting::PlaybackMode => {
                let next = self.queue.playback_mode.step(delta);
                if next != self.queue.playback_mode {
                    self.queue.playback_mode = next;
                    self.persist_state();
                }
            }
            Setting::Volume => {
                let before = self.playback.volume;
                self.change_volume(delta as i16 * VOLUME_STEP).await;
                if self.playback.volume != before {
                    self.persist_state();
                }
            }
            Setting::CoverSize => {
                let next = self.config.cover_size.step(delta);
                if next != self.config.cover_size {
                    self.config.cover_size = next;
                    // The pane is a different shape now, so the cover is a
                    // different picture and has to be drawn again at that size.
                    self.refresh_artwork(tx);
                    self.persist_config();
                }
            }
            // A list, not a value: the arrows move within it as usual.
            Setting::Keybindings => {}
        }
    }

    /// Binds the key that was just pressed to the highlighted action.
    ///
    /// A key another action already holds is refused and said so, and nothing
    /// is written: a keymap with two actions on one key has no meaning, so the
    /// only alternatives are to refuse or to steal, and stealing silently is
    /// how a user ends up with a binding they never made (§25).
    fn capture_binding(&mut self, key: crossterm::event::KeyEvent) {
        if key.code == crossterm::event::KeyCode::Esc {
            self.settings_mode = SettingsMode::Keybindings;
            self.notify("已取消改键");
            return;
        }

        let Some(action) = Action::ALL.get(self.selected).copied() else {
            self.settings_mode = SettingsMode::Keybindings;
            return;
        };

        // Ctrl+C never reaches here: it is answered above, before the keymap
        // and before this prompt. Nothing else in the vocabulary has to be
        // held back, so what is left can simply be bound.
        let Some(key) = niveus_ui::to_key(key) else {
            // Alt combinations and media keys are not in the vocabulary; the
            // editor stays open so the user can simply press another key.
            self.notify("这个按键无法绑定，请换一个");
            return;
        };

        match self.config.keymap.rebind(action, key) {
            Ok(()) => {
                self.settings_mode = SettingsMode::Keybindings;
                self.notify(format!("{} 已绑定到 {}", action.name(), key.label()));
                // Written down only now, when the map is known to be valid.
                self.persist_config();
            }
            Err(error) => {
                self.settings_mode = SettingsMode::Keybindings;
                self.notify(format!("无法绑定：{error}"));
            }
        }
    }

    /// Writes the preferences back, so a change survives a restart.
    fn persist_config(&mut self) {
        let path = self.paths.config_file();
        match self.config.save(&path) {
            Ok(()) => tracing::info!(path = %path.display(), "preferences saved"),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "cannot save preferences");
                self.notify("设置无法保存，下次启动会恢复原值");
            }
        }
    }

    fn clamp_selection(&mut self) {
        let count = self.list_view().item_count();
        self.selected = self.selected.min(count.saturating_sub(1));
        self.keep_selection_visible();
    }

    /// Pulls the cursor back inside a list that has just arrived.
    ///
    /// Only for the pane that list belongs to. A remembered position — from
    /// `state.json`, or from earlier in the session — can be past the end of a
    /// list that has since shrunk, and measuring it against whatever pane
    /// happens to be showing would measure the wrong list: the track list is
    /// still empty at the moment the playlist list arrives.
    fn clamp_if_showing(&mut self, pane: Pane) {
        if self.pane() == pane {
            self.clamp_selection();
        }
    }

    // -----------------------------------------------------------------
    // Album art
    // -----------------------------------------------------------------

    /// Where a cover would be drawn, if there is anywhere to draw one.
    fn artwork_region(&self) -> Option<Rect> {
        niveus_ui::compute(
            Rect::new(0, 0, self.size.0.max(1), self.size.1.max(1)),
            self.tier(),
            self.config.cover_size,
        )
        .artwork
    }

    /// Brings the picture in the now-playing pane up to date with the track and
    /// the size of the pane.
    ///
    /// Called whenever either changes, and at no other time: a cover is decoded
    /// and sampled once and then kept, which is what makes the pane cost
    /// nothing while a track plays.
    pub fn refresh_artwork(&mut self, tx: &Sender<AppEvent>) {
        let Some(region) = self.artwork_region() else {
            // No pane to draw in: nothing to fetch and nothing to keep.
            self.artwork = None;
            self.artwork_key = None;
            return;
        };
        if self.current_song.is_none() {
            self.artwork = None;
            self.artwork_key = None;
            return;
        }

        let area = cover::area(region);
        let key = self.current_song.as_ref().and_then(|song| {
            song.cover_url().map(|url| {
                CoverKey::new(
                    url,
                    area,
                    cover::options(self.config.artwork_mode, self.colour_depth),
                )
            })
        });

        let Some(key) = key else {
            // The track has no cover. The default sign is not an error state,
            // and drawing it costs nothing, so it is drawn here and now.
            self.artwork_key = None;
            self.artwork = Some(cover::view(&niveus_artwork::render_default(area)));
            return;
        };

        if self.artwork_key.as_ref() == Some(&key) {
            return;
        }
        self.artwork_key = Some(key.clone());

        // A size or a track that has already been drawn is drawn no again.
        if let Some(frame) = self.artwork_cache.get(&key) {
            self.artwork = Some(cover::view(&frame));
            return;
        }

        // Nothing to show until the picture arrives; the pane stays blank
        // rather than claiming a cover it does not have.
        self.artwork = None;
        self.pending += 1;
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = niveus_artwork::load(&key).await;
            let _ = tx.send(AppEvent::Cover { key, result }).await;
        });
    }

    /// Folds an arriving cover into the state.
    async fn set_cover(&mut self, key: CoverKey, result: Result<Frame, ArtworkError>) {
        self.pending = self.pending.saturating_sub(1);

        // A picture for a track or a size the pane has since left behind is
        // dropped rather than drawn over what is there now.
        if self.artwork_key.as_ref() != Some(&key) {
            tracing::debug!("a cover for a track that has moved on was dropped");
            return;
        }

        let frame = match result {
            Ok(frame) => {
                self.artwork_cache.put(key, frame.clone());
                frame
            }
            Err(error) => {
                // A cover that cannot be had is the same as no cover: the
                // default sign, which the design asks for by name.
                //
                // Deliberately not cached: what is being shown is a stand-in
                // for a picture that *does* exist, and a network that stumbled
                // once should not turn into a track that never has a cover.
                tracing::warn!(url = %key.url, %error, "cannot load a cover");
                niveus_artwork::render_default(key.area)
            }
        };

        self.artwork = Some(cover::view(&frame));
    }

    /// Waits for the terminal to stop being resized before redoing the work a
    /// resize costs.
    ///
    /// A drag of the window's edge produces a stream of sizes; redrawing the
    /// cover for each of them would decode the same picture twenty times and
    /// show none of them.
    fn note_resize(&mut self) {
        self.resize_at = Some(Instant::now());
    }

    fn resize_settled(&mut self, tx: &Sender<AppEvent>) {
        const SETTLE: Duration = Duration::from_millis(100);
        let Some(at) = self.resize_at else {
            return;
        };
        if at.elapsed() < SETTLE {
            return;
        }
        self.resize_at = None;
        self.keep_selection_visible();
        // The pane may now be a different size, which is a different picture.
        self.refresh_artwork(tx);
    }

    fn open_playlist(&mut self, playlist: Playlist, tx: &Sender<AppEvent>) {
        self.opened_playlist = Some(playlist.clone());
        self.begin_context();
        // A playlist already visited opens where it was left; one never opened
        // starts at the top, which is what `sync_cursor` finds for it.
        self.sync_cursor();
        self.songs.clear();
        self.request_songs(tx, playlist.id);
    }

    /// Volume is clamped rather than wrapped, and stops at both ends.
    async fn change_volume(&mut self, delta: i16) {
        let previous = self.playback.volume;
        let next = (i16::from(previous) + delta).clamp(0, 100);
        self.playback.volume = next as u8;

        let Some(player) = &self.player else {
            // No player to disagree with: the value is a preference, kept for
            // whenever one starts.
            return;
        };
        if let Err(error) = player.set_volume(self.playback.volume).await {
            // The player never took the change, so the display must not go on
            // claiming it did: the readout would disagree with what is coming
            // out of the speakers and nothing would ever correct it.
            tracing::warn!(%error, "cannot set the volume");
            self.playback.volume = previous;
            self.notify(format!("音量调整失败：{error}"));
        }
    }

    // -----------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------

    pub async fn handle_event(&mut self, event: AppEvent, tx: &Sender<AppEvent>) {
        self.handle_event_inner(event, tx).await;
        self.publish_mpris().await;
    }

    async fn handle_event_inner(&mut self, event: AppEvent, tx: &Sender<AppEvent>) {
        match event {
            AppEvent::Key(key) => {
                // Checked before anything else, and before the keymap is even
                // consulted: Ctrl+C is the one way out that always works, on
                // any page and in the middle of anything — including the key
                // prompt, which would otherwise be a place a user could be
                // stuck in. It is deliberately not rebindable (§24).
                if is_ctrl_c(key) {
                    self.quit = true;
                    return;
                }
                // While a rebind is waiting for a key, the key is the answer
                // rather than a command: pressing `q` here binds `q`, and does
                // not quit.
                if self.settings_mode == SettingsMode::Capturing {
                    self.capture_binding(key);
                    return;
                }
                if let Some(action) = niveus_ui::map_key(key, &self.config.keymap) {
                    self.handle_action(action, tx).await;
                } else if key.code == crossterm::event::KeyCode::Backspace {
                    // Backspace is accepted alongside the configured key for
                    // going back, as the browse design offers both.
                    self.handle_action(Action::Back, tx).await;
                }
            }
            AppEvent::Tick => {
                self.expire_notices();
                self.resize_settled(tx);
                // Held back rather than shown over whatever is on screen: these
                // are the least urgent things the program has to say, and they
                // would otherwise wipe out a message about the player having
                // failed to start, which the user needs far more. One per idle
                // tick, so several of them can all be read.
                if self.notice.is_none() {
                    if let Some(warning) = self.warnings.pop_front() {
                        self.notify(warning);
                    }
                }
            }
            AppEvent::Resize => self.note_resize(),
            AppEvent::Interrupt => {
                tracing::info!("interrupted, shutting down");
                self.quit = true;
            }
            AppEvent::SessionRestored(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(Some(user)) => self.sign_in(user, tx),
                    Ok(None) => {
                        tracing::info!("no valid session, starting QR sign-in");
                        self.request_qr(tx);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "session check failed");
                        self.login = LoginPhase::Failed(format!("无法校验登录状态：{error}"));
                    }
                }
            }
            AppEvent::QrStarted(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(login) => match niveus_ui::qr::encode(&login.url) {
                        Ok(grid) => {
                            self.qr = Some(grid);
                            self.login = LoginPhase::Waiting;
                            self.spawn_qr_poll(tx, login.key);
                        }
                        Err(error) => {
                            tracing::warn!(%error, "cannot render QR code");
                            self.login = LoginPhase::Failed(format!("无法生成二维码：{error}"));
                        }
                    },
                    Err(error) => {
                        tracing::warn!(%error, "cannot obtain QR code");
                        self.login = LoginPhase::Failed(format!("无法获取二维码：{error}"));
                    }
                }
            }
            AppEvent::QrPolled(result) => match result {
                Ok(QrLoginState::Waiting) => self.login = LoginPhase::Waiting,
                Ok(QrLoginState::Scanned) => self.login = LoginPhase::Scanned,
                Ok(QrLoginState::Confirmed) => {
                    self.login = LoginPhase::Scanned;
                    // Save the session the moment it is granted, before it is
                    // checked. If the check then fails, the scan has not been
                    // wasted and the session can still be examined.
                    self.persist_cookie();
                    self.spawn_qr_finish(tx);
                }
                Ok(QrLoginState::Expired) => {
                    tracing::info!("QR code expired, requesting another");
                    self.request_qr(tx);
                }
                Err(error) => {
                    tracing::warn!(%error, "QR poll failed");
                    self.login = LoginPhase::Failed(format!("扫码失败：{error}"));
                }
            },
            AppEvent::QrConfirmed(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(user) => self.sign_in(user, tx),
                    Err(error) => {
                        tracing::warn!(%error, "sign-in could not be confirmed");
                        self.login = LoginPhase::Failed(format!("登录确认失败：{error}"));
                    }
                }
            }
            AppEvent::Playlists(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(playlists) => self.set_playlists(playlists, tx),
                    Err(error) => {
                        // There is nothing to reopen without the list, and
                        // coming back to a playlist that cannot be found would
                        // be a puzzle rather than a position.
                        self.pending_playlist = None;
                        self.report_browse_failure("加载歌单失败", error, tx)
                    }
                }
            }
            AppEvent::Songs {
                generation,
                playlist_id,
                result,
            } => {
                // A reply for a context the user has left is dropped, not shown.
                if generation != self.generation {
                    tracing::debug!(
                        generation,
                        current = self.generation,
                        "stale track reply dropped"
                    );
                    return;
                }
                self.pending = self.pending.saturating_sub(1);
                if self.opened_playlist.as_ref().map(|p| p.id) != Some(playlist_id) {
                    return;
                }
                match result {
                    Ok(songs) => {
                        self.songs = songs;
                        // The remembered position may now be past the end.
                        self.clamp_selection();
                    }
                    Err(error) => self.report_browse_failure("加载歌曲失败", error, tx),
                }
            }
            AppEvent::Likes(result) => {
                self.pending = self.pending.saturating_sub(1);
                match result {
                    Ok(songs) => {
                        self.likes = songs;
                        self.likes_loaded = true;
                        // As with the playlist list: a remembered position can
                        // now be past the end.
                        self.clamp_if_showing(Pane::Likes);
                    }
                    Err(error) => self.report_browse_failure("加载喜欢的歌曲失败", error, tx),
                }
            }
            AppEvent::SongUrl {
                generation,
                song,
                result,
            } => {
                // Counted in before the check below, not after: a reply that
                // is dropped is still a reply that arrived, and leaving it out
                // would keep the status bar saying LOADING… for ever.
                self.pending = self.pending.saturating_sub(1);
                // A source for a track the user has already left behind is
                // dropped rather than played over the one they chose.
                if generation != self.play_generation {
                    tracing::debug!(generation, "stale audio source dropped");
                    return;
                }
                match result {
                    Ok(source) => self.begin_playback(source).await,
                    Err(error) => {
                        tracing::warn!(%error, "cannot resolve an audio source");
                        self.notify(format!("无法播放《{}》：{error}", song.title));
                        // Nothing is playing, so the pane must not keep showing
                        // the track as if it were.
                        self.current_song = None;
                        self.playback.status = PlaybackStatus::Stopped;
                    }
                }
            }
            AppEvent::Lyrics {
                generation,
                song_id,
                result,
            } => {
                // Counted in before the guard, for the same reason the audio
                // source reply is: a reply that is dropped still arrived, and
                // leaving it out would keep the bar saying LOADING… for ever.
                self.pending = self.pending.saturating_sub(1);
                if generation != self.play_generation {
                    tracing::debug!(generation, "stale lyrics dropped");
                    return;
                }
                match result {
                    Ok(raw) => self.set_lyrics(raw),
                    Err(error) => {
                        // The reason goes to the log; the panel says only that
                        // the lyrics cannot be used (SPEC §13.4, §33).
                        tracing::warn!(song = song_id.get(), %error, "cannot fetch lyrics");
                        self.lyrics = LyricsState::Failed;
                    }
                }
            }
            AppEvent::Player(event) => self.handle_player(event, tx).await,
            AppEvent::Cover { key, result } => self.set_cover(key, result).await,
            AppEvent::Mpris(command) => self.handle_mpris(command, tx).await,
        }
    }

    /// Acts on what the desktop asked for.
    ///
    /// Every one of these is the same thing the matching key does — the
    /// desktop is another way of pressing the buttons, not another kind of
    /// authority over the state.
    async fn handle_mpris(&mut self, command: MprisCommand, tx: &Sender<AppEvent>) {
        match command {
            MprisCommand::PlayPause => self.toggle_pause().await,
            MprisCommand::Play => match self.playback.status {
                PlaybackStatus::Playing => {}
                PlaybackStatus::Paused => self.toggle_pause().await,
                // Stopped: either a track is loaded and waiting to be started
                // again, or the queue has not begun.
                PlaybackStatus::Stopped => match self.current_song.clone() {
                    Some(song) => self.start_track(song, tx).await,
                    None => self.skip_forward(tx).await,
                },
            },
            MprisCommand::Pause => {
                if self.playback.status == PlaybackStatus::Playing {
                    self.toggle_pause().await;
                }
            }
            MprisCommand::Stop => self.stop_playback().await,
            MprisCommand::Next => self.skip_forward(tx).await,
            MprisCommand::Previous => self.skip_back(tx).await,
            // The interface counts microseconds; Niveus counts seconds.
            MprisCommand::Seek(offset) => self.jump(offset as f64 / 1_000_000.0).await,
            MprisCommand::SetPosition(position) => {
                self.jump_to(position as f64 / 1_000_000.0).await
            }
            MprisCommand::SetVolume(percent) => self.set_volume(percent).await,
            MprisCommand::Quit => self.quit = true,
        }
    }

    /// Publishes what is playing, when it differs from what was published.
    ///
    /// Called after every event: the service itself works out what actually
    /// changed, so a progress tick reaches the mutex and stops there rather
    /// than waking the desktop several times a second.
    async fn publish_mpris(&mut self) {
        let Some(service) = &self.mpris else {
            return;
        };
        let snapshot = Snapshot {
            status: self.playback.status,
            song: self.current_song.clone(),
            position_ms: self.playback.position_ms,
            volume: self.playback.volume,
        };
        if snapshot == self.published {
            return;
        }
        if let Err(error) = service.update(snapshot.clone()).await {
            tracing::warn!(%error, "cannot tell the desktop what is playing");
            return;
        }
        self.published = snapshot;
    }

    /// Folds what the player reports into the state.
    /// Starts the player process and wires its reports into the event loop.
    ///
    /// Start-up and recovery are the same operation, so they share one
    /// definition: when the player dies under the program, this is what puts
    /// another one in its place, and nothing about that path can drift away
    /// from how the first one was started.
    pub async fn launch_player(&mut self, tx: &Sender<AppEvent>) -> Result<(), PlayerError> {
        let (player_tx, mut player_rx) = mpsc::channel(64);
        let options = LaunchOptions {
            // Starts at the volume the interface shows, rather than at the
            // player's own idea of a sensible default.
            volume: Some(self.playback.volume),
            ..LaunchOptions::default()
        };

        let player = Player::launch(&self.paths.socket_file(), options, player_tx).await?;
        self.player = Some(player);
        self.player_started_at = Some(Instant::now());

        // The player reports on its own channel; the interface listens on one.
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some(event) = player_rx.recv().await {
                if tx.send(AppEvent::Player(event)).await.is_err() {
                    return;
                }
            }
        });
        Ok(())
    }

    /// What to do when the player process goes away under the program.
    ///
    /// Browsing, the queue, the settings and quitting never depended on the
    /// player and carry on. What has to be re-established is playback: the
    /// dead handle is dropped first, so the next command gets the one clear
    /// "播放器不可用" answer instead of a fresh error from a corpse, and a
    /// replacement is started — unless the last few died as soon as they
    /// started, in which case another one will not help either.
    async fn player_exited(&mut self, tx: &Sender<AppEvent>) {
        let had_player = self.player.take().is_some();
        tracing::warn!(had_player, "the player exited");

        self.playback.status = PlaybackStatus::Stopped;
        self.playback.position_ms = 0;
        // The player no longer has the track, but the track is still known —
        // which is exactly the state the play key resumes from. Without this
        // the key would toggle the pause of a player that holds nothing, and
        // the sound would never come back.
        if self.current_song.is_some() {
            self.resume_on_play = true;
        }
        self.loaded_generation = None;

        // Nothing was running: a player already given up on, or one whose
        // exit report has outlived it. Restarting here would undo the giving
        // up, so it is not done.
        if !had_player {
            return;
        }

        let quick = self
            .player_started_at
            .is_some_and(|at| at.elapsed() < PLAYER_RESTART_WINDOW);
        self.player_restarts = if quick { self.player_restarts + 1 } else { 0 };
        if self.player_restarts >= MAX_PLAYER_RESTARTS {
            tracing::error!(restarts = self.player_restarts, "the player keeps exiting");
            self.notify("播放器反复退出，播放功能已停用；重启 Niveus 可恢复");
            return;
        }

        match self.launch_player(tx).await {
            Ok(()) => {
                tracing::info!("player restarted");
                self.notify("播放器已退出，已重新启动；按 Space 继续播放");
            }
            Err(error) => {
                tracing::error!(%error, "cannot restart the player");
                self.notify(format!("播放器已退出，无法重新启动：{error}"));
            }
        }
    }

    async fn handle_player(&mut self, event: PlayerEvent, tx: &Sender<AppEvent>) {
        // Everything except the volume describes the track the player was last
        // handed; the volume belongs to the player itself, and so does the news
        // that the player itself is gone. An exit report is the one thing that
        // must never be dropped as stale: dropped, the program would go on
        // talking to a process that is not there.
        if !matches!(event, PlayerEvent::Volume(_) | PlayerEvent::Exited)
            && !self.report_is_current()
        {
            tracing::debug!(?event, "a report about a track that has been replaced");
            return;
        }

        match event {
            PlayerEvent::Loaded => {
                if self.current_song.is_some() {
                    self.playback.status = PlaybackStatus::Playing;
                }
            }
            PlayerEvent::Ended => {
                // A track that ran out is what moves the queue on — but only
                // the track the player was last handed. An end that was already
                // on its way when the user chose another track must not be
                // counted twice, or one keypress would skip two.
                if self.current_song.is_some() {
                    self.skip_forward(tx).await;
                }
            }
            PlayerEvent::Failed(message) => {
                tracing::warn!(%message, "playback failed");
                self.playback.status = PlaybackStatus::Stopped;
                self.notify(format!("播放失败：{message}"));
            }
            PlayerEvent::Position(seconds) => {
                if let Some(ms) = seconds_to_ms(seconds) {
                    self.playback.position_ms = ms;
                }
            }
            PlayerEvent::Duration(seconds) => {
                self.playback.duration_ms = seconds_to_ms(seconds);
            }
            PlayerEvent::Paused(paused) => {
                // The player reports its pause state on connect too, before
                // anything is playing; that must not read as playback.
                if self.current_song.is_some() {
                    self.playback.status = if paused {
                        PlaybackStatus::Paused
                    } else {
                        PlaybackStatus::Playing
                    };
                }
            }
            PlayerEvent::Volume(volume) => {
                if volume.is_finite() {
                    self.playback.volume = volume.round().clamp(0.0, 100.0) as u8;
                }
            }
            PlayerEvent::Exited => self.player_exited(tx).await,
        }
    }

    /// Whether the player's reports are about the track playing now.
    ///
    /// False while another track has been chosen but not yet handed over —
    /// the window in which a report about the old track is still in flight —
    /// and false while nothing has been handed over at all, which is how a
    /// restored session and a player that has just connected both look.
    fn report_is_current(&self) -> bool {
        self.loaded_generation == Some(self.play_generation)
    }

    fn sign_in(&mut self, user: User, tx: &Sender<AppEvent>) {
        tracing::info!(user = %user.name, "signed in");
        self.user = Some(user);
        self.qr = None;
        self.login = LoginPhase::Waiting;
        self.persist_cookie();
        self.begin_context();
        self.request_playlists(tx);
    }

    fn set_playlists(&mut self, playlists: Vec<Playlist>, tx: &Sender<AppEvent>) {
        self.created = playlists
            .iter()
            .filter(|p| p.kind == PlaylistKind::Created)
            .cloned()
            .collect();
        self.collected = playlists
            .iter()
            .filter(|p| p.kind == PlaylistKind::Collected)
            .cloned()
            .collect();
        tracing::info!(
            created = self.created.len(),
            collected = self.collected.len(),
            "playlists loaded"
        );

        // The remembered cursor may point past the end of a list that has since
        // lost playlists. Done before reopening anything, while the pane is
        // still the playlist list the cursor belongs to.
        self.clamp_if_showing(Pane::Playlists);

        // A session that had a playlist open comes back to it — but only once
        // the list is here to say whether it still exists. It may not: a
        // playlist can be deleted on another device between one run and the
        // next, and the answer to that is the playlist list, not an error.
        let Some(pending) = self.pending_playlist.take() else {
            return;
        };
        let playlist = self
            .created
            .iter()
            .chain(self.collected.iter())
            .find(|playlist| playlist.id == pending)
            .cloned();

        match playlist {
            Some(playlist) => {
                tracing::info!(playlist = %playlist.name, "reopening the playlist from last time");
                self.open_playlist(playlist, tx);
            }
            None => {
                tracing::info!(
                    playlist = pending.get(),
                    "the playlist from last time is gone"
                );
                self.notify("上次浏览的歌单已不存在");
            }
        }
    }

    /// Writes the session cookie so the next start does not need a scan.
    fn persist_cookie(&self) {
        let path = self.paths.cookie_file();
        let cookie = self.provider.cookie();
        if cookie.is_empty() {
            return;
        }
        if let Err(error) = write_private(&path, &cookie) {
            tracing::warn!(path = %path.display(), %error, "cannot save session cookie");
            return;
        }
        tracing::info!(path = %path.display(), "session cookie saved");
    }

    fn report_browse_failure(&mut self, what: &str, error: ProviderError, tx: &Sender<AppEvent>) {
        tracing::warn!(%error, "{what}");
        if !error.is_authentication() {
            self.notify(format!("{what}：{error}"));
            return;
        }

        // The session died under us. Telling the user to scan again is not
        // enough on its own — a code has to actually be asked for, or the
        // sign-in screen sits on "正在获取二维码…" for ever and the only way
        // out is to restart the program.
        self.user = None;
        self.notify("登录已过期，请重新扫码");
        self.begin_context();
        self.request_qr(tx);
    }
}

/// How a position is being moved.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Move {
    /// By a number of seconds, either way.
    By(f64),
    /// To a point in the track.
    To(f64),
}

/// Seconds as whole milliseconds, ignoring values that are not positions.
///
/// The player reports `None` for a property it has no value for, and a track
/// that has not started has a position of zero — both of which are simply not
/// an update.
fn seconds_to_ms(seconds: f64) -> Option<u64> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some((seconds * 1000.0) as u64)
}

/// The current moment, in seconds since the Unix epoch.
///
/// A clock that cannot be read — before 1970, or a system with no time — gives
/// zero rather than an error: a history entry with an implausible date is a
/// smaller problem than a track that cannot be recorded at all.
fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn is_ctrl_c(key: crossterm::event::KeyEvent) -> bool {
    key.code == crossterm::event::KeyCode::Char('c')
        && key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
}

/// Writes a file only the owner can read.
fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use niveus_config::{CoverSize, Key, Keymap};
    use niveus_music_core::{AudioQuality, PlaylistId, SongId};
    use niveus_ui::LayoutTier;

    fn playlist(id: u64, kind: PlaylistKind) -> Playlist {
        Playlist {
            id: PlaylistId(id),
            name: format!("list {id}"),
            kind,
        }
    }

    #[test]
    fn playlists_are_split_by_how_the_user_got_them() {
        let mut state = test_state();
        let tx = channel();
        state.set_playlists(
            vec![
                playlist(1, PlaylistKind::Created),
                playlist(2, PlaylistKind::Collected),
                playlist(3, PlaylistKind::Created),
            ],
            &tx,
        );
        assert_eq!(state.created.len(), 2);
        assert_eq!(state.collected.len(), 1);
    }

    #[test]
    fn selection_is_clamped_to_the_list() {
        let mut state = test_state();
        state.created = vec![
            playlist(1, PlaylistKind::Created),
            playlist(2, PlaylistKind::Created),
        ];

        state.move_selection(1);
        assert_eq!(state.selected, 1);
        // Past the end stays at the end.
        state.move_selection(1);
        assert_eq!(state.selected, 1);
        state.move_selection(-5);
        assert_eq!(state.selected, 0);
    }

    #[tokio::test]
    async fn each_list_remembers_where_its_cursor_was() {
        // The documented case: move down a playlist, go elsewhere, come back —
        // still in that playlist, still on that track.
        let mut state = signed_in();
        let tx = channel();

        let first = playlist(1, PlaylistKind::Created);
        let second = playlist(2, PlaylistKind::Created);
        state.created = vec![first.clone(), second.clone()];

        state.opened_playlist = Some(first.clone());
        state.songs = (1..=40).map(song).collect();
        state.current_list = ListId::Playlist(first.id);
        state.move_selection(1);
        for _ in 1..30 {
            state.move_selection(1);
        }
        assert_eq!(state.selected, 30);

        // Off to another page, and to another playlist, and back again.
        state.go_to_page(Page::Favorites, &tx);
        assert_ne!(
            state.selected, 30,
            "the favourites list kept the playlist's cursor"
        );

        state.go_to_page(Page::Playlist, &tx);
        assert_eq!(
            state.opened_playlist.as_ref().map(|p| p.id),
            Some(first.id),
            "coming back to PLAYLIST left the open playlist"
        );
        assert_eq!(
            state.selected, 30,
            "the playlist did not remember its cursor"
        );
    }

    #[tokio::test]
    async fn two_playlists_do_not_share_a_cursor() {
        let mut state = signed_in();
        let tx = channel();

        let first = playlist(1, PlaylistKind::Created);
        let second = playlist(2, PlaylistKind::Created);
        state.created = vec![first.clone(), second.clone()];
        state.songs = (1..=40).map(song).collect();

        state.open_playlist(first.clone(), &tx);
        state.songs = (1..=40).map(song).collect();
        for _ in 0..7 {
            state.move_selection(1);
        }
        assert_eq!(state.selected, 7);

        // Open the other playlist: it starts at the top.
        state.open_playlist(second.clone(), &tx);
        state.songs = (1..=40).map(song).collect();
        assert_eq!(state.selected, 0, "the second playlist inherited a cursor");

        for _ in 0..3 {
            state.move_selection(1);
        }

        // Each keeps its own.
        state.open_playlist(first.clone(), &tx);
        assert_eq!(state.selected, 7);
        state.open_playlist(second.clone(), &tx);
        assert_eq!(state.selected, 3);
    }

    #[test]
    fn the_queue_view_remembers_where_the_cursor_was() {
        // Opening the queue puts the cursor at its own top, not wherever the
        // list underneath happened to be; closing it puts the cursor back.
        let mut state = test_state();
        state.queue.replace_items((1..=5).map(song).collect());
        state.created = (1..=9)
            .map(|id| playlist(id, PlaylistKind::Created))
            .collect();

        state.selected = 3;
        state.toggle_queue_view();
        assert!(state.queue_view);
        assert_eq!(state.selected, 0, "the queue opened on the list's cursor");

        state.selected = 2;
        state.toggle_queue_view();
        assert!(!state.queue_view);
        assert_eq!(state.selected, 3, "the list did not remember its cursor");

        state.toggle_queue_view();
        assert_eq!(state.selected, 2, "the queue did not remember its cursor");
    }

    #[tokio::test]
    async fn a_remembered_cursor_is_pulled_back_inside_a_shorter_list() {
        let mut state = signed_in();
        let tx = channel();
        let list = playlist(1, PlaylistKind::Created);
        state.created = vec![list.clone()];

        state.open_playlist(list.clone(), &tx);
        state.songs = (1..=20).map(song).collect();
        state.selected = 15;

        state.go_to_page(Page::Settings, &tx);
        state.go_to_page(Page::Playlist, &tx);
        // The playlist comes back shorter than when it was left.
        state.songs = (1..=3).map(song).collect();
        state.clamp_selection();

        assert_eq!(state.selected, 2, "the cursor was left past the end");
    }

    #[test]
    fn the_selected_playlist_is_always_drawn() {
        // The playlist pane heads its two groups, and those rows come off the
        // height. Counting them as entries would scroll the window one heading
        // too tall and leave the cursor below the bottom of the pane.
        // Signed in: without a session the renderer draws the sign-in page.
        let mut state = signed_in();
        state.size = (120, 40);
        state.created = (1..=40)
            .map(|id| playlist(id, PlaylistKind::Created))
            .collect();
        // Every position, not just the last: the window that overruns the pane
        // is the one whose last row is the cursor, and which position that is
        // depends on how many heading rows fall inside it.
        for selected in 0..state.created.len() {
            state.selected = selected;
            state.keep_selection_visible();

            let area = Rect::new(0, 0, 120, 40);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, area.height))
                    .unwrap();
            let ui = state.ui_state();
            terminal
                .draw(|frame| niveus_ui::render(frame, &ui, &niveus_ui::Theme::default()))
                .unwrap();

            let buffer = terminal.backend().buffer();
            let drawn: String = (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buffer.cell((x, y)).unwrap().symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");

            assert!(
                drawn.contains(&format!("▶ list {}", selected + 1)),
                "playlist {} is not on screen",
                selected + 1
            );
        }
    }

    #[test]
    fn an_empty_list_ignores_movement() {
        let mut state = test_state();
        state.move_selection(1);
        assert_eq!(state.selected, 0);
    }

    #[tokio::test]
    async fn volume_is_clamped_at_both_ends() {
        let mut state = test_state();
        state.playback.volume = 98;
        state.change_volume(5).await;
        assert_eq!(state.playback.volume, 100);
        state.change_volume(5).await;
        assert_eq!(state.playback.volume, 100);
        state.playback.volume = 3;
        state.change_volume(-5).await;
        assert_eq!(state.playback.volume, 0);
    }

    #[test]
    fn changing_context_invalidates_outstanding_replies() {
        let mut state = test_state();
        let before = state.generation;
        state.begin_context();
        assert!(state.generation > before);
        // A cancelled context reports nothing as loading.
        assert_eq!(state.pending, 0);
    }

    #[test]
    fn notices_expire() {
        let mut state = test_state();
        state.notify("hello");
        assert!(state.notice.is_some());
        // Force expiry rather than sleeping.
        state.notice.as_mut().unwrap().until = Instant::now() - Duration::from_secs(1);
        state.expire_notices();
        assert!(state.notice.is_none());
    }

    #[tokio::test]
    async fn a_reply_for_an_abandoned_playlist_is_discarded() {
        let mut state = test_state();
        state.opened_playlist = Some(playlist(7, PlaylistKind::Created));
        let generation = state.generation;

        let tx = channel();
        state
            .handle_event(
                AppEvent::Songs {
                    generation: generation.wrapping_sub(1),
                    playlist_id: PlaylistId(7),
                    result: Ok(vec![song(1)]),
                },
                &tx,
            )
            .await;
        assert!(state.songs.is_empty(), "a stale reply was applied");
    }

    #[tokio::test]
    async fn a_reply_for_a_different_playlist_is_discarded() {
        let mut state = test_state();
        state.opened_playlist = Some(playlist(7, PlaylistKind::Created));
        let generation = state.generation;

        let tx = channel();
        state
            .handle_event(
                AppEvent::Songs {
                    generation,
                    playlist_id: PlaylistId(8),
                    result: Ok(vec![song(1)]),
                },
                &tx,
            )
            .await;
        assert!(
            state.songs.is_empty(),
            "another playlist's tracks were applied"
        );
    }

    #[tokio::test]
    async fn a_current_reply_is_applied() {
        let mut state = test_state();
        state.opened_playlist = Some(playlist(7, PlaylistKind::Created));
        state.pending = 1;
        let generation = state.generation;

        let tx = channel();
        state
            .handle_event(
                AppEvent::Songs {
                    generation,
                    playlist_id: PlaylistId(7),
                    result: Ok(vec![song(1), song(2)]),
                },
                &tx,
            )
            .await;
        assert_eq!(state.songs.len(), 2);
        assert_eq!(state.pending, 0);
    }

    #[tokio::test]
    async fn an_expired_session_sends_the_user_back_to_sign_in() {
        let mut state = test_state();
        state.user = Some(User {
            id: niveus_music_core::UserId(1),
            name: "n".into(),
        });
        state.pending = 1;

        let tx = channel();
        state
            .handle_event(
                AppEvent::Playlists(Err(ProviderError::Api(
                    niveus_ncm_provider::ApiFailure::Authentication("过期".into()),
                ))),
                &tx,
            )
            .await;

        assert!(state.user.is_none());
        assert!(state
            .notice
            .as_ref()
            .is_some_and(|n| n.text.contains("登录已过期")));
        // Saying "请重新扫码" is not enough on its own: a code has to be asked
        // for, or the sign-in screen promises one that never arrives and the
        // only way out is to restart the program.
        assert!(
            matches!(state.login, LoginPhase::Starting),
            "the user was left on a sign-in screen with no code coming"
        );
        assert!(state.pending > 0, "no code was actually requested");
    }

    /// A player connected to a socket that hangs up at once, so that every
    /// command fails the way it would with the player gone.
    async fn dead_player(name: &str) -> Player {
        let dir = std::env::temp_dir().join(format!("niveus-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("player.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();

        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                drop(stream);
            }
        });

        let (events, keep) = tokio::sync::mpsc::channel(4);
        std::mem::forget(keep);
        Player::connect(&socket, events).await.unwrap()
    }

    // ---- desktop integration ---------------------------------------------

    #[tokio::test]
    async fn what_the_desktop_asks_for_is_the_same_thing_a_key_does() {
        // "桌面点一下播放键，和用户按一下 Space，走的是同一条路": the two must
        // arrive at the same place, so the command is put through the same
        // handler the key is.
        let mut state = signed_in();
        state.queue.replace_items((1..=3).map(song).collect());
        state.queue.select(0);
        state.current_song = Some(song(1));
        state.playback.status = PlaybackStatus::Playing;

        let tx = channel();
        state
            .handle_event(AppEvent::Mpris(MprisCommand::PlayPause), &tx)
            .await;
        // Nothing to pause with no player, but it took the pause path and said
        // so rather than doing something else.
        assert!(state
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("播放器")));

        // Next moves the queue on, exactly as the `n` key does.
        state
            .handle_event(AppEvent::Mpris(MprisCommand::Next), &tx)
            .await;
        assert_eq!(state.queue.current_index, Some(1));
        assert_eq!(state.current_song.as_ref().unwrap().id, SongId(2));

        // Stop clears the track.
        state
            .handle_event(AppEvent::Mpris(MprisCommand::Stop), &tx)
            .await;
        assert!(state.current_song.is_none());
        assert_eq!(state.playback.status, PlaybackStatus::Stopped);

        // And quit is quit.
        state
            .handle_event(AppEvent::Mpris(MprisCommand::Quit), &tx)
            .await;
        assert!(state.quit);
    }

    #[tokio::test]
    async fn a_desktop_seek_is_counted_in_microseconds() {
        let mut state = signed_in();
        state.current_song = Some(song(1));

        let tx = channel();
        // No player, so nothing happens but the message; what is being pinned
        // down is that microseconds are not taken for milliseconds.
        state
            .handle_event(AppEvent::Mpris(MprisCommand::Seek(-5_000_000)), &tx)
            .await;
        // The value reaching the player is handled in the player's own tests;
        // here the conversion is what matters.
        assert_eq!(-5_000_000f64 / 1_000_000.0, -5.0);
    }

    #[tokio::test]
    async fn a_desktop_volume_is_a_percentage() {
        let mut state = signed_in();
        let tx = channel();
        state
            .handle_event(AppEvent::Mpris(MprisCommand::SetVolume(42)), &tx)
            .await;
        assert_eq!(state.playback.volume, 42);
    }

    #[tokio::test]
    async fn nothing_is_published_without_a_desktop_listening() {
        // No service, no errors and no work: the snapshot is only handed over
        // when there is somewhere to hand it.
        let mut state = signed_in();
        let tx = channel();
        state
            .handle_event(AppEvent::Mpris(MprisCommand::Next), &tx)
            .await;
        assert!(state.mpris.is_none());
    }

    #[tokio::test]
    async fn a_volume_the_player_refused_is_rolled_back() {
        // The readout must not go on showing a volume that is not coming out
        // of the speakers: nothing would ever correct it.
        let mut state = signed_in();
        state.player = Some(dead_player("volume").await);
        state.playback.volume = 50;

        state.change_volume(5).await;
        assert_eq!(state.playback.volume, 50, "a refused volume was kept");
        assert!(state
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("音量调整失败")));
    }

    // ---- queue view ------------------------------------------------------

    /// Open the queue view on a state holding `count` tracks.
    fn queued(count: u64) -> AppState {
        let mut state = test_state();
        state.queue.replace_items((1..=count).map(song).collect());
        state.queue_view = true;
        state
    }

    #[test]
    fn the_queue_view_replaces_the_list_and_gives_it_back() {
        let mut state = queued(3);
        assert!(matches!(state.list_view(), ListView::Queue { .. }));

        state.toggle_queue_view();
        assert!(matches!(state.list_view(), ListView::Playlists { .. }));
    }

    #[tokio::test]
    async fn leaving_the_page_closes_the_queue_view() {
        let mut state = test_state();
        state.queue_view = true;
        state.go_back();
        assert!(!state.queue_view, "escape did not close the queue");

        state.queue_view = true;
        let tx = channel();
        state.go_to_page(Page::Favorites, &tx);
        assert!(!state.queue_view, "changing page left the queue open");
    }

    #[test]
    fn the_playing_track_is_marked_only_when_it_is_in_the_queue() {
        let mut state = queued(3);
        state.queue.select(1);
        // A track selected out of the queue is one of its rows.
        let crate::state::Pane::Queue = state.pane() else {
            unreachable!()
        };

        let playing = state.queue.current().unwrap().clone();
        state.current_song = Some(playing);
        let ListView::Queue { playing, .. } = state.list_view() else {
            panic!("the queue view is not showing")
        };
        assert_eq!(playing, Some(1));

        // A track from "play next" is not one of them, so nothing is marked.
        state.current_song = Some(song(99));
        let ListView::Queue { playing, .. } = state.list_view() else {
            panic!()
        };
        assert_eq!(playing, None);
    }

    #[test]
    fn deleting_removes_the_row_and_keeps_the_cursor_in_range() {
        let mut state = queued(3);
        state.selected = 2;
        state.delete_from_queue();

        assert_eq!(state.queue.items.len(), 2);
        assert_eq!(state.selected, 1, "the cursor was left past the end");
    }

    // ---- settings --------------------------------------------------------

    /// A state on the settings page, with its files under its own directory.
    ///
    /// These tests write a configuration and a state file, and neither may be
    /// the ones the person running the tests actually uses.
    fn settings_page(name: &str) -> AppState {
        let dir = settings_dir(name);
        let mut state = AppState::new(
            Paths::new(dir.join("config"), dir.join("state"), dir.join("run")),
            Config::default(),
            Arc::new(NcmProvider::anonymous()),
            Vec::new(),
        );
        state.size = (120, 40);
        state.user = Some(User {
            id: niveus_music_core::UserId(1),
            name: "listener".into(),
        });
        state.go_to_page(Page::Settings, &channel());
        state
    }

    fn settings_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("niveus-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Moves the cursor onto a setting.
    fn point_at(state: &mut AppState, setting: Setting) {
        state.selected = Setting::index_of(setting);
    }

    #[tokio::test]
    async fn a_setting_is_picked_up_and_put_down_with_enter() {
        let mut state = settings_page("pick-up");
        let tx = channel();
        point_at(&mut state, Setting::CoverSize);

        // Picked up.
        state.handle_action(Action::Select, &tx).await;
        assert_eq!(state.editing_setting(), Some(Setting::CoverSize));

        // The arrows change it while it is held, rather than moving the cursor.
        state.handle_action(Action::MoveDown, &tx).await;
        assert_eq!(state.config.cover_size, CoverSize::Large);
        assert_eq!(state.selected, Setting::index_of(Setting::CoverSize));

        // Put down.
        state.handle_action(Action::Select, &tx).await;
        assert_eq!(state.editing_setting(), None);
        assert!(state
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("COVER SIZE")));

        // And now the arrows move the cursor again.
        state.handle_action(Action::MoveUp, &tx).await;
        assert_eq!(state.selected, Setting::index_of(Setting::Volume));

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn leaving_an_edit_keeps_what_was_changed_but_owns_up_to_it() {
        let mut state = settings_page("leave-edit");
        let tx = channel();
        point_at(&mut state, Setting::Volume);

        state.handle_action(Action::Select, &tx).await;
        state.handle_action(Action::MoveDown, &tx).await;
        assert_eq!(state.playback.volume, 77);

        // Esc leaves the edit without undoing it — a value changed is changed —
        // and says what it now is.
        state.handle_action(Action::Back, &tx).await;
        assert_eq!(state.editing_setting(), None);
        assert_eq!(state.playback.volume, 77);

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn every_setting_can_be_edited_from_the_page() {
        // Each row is reachable, changeable, and the change shows in the value
        // the page draws. A row that quietly did nothing would still pass a
        // test that only looked at the model.
        let mut state = settings_page("every-setting");
        let tx = channel();

        let steps = [
            (
                Setting::AudioQuality,
                AudioQuality::default().step(1).label().to_string(),
            ),
            (
                Setting::PlaybackMode,
                PlaybackMode::default().step(1).label().to_string(),
            ),
            (
                Setting::CoverSize,
                CoverSize::default().step(1).name().to_string(),
            ),
        ];

        for (setting, expected) in steps {
            point_at(&mut state, setting);
            state.handle_action(Action::Select, &tx).await;
            state.handle_action(Action::MoveDown, &tx).await;
            assert_eq!(
                state.setting_value(setting),
                expected,
                "{setting:?} did not move"
            );
            state.handle_action(Action::Back, &tx).await;
        }

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn a_preference_is_written_to_config_and_a_runtime_setting_to_state() {
        // §26 splits them, and the split is the point: a preferences file that
        // filled up with whatever the program was doing would be a state file
        // with a misleading name.
        let mut state = settings_page("two-files");
        let tx = channel();

        point_at(&mut state, Setting::AudioQuality);
        state.handle_action(Action::Select, &tx).await;
        state.handle_action(Action::MoveDown, &tx).await;
        state.handle_action(Action::Back, &tx).await;

        point_at(&mut state, Setting::PlaybackMode);
        state.handle_action(Action::Select, &tx).await;
        state.handle_action(Action::MoveDown, &tx).await;
        state.handle_action(Action::Back, &tx).await;

        let config = Config::load_or_default(&state.paths.config_file());
        assert_eq!(config.audio_quality, state.config.audio_quality);
        assert_ne!(
            config.audio_quality,
            AudioQuality::Exhigh,
            "nothing changed"
        );

        let (saved, error) = StateFile::load(&state.paths.state_file());
        assert!(error.is_none(), "{error:?}");
        assert_eq!(saved.queue.playback_mode, state.queue.playback_mode);
        assert_ne!(saved.queue.playback_mode, PlaybackMode::Sequential);
        // And the mode is not in the preferences file, which is not its home.
        assert!(!std::fs::read_to_string(state.paths.config_file())
            .unwrap()
            .contains("playback_mode"));

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn the_volume_written_to_state_is_the_volume_shown() {
        let mut state = settings_page("volume");
        let tx = channel();
        point_at(&mut state, Setting::Volume);

        state.handle_action(Action::Select, &tx).await;
        state.handle_action(Action::MoveUp, &tx).await;
        state.handle_action(Action::MoveUp, &tx).await;
        assert_eq!(state.playback.volume, 62);

        let (saved, _) = StateFile::load(&state.paths.state_file());
        assert_eq!(saved.volume, 62);

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn the_keybinding_list_scrolls_to_keep_the_cursor_on_screen() {
        // A short terminal cannot hold 23 actions, and the pane takes the
        // bottom off whatever is left. The window the app computes has to be
        // the one the renderer draws, or the cursor walks off the pane and the
        // user rebinds a row they cannot see (§5 第 16 条).
        let mut state = settings_page("kb-scroll");
        state.size = (80, 24);
        let tx = channel();

        state.selected = Setting::index_of(Setting::Keybindings);
        state.handle_action(Action::Select, &tx).await;
        for _ in 0..Action::ALL.len() - 1 {
            state.handle_action(Action::MoveDown, &tx).await;
        }

        assert_eq!(state.selected, Action::ALL.len() - 1);
        assert!(state.scroll > 0, "the cursor scrolled past the pane");
        let ListView::Keybindings {
            selected, scroll, ..
        } = state.list_view()
        else {
            panic!("the keybinding list is not showing")
        };
        assert_eq!(selected, state.selected);
        assert_eq!(
            scroll, state.scroll,
            "the window the app keeps the cursor in is not the one it draws"
        );

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn keybindings_open_as_a_list_and_can_be_rebound() {
        let mut state = settings_page("rebind");
        let tx = channel();

        state.selected = Setting::index_of(Setting::Keybindings);
        state.handle_action(Action::Select, &tx).await;
        assert_eq!(state.settings_mode, SettingsMode::Keybindings);
        assert!(matches!(state.list_view(), ListView::Keybindings { .. }));

        // The cursor is the binding list's own, so it starts where that list
        // was left rather than where the settings list was.
        let action = Action::Quit;
        state.selected = Action::ALL.iter().position(|a| *a == action).unwrap();
        state.handle_action(Action::Select, &tx).await;
        assert_eq!(state.settings_mode, SettingsMode::Capturing);

        // The next key is the answer, not a command: `x` is bound, and the
        // program does not also treat it as anything else.
        state
            .handle_event(
                AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
                &tx,
            )
            .await;
        assert_eq!(
            state.config.keymap.key_for(Action::Quit),
            Some(Key::Char('x'))
        );
        assert_eq!(
            state.settings_mode,
            SettingsMode::Keybindings,
            "still capturing"
        );

        // Written down only now, and readable back.
        let config = Config::load_or_default(&state.paths.config_file());
        assert_eq!(config.keymap.key_for(Action::Quit), Some(Key::Char('x')));

        // §36 rule 8: what the interface shows has to be what the keymap says,
        // or the hint teaches a key that no longer does anything.
        assert_eq!(state.ui_state().hint(Action::Quit), "[X]");

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn a_key_another_action_holds_is_refused_and_nothing_is_written() {
        // §25: a conflict is reported and the change does not happen. Stealing
        // the key silently is how a user ends up with a binding they never made,
        // and writing the file anyway would make it survive a restart.
        let mut state = settings_page("conflict");
        let tx = channel();

        state.selected = Setting::index_of(Setting::Keybindings);
        state.handle_action(Action::Select, &tx).await;
        state.selected = Action::ALL.iter().position(|a| *a == Action::Quit).unwrap();
        state.handle_action(Action::Select, &tx).await;

        // `1` belongs to the playlist page.
        state
            .handle_event(
                AppEvent::Key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
                &tx,
            )
            .await;

        assert_eq!(
            state.config.keymap.key_for(Action::Quit),
            Some(Key::Char('q'))
        );
        assert_eq!(
            state.config.keymap.key_for(Action::ShowPlaylist),
            Some(Key::Char('1')),
            "the key was taken from the action that held it"
        );
        assert!(state
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("无法绑定")));
        assert!(
            !state.paths.config_file().exists(),
            "a refused change was written"
        );

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn the_fixed_way_out_still_works_in_the_middle_of_a_rebind() {
        // Ctrl+C is how a user always leaves, whatever is on screen (§24). The
        // key prompt is exactly the place where being trapped would be worst:
        // every other key is being swallowed as a potential binding.
        let mut state = settings_page("ctrl-c");
        let tx = channel();
        state.selected = Setting::index_of(Setting::Keybindings);
        state.handle_action(Action::Select, &tx).await;
        state.selected = 0;
        state.handle_action(Action::Select, &tx).await;
        assert_eq!(state.settings_mode, SettingsMode::Capturing);

        state
            .handle_event(
                AppEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                &tx,
            )
            .await;

        assert!(state.quit, "the prompt swallowed the way out");
        // And it was not taken as the binding, either: it is not in the keymap
        // and no configuration can put it there (§24).
        assert!(state
            .config
            .keymap
            .iter()
            .all(|(_, key)| key != Key::Ctrl('c')));
        assert!(!state.paths.config_file().exists(), "a binding was written");

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn escape_abandons_a_rebind() {
        let mut state = settings_page("escape");
        let tx = channel();
        state.selected = Setting::index_of(Setting::Keybindings);
        state.handle_action(Action::Select, &tx).await;
        state.selected = 0;
        state.handle_action(Action::Select, &tx).await;

        state
            .handle_event(
                AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                &tx,
            )
            .await;

        assert_eq!(state.settings_mode, SettingsMode::Keybindings);
        assert_eq!(
            state.config.keymap,
            Keymap::default(),
            "cancelling changed the map"
        );

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn leaving_the_settings_page_puts_an_edit_down() {
        // A half-finished edit must not follow the user to another page and
        // start eating the arrow keys there.
        let mut state = settings_page("leave-page");
        let tx = channel();
        point_at(&mut state, Setting::Volume);
        state.handle_action(Action::Select, &tx).await;
        assert!(state.editing_setting().is_some());

        state.handle_action(Action::ShowFavorites, &tx).await;
        assert_eq!(state.editing_setting(), None);

        // And the arrows move the cursor there, as they always did.
        state.handle_action(Action::MoveDown, &tx).await;
        assert_eq!(state.editing_setting(), None);

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[test]
    fn deleting_works_only_in_the_queue_view() {
        let mut state = queued(3);
        state.queue_view = false;
        state.delete_from_queue();
        assert_eq!(state.queue.items.len(), 3, "a browse list was edited");
    }

    #[test]
    fn reordering_takes_the_cursor_with_the_track() {
        let mut state = queued(3);
        state.selected = 0;
        state.reorder_selected(1);

        let ids: Vec<u64> = state.queue.items.iter().map(|song| song.id.get()).collect();
        assert_eq!(ids, vec![2, 1, 3]);
        assert_eq!(state.selected, 1, "the cursor stayed behind");

        state.reorder_selected(-1);
        let ids: Vec<u64> = state.queue.items.iter().map(|song| song.id.get()).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn reordering_stops_at_the_ends() {
        let mut state = queued(3);
        state.reorder_selected(-1);
        assert_eq!(state.selected, 0);
        let ids: Vec<u64> = state.queue.items.iter().map(|song| song.id.get()).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    // ---- playback --------------------------------------------------------

    #[tokio::test]
    async fn choosing_a_track_makes_its_list_the_queue() {
        let mut state = signed_in();
        state.songs = (11..=13).map(song).collect();
        state.page = Page::Playlist;
        state.opened_playlist = Some(playlist(7, PlaylistKind::Created));
        state.selected = 1;

        let tx = channel();
        state.handle_action(Action::Select, &tx).await;

        let ids: Vec<u64> = state.queue.items.iter().map(|song| song.id.get()).collect();
        assert_eq!(ids, vec![11, 12, 13]);
        assert_eq!(state.current_song.as_ref().unwrap().id, SongId(12));
    }

    #[tokio::test]
    async fn playing_another_track_of_the_same_list_keeps_what_was_queued_next() {
        let mut state = signed_in();
        state.songs = (11..=13).map(song).collect();
        state.opened_playlist = Some(playlist(7, PlaylistKind::Created));

        let tx = channel();
        state.selected = 0;
        state.handle_action(Action::Select, &tx).await;
        state.handle_action(Action::PlayNext, &tx).await;
        assert_eq!(state.queue.pending_next.len(), 1);

        // A second pick out of the same list must not rebuild the queue.
        state.selected = 2;
        state.handle_action(Action::Select, &tx).await;
        assert_eq!(
            state.queue.pending_next.len(),
            1,
            "choosing another track threw away the queue"
        );
        assert_eq!(state.current_song.as_ref().unwrap().id, SongId(13));
    }

    /// Marks the track showing as one the player has actually been given.
    ///
    /// The player's reports only count for a track it was handed, so a test
    /// that wants to deliver one has to say so — the same thing
    /// `begin_playback` does when the source has resolved.
    fn handed_over(state: &mut AppState) {
        state.loaded_generation = Some(state.play_generation);
    }

    #[tokio::test]
    async fn a_natural_end_moves_the_queue_on() {
        let mut state = signed_in();
        state.queue.replace_items((1..=3).map(song).collect());
        state.queue.select(0);
        state.current_song = Some(song(1));
        state.playback.status = PlaybackStatus::Playing;
        handed_over(&mut state);

        let tx = channel();
        state
            .handle_event(AppEvent::Player(PlayerEvent::Ended), &tx)
            .await;

        assert_eq!(state.queue.current_index, Some(1));
        assert_eq!(state.current_song.as_ref().unwrap().id, SongId(2));
    }

    #[tokio::test]
    async fn an_exit_report_is_never_dropped_as_stale() {
        // The news that the player itself is gone describes no track, so the
        // check that discards reports about a replaced track must not swallow
        // it. Swallowed, the program would go on giving commands to a process
        // that is not there, and every one of them would fail.
        let mut state = signed_in();
        state.current_song = Some(song(1));
        state.playback.status = PlaybackStatus::Playing;
        // A track chosen but not yet handed over: exactly the window every
        // other report is dropped in.
        state.play_generation = 5;
        state.loaded_generation = Some(4);

        let tx = channel();
        state
            .handle_event(AppEvent::Player(PlayerEvent::Exited), &tx)
            .await;

        assert_eq!(state.playback.status, PlaybackStatus::Stopped);
        assert!(state.player.is_none(), "the dead player was kept");
        assert_eq!(state.loaded_generation, None);
    }

    #[tokio::test]
    async fn a_player_that_died_leaves_the_play_key_something_to_resume() {
        // The track is still known but the player no longer holds it, so the
        // play key has to start it afresh. Without this it would toggle the
        // pause of an empty player and the sound would never come back.
        let mut state = signed_in();
        state.current_song = Some(song(1));
        state.playback.status = PlaybackStatus::Playing;
        state.resume_on_play = false;
        handed_over(&mut state);

        let tx = channel();
        state
            .handle_event(AppEvent::Player(PlayerEvent::Exited), &tx)
            .await;

        assert!(state.resume_on_play, "there is nothing to resume");
        assert_eq!(state.playback.status, PlaybackStatus::Stopped);
        assert_eq!(state.playback.position_ms, 0);
    }

    #[tokio::test]
    async fn an_exit_report_does_not_resurrect_a_player_given_up_on() {
        // With no player in hand the report can only be one that outlived its
        // player. Starting another here would undo the decision to stop
        // trying, and a failing player would be restarted forever.
        let mut state = signed_in();
        state.player = None;
        state.player_restarts = MAX_PLAYER_RESTARTS;

        let tx = channel();
        state
            .handle_event(AppEvent::Player(PlayerEvent::Exited), &tx)
            .await;

        assert!(state.player.is_none());
        assert_eq!(state.player_restarts, MAX_PLAYER_RESTARTS);
    }

    #[tokio::test]
    async fn an_end_after_a_manual_choice_does_not_move_the_queue_on() {
        // The player reports an end only for a track that ran out; this pins
        // down that an end arriving with nothing selected is ignored rather
        // than advancing a queue the user never started.
        let mut state = signed_in();
        state.queue.replace_items((1..=3).map(song).collect());
        state.queue.select(0);
        let before = state.queue.current_index;

        let tx = channel();
        state
            .handle_event(AppEvent::Player(PlayerEvent::Ended), &tx)
            .await;
        assert_eq!(state.queue.current_index, before);
    }

    #[tokio::test]
    async fn an_end_already_on_its_way_does_not_skip_the_chosen_track() {
        // The end of file for track A can be queued before the user's keypress
        // is handled. Acting on it afterwards would advance twice for one
        // keypress and the chosen track would never be heard.
        let mut state = signed_in();
        state.queue.replace_items((1..=3).map(song).collect());
        state.queue.select(0);
        state.current_song = Some(song(1));
        handed_over(&mut state);

        let tx = channel();
        // The user moves on while A's end of file is still in the queue.
        state.skip_forward(&tx).await;
        assert_eq!(state.queue.current_index, Some(1));

        state
            .handle_event(AppEvent::Player(PlayerEvent::Ended), &tx)
            .await;
        assert_eq!(
            state.queue.current_index,
            Some(1),
            "one keypress skipped two tracks"
        );
    }

    #[tokio::test]
    async fn the_end_of_a_queue_that_does_not_loop_stops_playback() {
        let mut state = signed_in();
        state.queue.replace_items(vec![song(1)]);
        state.queue.select(0);
        state.current_song = Some(song(1));
        handed_over(&mut state);

        let tx = channel();
        state
            .handle_event(AppEvent::Player(PlayerEvent::Ended), &tx)
            .await;

        assert_eq!(state.playback.status, PlaybackStatus::Stopped);
        assert!(state.current_song.is_none());
        assert!(state
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("播放完毕")));
    }

    #[tokio::test]
    async fn progress_becomes_the_playback_position() {
        let mut state = signed_in();
        state.current_song = Some(song(1));
        handed_over(&mut state);

        let tx = channel();
        state
            .handle_event(AppEvent::Player(PlayerEvent::Position(12.5)), &tx)
            .await;
        state
            .handle_event(AppEvent::Player(PlayerEvent::Duration(269.0)), &tx)
            .await;

        assert_eq!(state.playback.position_ms, 12_500);
        assert_eq!(state.playback.duration_ms, Some(269_000));
    }

    #[tokio::test]
    async fn the_pause_state_is_only_playback_once_something_is_chosen() {
        // The player reports its pause state as soon as the session connects,
        // which is before any track exists.
        let mut state = signed_in();

        let tx = channel();
        state
            .handle_event(AppEvent::Player(PlayerEvent::Paused(false)), &tx)
            .await;
        assert_eq!(
            state.playback.status,
            PlaybackStatus::Stopped,
            "an idle player was taken for playing"
        );

        state.current_song = Some(song(1));
        state
            .handle_event(AppEvent::Player(PlayerEvent::Paused(true)), &tx)
            .await;
        assert_eq!(
            state.playback.status,
            PlaybackStatus::Stopped,
            "a report counted for a track the player was never given"
        );

        // Once it has been handed the track, its reports do count.
        handed_over(&mut state);
        state
            .handle_event(AppEvent::Player(PlayerEvent::Paused(true)), &tx)
            .await;
        assert_eq!(state.playback.status, PlaybackStatus::Paused);
    }

    #[tokio::test]
    async fn a_track_restored_from_last_time_is_not_reported_as_playing() {
        // The pane shows the track that was playing when the program last
        // stopped, and the player — which has just connected and been given
        // nothing — reports its own idle pause state as soon as it is watched.
        // Taking that as news would have the status bar and the desktop both
        // claim sound that never came out, until the user pressed play.
        let mut state = signed_in();
        let mut queue = Queue::new(PlaybackMode::Sequential);
        queue.replace_items((1..=3).map(song).collect());
        queue.select(1);
        state.restore(StateFile {
            queue,
            ..StateFile::default()
        });
        assert_eq!(state.current_song.as_ref().map(|s| s.id), Some(SongId(2)));
        assert!(state.resume_on_play, "there is nothing to resume");

        let tx = channel();
        for event in [
            PlayerEvent::Paused(false),
            PlayerEvent::Loaded,
            PlayerEvent::Position(0.0),
        ] {
            let described = format!("{event:?}");
            state.handle_event(AppEvent::Player(event), &tx).await;
            assert_eq!(
                state.playback.status,
                PlaybackStatus::Stopped,
                "{described} claimed playback of a track nobody played"
            );
        }
    }

    #[tokio::test]
    async fn a_source_for_a_track_the_user_has_left_is_discarded() {
        let mut state = signed_in();
        state.current_song = Some(song(2));
        state.play_generation = 5;
        state.pending = 1;

        let tx = channel();
        state
            .handle_event(
                AppEvent::SongUrl {
                    generation: 4,
                    song: song(1),
                    result: Ok(AudioSource {
                        url: "https://example.invalid/stale.mp3".into(),
                        level: None,
                        format: None,
                        bitrate: None,
                        is_trial: false,
                    }),
                },
                &tx,
            )
            .await;

        assert!(state.source.is_none(), "a stale source was played");
        // Dropped, but still counted in: leaving the count high would pin the
        // status bar on LOADING… for the rest of the session.
        assert_eq!(state.pending, 0);
    }

    #[tokio::test]
    async fn a_source_that_could_not_be_resolved_leaves_nothing_playing() {
        let mut state = signed_in();
        let generation = state.play_generation;
        state.current_song = Some(song(1));
        state.pending = 1;

        let tx = channel();
        state
            .handle_event(
                AppEvent::SongUrl {
                    generation,
                    song: song(1),
                    result: Err(ProviderError::MissingResource("不可播放".into())),
                },
                &tx,
            )
            .await;

        assert!(state.current_song.is_none());
        assert_eq!(state.playback.status, PlaybackStatus::Stopped);
        assert_eq!(state.pending, 0);
    }

    #[tokio::test]
    async fn play_next_puts_the_highlighted_track_ahead_of_the_rest() {
        let mut state = signed_in();
        state.queue.replace_items((1..=3).map(song).collect());
        state.queue.select(0);
        state.queue_view = true;
        state.selected = 2;

        let tx = channel();
        state.handle_action(Action::PlayNext, &tx).await;

        assert_eq!(state.queue.pending_next.len(), 1);
        assert_eq!(state.queue.pending_next[0].id, SongId(3));
        assert!(state
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("已加入下一首")));
    }

    #[tokio::test]
    async fn without_a_player_playback_says_so_rather_than_failing_silently() {
        let mut state = signed_in();
        state.current_song = Some(song(1));

        let tx = channel();
        state.handle_action(Action::PlayPause, &tx).await;

        assert!(state
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("播放器")));
    }

    // ---- lyrics -----------------------------------------------------------

    /// Three lines, ten seconds apart, with the first at 0:01.
    const LRC: &str = "[00:01.00]第一句\n[00:11.00]第二句\n[00:21.00]第三句";

    async fn deliver_lyrics(
        state: &mut AppState,
        generation: u64,
        result: Result<RawLyrics, ProviderError>,
    ) {
        let tx = channel();
        state
            .handle_event(
                AppEvent::Lyrics {
                    generation,
                    song_id: SongId(1),
                    result,
                },
                &tx,
            )
            .await;
    }

    /// The lines the panel would draw, oldest first, empty slots and all.
    fn window_of(state: &AppState) -> Vec<Option<String>> {
        match state.ui_state().now_playing.lyrics {
            LyricsView::Window(window) => window
                .lines()
                .iter()
                .map(|line| line.map(str::to_string))
                .collect(),
            other => panic!("expected a lyric window, found {other:?}"),
        }
    }

    /// The line the window puts in its middle, which is the one being sung.
    fn current_of(state: &AppState) -> Option<String> {
        let window = window_of(state);
        window.get(window.len() / 2).cloned().flatten()
    }

    /// A state showing a track, with its lyrics still on their way.
    fn awaiting_lyrics() -> AppState {
        let mut state = showing_a_track();
        state.play_generation = 1;
        state.pending = 1;
        state
    }

    #[tokio::test]
    async fn the_line_being_sung_takes_the_middle_of_the_three() {
        let mut state = awaiting_lyrics();
        state.playback.position_ms = 12_000;
        deliver_lyrics(&mut state, 1, Ok(RawLyrics::original(LRC))).await;

        assert_eq!(current_of(&state), Some("第二句".into()));
        assert_eq!(
            state.pending, 0,
            "the reply did not come off the loading count"
        );
    }

    #[tokio::test]
    async fn the_window_follows_the_position_without_the_rows_moving() {
        let mut state = awaiting_lyrics();
        deliver_lyrics(&mut state, 1, Ok(RawLyrics::original(LRC))).await;

        // Before the first line there is nothing being sung, and the first line
        // is already in the slot it will move up through.
        state.playback.position_ms = 0;
        assert_eq!(current_of(&state), None, "the first line should be waiting");
        let window = window_of(&state);
        let middle = window.len() / 2;
        assert_eq!(window[middle + 1], Some("第一句".into()));

        // 0:01 and 0:21: the line being sung is the middle slot either way.
        state.playback.position_ms = 1_000;
        assert_eq!(current_of(&state), Some("第一句".into()));
        state.playback.position_ms = 30_000;
        assert_eq!(current_of(&state), Some("第三句".into()));

        // And the rows do not move: the window is the same size throughout.
        assert_eq!(window_of(&state).len(), window.len());
    }

    #[tokio::test]
    async fn the_window_is_the_largest_one_the_panel_has_room_for() {
        let mut state = awaiting_lyrics();
        deliver_lyrics(&mut state, 1, Ok(RawLyrics::original(LRC))).await;

        // 120×40: a 36-row pane, a 12-row cover and a 15-row panel, so the
        // window is the largest 4k+1 that fits — 13 rows, which is seven lines.
        assert_eq!(state.lyric_rows(), 15);
        assert_eq!(window_of(&state).len(), 7);

        // The metadata block costs six of those rows when it is showing: five
        // for itself and one for the rule above it.
        state.metadata = true;
        assert_eq!(
            state.lyric_rows(),
            9,
            "the block and its rule were not taken off"
        );
        assert_eq!(window_of(&state).len(), 5, "nine rows hold five lines");

        // A shorter terminal leaves less of a panel, and a smaller window.
        state.metadata = false;
        state.size = (120, 32);
        assert_eq!(window_of(&state).len(), 5, "ten rows hold five lines");
    }

    #[tokio::test]
    async fn lyrics_for_a_track_the_user_has_left_are_dropped() {
        let mut state = awaiting_lyrics();
        state.play_generation = 2;

        deliver_lyrics(&mut state, 1, Ok(RawLyrics::original(LRC))).await;

        assert_eq!(
            state.ui_state().now_playing.lyrics,
            LyricsView::Pending,
            "a stale reply was shown"
        );
        // Dropped, but still counted in: the status bar would otherwise say
        // LOADING… for the rest of the session.
        assert_eq!(state.pending, 0);
    }

    #[tokio::test]
    async fn a_translation_with_no_original_says_so_rather_than_showing_nothing() {
        let mut state = awaiting_lyrics();
        deliver_lyrics(&mut state, 1, Ok(RawLyrics::TranslationOnly)).await;
        assert_eq!(
            state.ui_state().now_playing.lyrics,
            LyricsView::TranslationOnly
        );
    }

    #[tokio::test]
    async fn a_track_with_no_lyrics_is_instrumental_and_not_a_failure() {
        let mut state = awaiting_lyrics();
        deliver_lyrics(&mut state, 1, Ok(RawLyrics::Unavailable)).await;
        assert_eq!(
            state.ui_state().now_playing.lyrics,
            LyricsView::Instrumental
        );
    }

    #[tokio::test]
    async fn a_body_that_is_not_a_timeline_is_reported_as_a_lyric_failure() {
        let mut state = awaiting_lyrics();
        deliver_lyrics(
            &mut state,
            1,
            Ok(RawLyrics::original("没有时间戳的一段歌词")),
        )
        .await;

        assert_eq!(state.ui_state().now_playing.lyrics, LyricsView::Failed);
    }

    #[tokio::test]
    async fn a_lyric_fetch_that_fails_is_a_panel_state_and_not_a_status_notice() {
        let mut state = awaiting_lyrics();
        deliver_lyrics(
            &mut state,
            1,
            Err(ProviderError::MissingResource("歌词接口异常".into())),
        )
        .await;

        assert_eq!(state.ui_state().now_playing.lyrics, LyricsView::Failed);
        // The reason belongs in the log: a track with no lyrics is ordinary, and
        // announcing it on the status bar on every track change would be noise.
        assert!(
            state.ui_state().status.notice.is_none(),
            "a failed lyric fetch was announced as a notice"
        );
    }

    #[tokio::test]
    async fn a_new_track_clears_the_lyrics_of_the_one_before_it() {
        let mut state = awaiting_lyrics();
        deliver_lyrics(&mut state, 1, Ok(RawLyrics::original(LRC))).await;
        assert_ne!(state.ui_state().now_playing.lyrics, LyricsView::Pending);

        // The whole point: the old song's words must not stay up under the new
        // song's name while the new ones are being fetched.
        let tx = channel();
        state.start_track(song(2), &tx).await;
        assert_eq!(state.ui_state().now_playing.lyrics, LyricsView::Pending);
    }

    #[tokio::test]
    async fn stopping_playback_clears_the_lyrics_too() {
        let mut state = awaiting_lyrics();
        deliver_lyrics(&mut state, 1, Ok(RawLyrics::original(LRC))).await;

        state.stop_playback().await;
        assert_eq!(state.ui_state().now_playing.lyrics, LyricsView::Pending);
    }

    // ---- metadata ---------------------------------------------------------

    fn lossless_source() -> AudioSource {
        AudioSource {
            url: "https://example.invalid/song.flac".into(),
            level: Some(AudioQuality::Lossless),
            format: Some("flac".into()),
            bitrate: Some(999_000),
            is_trial: false,
        }
    }

    #[test]
    fn the_resource_playing_is_projected_for_the_metadata_panel() {
        let mut state = showing_a_track();
        state.source = Some(lossless_source());

        let ui = state.ui_state();
        let source = ui.now_playing.source.expect("the source is projected");
        // The served tier, not the requested one — this is what the panel shows
        // and what the status bar counts in kilobits.
        assert_eq!(source.level, Some(AudioQuality::Lossless));
        assert_eq!(source.format.as_deref(), Some("flac"));
        assert_eq!(ui.status.bitrate, Some(999_000));
    }

    #[test]
    fn nothing_playing_means_no_bitrate_and_no_source() {
        let state = showing_a_track();
        let ui = state.ui_state();
        assert!(ui.now_playing.source.is_none());
        assert_eq!(ui.status.bitrate, None);
    }

    // ---- album art --------------------------------------------------------

    /// A state with a session, a roomy terminal and a track showing.
    fn showing_a_track() -> AppState {
        let mut state = signed_in();
        state.size = (120, 40);
        state.current_song = Some(song(1));
        state
    }

    fn cover(url: &str) -> CoverKey {
        let area = app_artwork_area();
        CoverKey::new(
            url,
            area,
            cover::options(Config::default().artwork_mode, ColourDepth::TrueColor),
        )
    }

    fn app_artwork_area() -> niveus_artwork::Size {
        cover::area(
            niveus_ui::compute(
                Rect::new(0, 0, 120, 40),
                LayoutTier::Standard,
                CoverSize::Normal,
            )
            .artwork
            .expect("standard tier has room for art"),
        )
    }

    #[test]
    fn a_track_with_no_cover_gets_the_default_sign() {
        // No cover is not a failure: the design asks for the sign by name, and
        // there is nothing to wait for.
        let mut state = showing_a_track();
        let tx = channel();
        state.refresh_artwork(&tx);

        assert!(state.artwork.is_some(), "nothing was drawn");
        assert!(state.artwork_key.is_none());
        assert_eq!(state.pending, 0, "the default sign was waited for");
    }

    #[tokio::test]
    async fn a_cover_is_asked_for_once_and_the_pane_waits_blank() {
        let mut state = showing_a_track();
        state.current_song = Some(song_with_cover(1, "https://example.invalid/a.png"));
        let tx = channel();
        state.refresh_artwork(&tx);

        assert_eq!(
            state.artwork_key.as_ref().unwrap().url,
            "https://example.invalid/a.png"
        );
        assert!(
            state.artwork.is_none(),
            "a cover was shown before it arrived"
        );
        assert_eq!(state.pending, 1);

        // Asking again for the same track and the same size changes nothing.
        state.refresh_artwork(&tx);
        assert_eq!(state.pending, 1, "the same cover was asked for twice");
    }

    #[tokio::test]
    async fn a_cover_for_another_track_is_dropped() {
        let mut state = showing_a_track();
        state.current_song = Some(song_with_cover(2, "https://example.invalid/b.png"));
        let tx = channel();
        state.refresh_artwork(&tx);
        let wanted = state.artwork_key.clone().unwrap();
        state.pending = 1;

        // The cover for the track the user has just left arrives late.
        state
            .handle_event(
                AppEvent::Cover {
                    key: cover("https://example.invalid/a.png"),
                    result: Ok(Frame::new(2, 2)),
                },
                &tx,
            )
            .await;

        assert!(state.artwork.is_none(), "a stale cover was drawn");
        assert_eq!(state.artwork_key.as_ref(), Some(&wanted));
    }

    #[tokio::test]
    async fn an_arriving_cover_is_kept_and_drawn() {
        let mut state = showing_a_track();
        state.current_song = Some(song_with_cover(1, "https://example.invalid/a.png"));
        let tx = channel();
        state.refresh_artwork(&tx);
        let key = state.artwork_key.clone().unwrap();

        let mut frame = Frame::new(4, 4);
        frame.set(
            0,
            0,
            niveus_artwork::Cell::solid('#', niveus_artwork::Ink::Accent),
        );
        state
            .handle_event(
                AppEvent::Cover {
                    key: key.clone(),
                    result: Ok(frame),
                },
                &tx,
            )
            .await;

        let artwork = state.artwork.as_ref().expect("the cover was not drawn");
        assert_eq!((artwork.width(), artwork.height()), (4, 4));
        assert_eq!(artwork.cell(0, 0).unwrap().symbol, '#');
        assert_eq!(state.pending, 0);

        // It is kept, so coming back to this track costs nothing.
        state.artwork = None;
        state.artwork_key = None;
        state.refresh_artwork(&tx);
        assert!(state.artwork.is_some(), "the cover was fetched again");
        assert_eq!(state.pending, 0);
    }

    #[tokio::test]
    async fn a_cover_that_cannot_be_had_becomes_the_default_sign() {
        let mut state = showing_a_track();
        state.current_song = Some(song_with_cover(1, "https://example.invalid/a.png"));
        let tx = channel();
        state.refresh_artwork(&tx);
        let key = state.artwork_key.clone().unwrap();
        state.pending = 1;

        state
            .handle_event(
                AppEvent::Cover {
                    key,
                    result: Err(ArtworkError::Status(404)),
                },
                &tx,
            )
            .await;

        assert!(state.artwork.is_some(), "the pane was left blank");
        assert!(!state.artwork.as_ref().unwrap().is_empty());
    }

    #[test]
    fn a_narrow_terminal_has_nowhere_to_draw_a_cover() {
        let mut state = signed_in();
        // Minimal: the now-playing pane is not shown at all.
        state.size = (100, 28);
        state.current_song = Some(song_with_cover(1, "https://example.invalid/a.png"));

        let tx = channel();
        state.refresh_artwork(&tx);
        assert!(state.artwork.is_none());
        assert!(state.artwork_key.is_none());
        assert_eq!(
            state.pending, 0,
            "a cover was fetched for a pane that is not shown"
        );
    }

    #[test]
    fn a_resize_is_waited_out_before_the_cover_is_redone() {
        let mut state = showing_a_track();
        let tx = channel();
        state.refresh_artwork(&tx);
        assert!(state.artwork.is_some());

        // A drag of the window's edge produces a stream of sizes; only the last
        // of them is worth acting on.
        state.note_resize();
        state.resize_settled(&tx);
        assert!(
            state.resize_at.is_some(),
            "a resize was acted on immediately"
        );

        state.resize_at = Some(Instant::now() - Duration::from_millis(500));
        state.resize_settled(&tx);
        assert!(state.resize_at.is_none(), "the resize was never acted on");
    }

    // ---- persistence -----------------------------------------------------

    /// A player that accepts every command, so a track can reach the moment it
    /// counts as heard without a real mpv.
    ///
    /// Only the success path is scripted: the interesting question here is when
    /// a track is recorded, not what a real player does with the URL.
    async fn willing_player(name: &str) -> Player {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let dir = std::env::temp_dir().join(format!("niveus-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("player.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();

        tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (read, mut write) = tokio::io::split(stream);
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                let Some(id) = message.get("request_id").and_then(|id| id.as_u64()) else {
                    continue;
                };
                let reply = format!("{{\"request_id\":{id},\"error\":\"success\"}}\n");
                if write.write_all(reply.as_bytes()).await.is_err() {
                    return;
                }
                let _ = write.flush().await;
            }
        });

        let (events, keep) = tokio::sync::mpsc::channel(8);
        std::mem::forget(keep);
        Player::connect(&socket, events).await.unwrap()
    }

    fn a_source() -> AudioSource {
        AudioSource {
            url: "http://example.invalid/song.mp3".into(),
            level: Some(AudioQuality::Exhigh),
            format: Some("mp3".into()),
            bitrate: Some(320_000),
            is_trial: false,
        }
    }

    #[tokio::test]
    async fn a_track_is_recorded_as_heard_when_it_starts_playing() {
        // SPEC §30 puts the moment at "the audio URL resolved and the player
        // took it". Recording on selection instead would list tracks that never
        // sounded; recording on the next track would lose the last one.
        let mut state = signed_in();
        state.player = Some(willing_player("history").await);
        state.current_song = Some(song(7));

        assert!(
            state.history.is_empty(),
            "something was recorded before playing"
        );
        state.begin_playback(a_source()).await;

        assert_eq!(state.history.len(), 1);
        assert_eq!(state.history.entries()[0].song.id, SongId(7));
        assert!(
            state.history.entries()[0].played_at > 0,
            "no time was recorded"
        );
        assert_eq!(state.playback.status, PlaybackStatus::Playing);
    }

    #[tokio::test]
    async fn a_track_that_could_not_be_played_is_not_recorded_as_heard() {
        let mut state = signed_in();
        let generation = state.play_generation;
        state.current_song = Some(song(1));
        state.pending = 1;

        let tx = channel();
        state
            .handle_event(
                AppEvent::SongUrl {
                    generation,
                    song: song(1),
                    result: Err(ProviderError::MissingResource("不可播放".into())),
                },
                &tx,
            )
            .await;

        assert!(
            state.history.is_empty(),
            "silence was written down as heard"
        );
    }

    #[tokio::test]
    async fn the_history_page_lists_what_was_heard_newest_first() {
        let mut state = signed_in();
        state.history.record(song(1), 1_700_000_000);
        state.history.record(song(2), 1_700_000_100);

        state.go_to_page(Page::History, &channel());
        let ListView::History { entries, .. } = state.list_view() else {
            panic!("the history page is not showing its record")
        };
        let ids: Vec<u64> = entries.iter().map(|entry| entry.song.id.get()).collect();
        assert_eq!(ids, vec![2, 1], "the newest is not first");

        // And its rows are playable: the pane hands its tracks over like any
        // other list.
        assert_eq!(
            state
                .listed_songs()
                .iter()
                .map(|s| s.id.get())
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
        assert_eq!(state.selected_song().map(|s| s.id), Some(SongId(2)));
    }

    #[tokio::test]
    async fn an_empty_history_page_says_so_rather_than_showing_another_list() {
        let mut state = signed_in();
        state.go_to_page(Page::History, &channel());
        assert!(
            matches!(state.list_view(), ListView::History { entries, .. } if entries.is_empty())
        );
        assert!(state.listed_songs().is_empty());
        assert_eq!(state.selected_song(), None);
    }

    /// A state whose files live under its own directory.
    fn persistable(name: &str) -> AppState {
        let dir =
            std::env::temp_dir().join(format!("niveus-persist-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("state")).unwrap();

        let mut state = AppState::new(
            Paths::new(dir.join("config"), dir.join("state"), dir.join("run")),
            Config::default(),
            Arc::new(NcmProvider::anonymous()),
            Vec::new(),
        );
        state.user = Some(User {
            id: niveus_music_core::UserId(1),
            name: "listener".into(),
        });
        state.size = (120, 40);
        state
    }

    #[tokio::test]
    async fn a_restart_finds_the_queue_the_history_and_the_cursors_where_they_were() {
        // The point of the file: what the user was in the middle of is still
        // there next time, rather than a fresh program that happens to remember
        // their volume.
        let mut state = persistable("round-trip");
        state.queue.replace_items((1..=5).map(song).collect());
        state.queue.select(2);
        state.queue.playback_mode = PlaybackMode::ListLoop;
        state.queue.push_next(song(9));
        state.history.record(song(4), 1_700_000_000);
        state.playback.volume = 41;
        state.playback.position_ms = 30_000;
        state.pending_playlist = Some(PlaylistId(77));
        state.cursors.insert(
            ListId::Playlist(PlaylistId(77)),
            Cursor {
                selected: 3,
                scroll: 1,
            },
        );
        state.selected = 4;
        state.scroll = 2;
        state.persist_state();

        let mut restarted = persistable("round-trip-again");
        restarted.paths = state.paths.clone();
        let (saved, error) = StateFile::load(&restarted.paths.state_file());
        assert!(error.is_none(), "{error:?}");
        restarted.restore(saved);

        assert_eq!(restarted.queue.items.len(), 5);
        assert_eq!(restarted.queue.current_index, Some(2));
        assert_eq!(restarted.queue.playback_mode, PlaybackMode::ListLoop);
        assert_eq!(restarted.queue.pending_next.len(), 1, "play-next was lost");
        assert_eq!(restarted.history.len(), 1);
        assert_eq!(restarted.playback.volume, 41);
        assert_eq!(restarted.playback.position_ms, 30_000);
        assert_eq!(restarted.pending_playlist, Some(PlaylistId(77)));
        assert_eq!(restarted.selected, 4, "the cursor started somewhere else");
        assert_eq!(restarted.scroll, 2);
        let playlist_cursor = restarted
            .cursors
            .get(&ListId::Playlist(PlaylistId(77)))
            .unwrap();
        assert_eq!((playlist_cursor.selected, playlist_cursor.scroll), (3, 1));

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn the_track_that_was_playing_comes_back_but_is_not_claimed_to_be_playing() {
        let mut state = persistable("track");
        state.queue.replace_items((1..=3).map(song).collect());
        state.queue.select(1);
        state.current_song = Some(song(2));
        state.playback.position_ms = 12_000;
        state.persist_state();

        let mut restarted = persistable("track-again");
        let (saved, _) = StateFile::load(&state.paths.state_file());
        restarted.restore(saved);

        assert_eq!(
            restarted.current_song.as_ref().map(|s| s.id),
            Some(SongId(2))
        );
        assert_eq!(restarted.playback.position_ms, 12_000);
        // No sound came out of this process, so nothing may say it did.
        assert_eq!(restarted.playback.status, PlaybackStatus::Stopped);
        assert!(
            restarted.resume_on_play,
            "the track cannot be started again"
        );

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn pressing_play_starts_the_track_that_came_back_from_last_time() {
        let mut state = signed_in();
        state.queue.replace_items((1..=3).map(song).collect());
        state.current_song = Some(song(2));
        state.resume_on_play = true;
        let before = state.play_generation;

        let tx = channel();
        state.handle_action(Action::PlayPause, &tx).await;

        assert!(state.play_generation > before, "the track was not started");
        assert!(!state.resume_on_play);
        // Started for real, which means asking the service for its source.
        assert!(state.pending > 0);
    }

    #[tokio::test]
    async fn the_playlist_that_was_open_is_reopened_once_the_list_arrives() {
        // Only the id is written down: whether the playlist still exists is the
        // service's to say, so the answer is waited for rather than assumed.
        let mut state = signed_in();
        state.pending_playlist = Some(PlaylistId(2));
        let tx = channel();

        state.set_playlists(
            vec![
                playlist(1, PlaylistKind::Created),
                playlist(2, PlaylistKind::Collected),
            ],
            &tx,
        );

        assert_eq!(
            state.opened_playlist.as_ref().map(|p| p.id),
            Some(PlaylistId(2))
        );
        assert!(state.pending_playlist.is_none());
        assert!(matches!(state.pane(), Pane::Songs));
    }

    #[tokio::test]
    async fn a_cursor_restored_from_last_time_is_pulled_back_inside_a_shorter_list() {
        // A position remembered from last time can point past the end of a list
        // that has since shrunk — playlists get deleted. Left alone, the cursor
        // sits on no row at all and the pane looks empty.
        let mut state = signed_in();
        state.selected = 30;
        state.scroll = 28;
        let tx = channel();

        state.set_playlists(vec![playlist(1, PlaylistKind::Created)], &tx);

        assert_eq!(state.selected, 0, "the cursor was left past the end");
        assert_eq!(state.scroll, 0);
    }

    #[tokio::test]
    async fn the_playlist_list_arriving_does_not_clamp_the_playlist_it_reopens() {
        // The two happen in the same call, and only the first may be measured
        // against the list that just arrived: the track list is still empty at
        // that moment, so clamping a remembered track cursor against it would
        // throw the position away exactly when it is being restored.
        let mut state = signed_in();
        state.pending_playlist = Some(PlaylistId(2));
        state.cursors.insert(
            ListId::Playlist(PlaylistId(2)),
            Cursor {
                selected: 5,
                scroll: 3,
            },
        );
        let tx = channel();

        state.set_playlists(
            vec![
                playlist(1, PlaylistKind::Created),
                playlist(2, PlaylistKind::Created),
            ],
            &tx,
        );

        assert_eq!(
            state.opened_playlist.as_ref().map(|p| p.id),
            Some(PlaylistId(2))
        );
        assert_eq!(
            (state.selected, state.scroll),
            (5, 3),
            "the track cursor was measured against an empty list"
        );
    }

    #[tokio::test]
    async fn a_playlist_that_is_no_longer_there_falls_back_to_the_list() {
        // It can be deleted on another device between one run and the next, and
        // the answer to that is the playlist list — not an error, and not a
        // window that cannot be left.
        let mut state = signed_in();
        state.pending_playlist = Some(PlaylistId(999));
        let tx = channel();

        state.set_playlists(vec![playlist(1, PlaylistKind::Created)], &tx);

        assert!(state.opened_playlist.is_none());
        assert_eq!(state.pane(), Pane::Playlists);
        assert!(state
            .notice
            .as_ref()
            .is_some_and(|notice| notice.text.contains("上次浏览的歌单")));
    }

    #[tokio::test]
    async fn a_state_file_that_cannot_be_read_is_reported_and_shrugged_off() {
        // A damaged file costs the user their queue; it must not cost them the
        // program. What is checked here is that the fallback is a usable state
        // rather than an empty one that breaks something later.
        let mut state = persistable("damaged");
        std::fs::write(state.paths.state_file(), "{ not json").unwrap();

        let (saved, error) = StateFile::load(&state.paths.state_file());
        assert!(error.is_some());
        state.warn(format!("无法恢复上次的状态：{}", error.unwrap()));
        state.restore(saved);

        assert_eq!(state.playback.volume, 72, "the volume fell to silence");
        assert!(state.queue.is_empty());
        assert!(state.current_song.is_none());
        // And the message is waiting its turn to be shown.
        assert_eq!(state.warnings.len(), 1);

        std::fs::remove_dir_all(state.paths.config_dir().parent().unwrap()).unwrap();
    }

    /// A state with a session, which is what playing anything requires.
    fn signed_in() -> AppState {
        let mut state = test_state();
        state.user = Some(User {
            id: niveus_music_core::UserId(1),
            name: "listener".into(),
        });
        state
    }

    fn song(id: u64) -> Song {
        Song {
            id: SongId(id),
            title: format!("song {id}"),
            artists: Vec::new(),
            album: None,
            duration_ms: None,
        }
    }

    fn song_with_cover(id: u64, url: &str) -> Song {
        Song {
            album: Some(niveus_music_core::Album {
                id: niveus_music_core::AlbumId(id),
                name: "album".into(),
                cover_url: Some(url.to_string()),
            }),
            ..song(id)
        }
    }

    fn channel() -> Sender<AppEvent> {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        // Keep the receiver alive for the duration of the test.
        std::mem::forget(rx);
        tx
    }

    fn test_state() -> AppState {
        let paths = Paths::discover().expect("a home directory exists in tests");
        let provider = Arc::new(NcmProvider::anonymous());
        AppState::new(paths, Config::default(), provider, Vec::new())
    }
}
