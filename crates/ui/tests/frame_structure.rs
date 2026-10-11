//! Whole-frame checks.
//!
//! The design makes structural demands that are easy to satisfy by accident and
//! easy to break by accident: every section is separated from its neighbours by
//! a rule spanning the full width, side-by-side columns by a rule spanning the
//! full height, and no section is ever fenced off with a box. All of them are
//! asserted here on a real rendered frame rather than on a single widget.

use niveus_config::CoverSize;
use niveus_config::{Action, Key, Keymap};
use niveus_music_core::{
    Album, AlbumId, Artist, ArtistId, PlaybackMode, PlaybackStatus, Playlist, PlaylistId,
    PlaylistKind, Song, SongId, User, UserId,
};
use niveus_ui::{
    compute, render, Focus, LayoutTier, ListView, LyricsView, NowPlayingView, Primary, Section,
    SessionView, StatusView, Theme, UiState,
};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Terminal;

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 140,
    height: 40,
};

/// Box-drawing characters that would mean a section is fenced off rather than
/// separated by a rule.
const FORBIDDEN_CORNERS: [char; 8] = ['┌', '┐', '└', '┘', '├', '┤', '┬', '┴'];

fn user() -> User {
    User {
        id: UserId(1),
        name: "listener".into(),
    }
}

fn playlist(id: u64, name: &str, kind: PlaylistKind) -> Playlist {
    Playlist {
        id: PlaylistId(id),
        name: name.into(),
        kind,
    }
}

/// A track whose title, artist and album all run past the pane.
fn long_chinese_song() -> Song {
    Song {
        id: SongId(9),
        title: "这是一首名字特别特别长的中文歌曲标题专门用来测试截断行为是否按显示列宽工作".into(),
        artists: vec![Artist {
            id: ArtistId(7),
            name: "演唱者名字也很长的歌手名字同样很长".into(),
        }],
        album: Some(Album {
            id: AlbumId(8),
            name: "专辑名字同样很长的专辑名字同样很长".into(),
            cover_url: None,
        }),
        duration_ms: Some(261_000),
    }
}

fn song(id: u64, title: &str) -> Song {
    Song {
        id: SongId(id),
        title: title.into(),
        artists: vec![Artist {
            id: ArtistId(2),
            name: "Artist".into(),
        }],
        album: Some(Album {
            id: AlbumId(3),
            name: "Album".into(),
            cover_url: None,
        }),
        duration_ms: Some(215_000),
    }
}

struct Frame {
    buffer: Buffer,
    regions: niveus_ui::Regions,
}

impl Frame {
    fn text(&self, area: Rect) -> String {
        (area.y..area.y + area.height)
            .map(|y| {
                (area.x..area.x + area.width)
                    .map(|x| self.buffer.cell((x, y)).unwrap().symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// As above, with wide-character padding removed.
    fn compact(&self, area: Rect) -> String {
        self.text(area)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    fn row(&self, y: u16) -> String {
        (0..self.buffer.area.width)
            .map(|x| self.buffer.cell((x, y)).unwrap().symbol())
            .collect()
    }

    fn is_full_rule(&self, area: Rect) -> bool {
        (area.x..area.x + area.width)
            .all(|x| self.buffer.cell((x, area.y)).unwrap().symbol() == "─")
    }

    /// The cell at a column of the body, down every row of it.
    fn column_is_rule(&self, x: u16) -> bool {
        let body = self.regions.body;
        (body.y..body.y + body.height).all(|y| self.buffer.cell((x, y)).unwrap().symbol() == "│")
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_with<'a>(
    list: ListView<'a>,
    keymap: &'a Keymap,
    user: &'a User,
    tier: LayoutTier,
    loading: bool,
    notice: Option<&'a str>,
    focus: Focus,
) -> Frame {
    let state = UiState {
        primary: Primary::Library,
        section: Section::Playlists,
        focus,
        session: SessionView::Ready { user },
        list,
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
            loading,
            notice,
            volume: 72,
            playback_mode: PlaybackMode::Sequential,
            queue_len: 12,
            playback: PlaybackStatus::Stopped,
            bitrate: None,
        },
        keymap,
        tier,
        cover_size: CoverSize::Normal,
    };

    let mut terminal = Terminal::new(TestBackend::new(AREA.width, AREA.height)).unwrap();
    terminal
        .draw(|frame| render(frame, &state, &Theme::default()))
        .unwrap();

    Frame {
        buffer: terminal.backend().buffer().clone(),
        regions: compute(AREA, tier, CoverSize::Normal),
    }
}

fn draw<'a>(
    list: ListView<'a>,
    keymap: &'a Keymap,
    user: &'a User,
    tier: LayoutTier,
    loading: bool,
) -> Frame {
    draw_with(list, keymap, user, tier, loading, None, Focus::Main)
}

/// Rebuilds a row's text, stepping over the cell each wide glyph covers.
fn row_text(buffer: &Buffer, area: Rect, y: u16) -> String {
    let mut text = String::new();
    let mut x = area.x;
    while x < area.x + area.width {
        let symbol = buffer.cell((x, y)).unwrap().symbol();
        text.push_str(symbol);
        let width = unicode_width::UnicodeWidthStr::width(symbol);
        x += if width >= 2 { width as u16 } else { 1 };
    }
    text
}

fn row_width(buffer: &Buffer, area: Rect, y: u16) -> usize {
    unicode_width::UnicodeWidthStr::width(row_text(buffer, area, y).as_str())
}

fn playlists() -> (Vec<Playlist>, Vec<Playlist>) {
    (
        vec![playlist(1, "Rock", PlaylistKind::Created)],
        vec![playlist(2, "Post-rock", PlaylistKind::Collected)],
    )
}

#[test]
fn the_sections_are_separated_by_full_width_rules() {
    let keymap = Keymap::default();
    let owner = user();
    let (created, collected) = playlists();
    let frame = draw(
        ListView::Playlists {
            created: &created,
            collected: &collected,
            selected: 0,
            scroll: 0,
        },
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
    );

    assert!(
        frame.is_full_rule(frame.regions.nav_rule),
        "the rule under the navigation is not full width:\n{}",
        frame.row(frame.regions.nav_rule.y)
    );
    assert!(
        frame.is_full_rule(frame.regions.status_rule),
        "the rule above the status bar is not full width"
    );
}

#[test]
fn no_section_is_fenced_off_with_a_box() {
    let keymap = Keymap::default();
    let owner = user();
    let created = vec![playlist(1, "Rock", PlaylistKind::Created)];
    let frame = draw(
        ListView::Playlists {
            created: &created,
            collected: &[],
            selected: 0,
            scroll: 0,
        },
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
    );

    let everything = frame.text(frame.buffer.area);
    for corner in FORBIDDEN_CORNERS {
        assert!(
            !everything.contains(corner),
            "found {corner}, which means a section was boxed"
        );
    }
}

#[test]
fn the_columns_are_split_by_vertical_rules() {
    let keymap = Keymap::default();
    let owner = user();
    let created = vec![playlist(1, "Rock", PlaylistKind::Created)];
    let frame = draw(
        ListView::Playlists {
            created: &created,
            collected: &[],
            selected: 0,
            scroll: 0,
        },
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
    );

    assert!(
        frame.column_is_rule(frame.regions.secondary_rule.x),
        "the rule between the secondary column and the main area is broken"
    );
    let pane_rule = frame.regions.pane_rule.expect("three columns are shown");
    assert!(
        frame.column_is_rule(pane_rule.x),
        "the rule between the main area and the now-playing pane is broken"
    );
}

#[test]
fn the_standard_layout_shows_every_region() {
    let keymap = Keymap::default();
    let owner = user();
    let (created, collected) = playlists();
    let frame = draw(
        ListView::Playlists {
            created: &created,
            collected: &collected,
            selected: 0,
            scroll: 0,
        },
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
    );

    let nav = frame.compact(frame.regions.nav);
    assert!(nav.contains("LIBRARY[1]"), "{nav}");
    assert!(nav.contains("QUEUE[3]"), "{nav}");
    assert!(nav.contains("SETTINGS[4]"), "{nav}");
    // SEARCH is reserved, not drawn.
    assert!(!nav.contains("SEARCH"), "{nav}");

    let secondary = frame.compact(frame.regions.secondary);
    assert!(secondary.contains("LIKED"), "{secondary}");
    assert!(secondary.contains("PLAYLISTS"), "{secondary}");
    assert!(secondary.contains("HISTORY"), "{secondary}");

    let main = frame.compact(frame.regions.main);
    assert!(main.contains("PLAYLISTS"), "{main}");
    assert!(main.contains("CREATED"), "{main}");
    assert!(main.contains("▶Rock"), "{main}");

    let right = frame.compact(frame.regions.now_playing.unwrap());
    assert!(right.contains("NOWPLAYING"), "{right}");
    assert!(right.contains("LYRICSMETADATA[M]"), "{right}");
    assert!(!right.contains("LYRICS["), "{right}");

    let status = frame.compact(frame.regions.status);
    assert!(status.contains("VOL72%[-/=]"), "{status}");
    assert!(status.contains("QUEUE12[3]"), "{status}");
    assert!(status.contains("SEQUENTIAL[O]"), "{status}");
    assert!(status.contains("PLAY[Space]"), "{status}");
}

#[test]
fn the_minimal_layout_keeps_the_secondary_column_and_hides_the_pane() {
    let keymap = Keymap::default();
    let owner = user();
    let area = Rect::new(0, 0, 100, 30);
    let state = UiState {
        primary: Primary::Library,
        section: Section::Playlists,
        focus: Focus::Main,
        session: SessionView::Ready { user: &owner },
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
        tier: LayoutTier::Minimal,
        cover_size: CoverSize::Normal,
    };
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| render(frame, &state, &Theme::default()))
        .unwrap();
    let frame = Frame {
        buffer: terminal.backend().buffer().clone(),
        regions: compute(area, LayoutTier::Minimal, CoverSize::Normal),
    };

    // The navigation and the secondary column survive...
    assert!(frame.compact(frame.regions.nav).contains("LIBRARY[1]"));
    assert!(frame.compact(frame.regions.secondary).contains("PLAYLISTS"));
    // ...and the third column is gone entirely.
    assert!(frame.regions.now_playing.is_none());
    assert!(!frame.compact(frame.buffer.area).contains("NOWPLAYING"));
}

#[test]
fn the_focus_marker_moves_between_the_two_regions() {
    let keymap = Keymap::default();
    let owner = user();
    let (created, collected) = playlists();
    let list = || ListView::Playlists {
        created: &created,
        collected: &collected,
        selected: 0,
        scroll: 0,
    };

    let accent = Theme::default().accent.fg.unwrap();
    // PLAYLISTS is the second item of LIBRARY's three, and Rock the first row
    // of the main area's list (below its heading and rule).
    let cell =
        |frame: &Frame, area: Rect, row: u16| frame.buffer.cell((area.x, row)).unwrap().clone();

    // There is one cursor, and it is in the region that holds the focus; the
    // other region keeps its selection only as a dimmed label. That is what
    // makes the arrow keys' target visible (SPEC §10.3).
    let main_focused = draw_with(
        list(),
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
        None,
        Focus::Main,
    );
    let secondary = &main_focused.regions.secondary;
    let main = &main_focused.regions.main;
    let first_row = main.y + 3; // heading, rule, CREATED, then Rock.
    assert_eq!(
        cell(&main_focused, *secondary, secondary.y + 1).symbol(),
        " "
    );
    assert_eq!(cell(&main_focused, *main, first_row).symbol(), "▶");
    assert_eq!(cell(&main_focused, *main, first_row).fg, accent);

    // Tab swaps them.
    let secondary_focused = draw_with(
        list(),
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
        None,
        Focus::Secondary,
    );
    assert_eq!(
        cell(&secondary_focused, *secondary, secondary.y + 1).symbol(),
        "▶"
    );
    assert_eq!(
        cell(&secondary_focused, *secondary, secondary.y + 1).fg,
        accent
    );
    assert_eq!(cell(&secondary_focused, *main, first_row).symbol(), " ");
}

#[test]
fn a_playlist_shows_its_tracks_with_credits_and_durations() {
    let keymap = Keymap::default();
    let owner = user();
    let songs = vec![song(1, "Sunny Day"), song(2, "Rainy Night")];
    let frame = draw(
        ListView::Songs {
            playlist_name: "Rock",
            songs: &songs,
            selected: 1,
            scroll: 0,
        },
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
    );

    let main = frame.compact(frame.regions.main);
    assert!(main.contains("CURRENTPLAYLIST‹Rock"), "{main}");
    assert!(main.contains("01SunnyDay"), "{main}");
    assert!(main.contains("Artist·Album"), "{main}");
    assert!(main.contains("03:35"), "{main}");
    // The cursor marks the second row only.
    assert!(main.contains("▶02RainyNight"), "{main}");
}

#[test]
fn loading_is_reported_without_blanking_the_list() {
    let keymap = Keymap::default();
    let owner = user();
    let created = vec![playlist(1, "Rock", PlaylistKind::Created)];
    let frame = draw(
        ListView::Playlists {
            created: &created,
            collected: &[],
            selected: 0,
            scroll: 0,
        },
        &keymap,
        &owner,
        LayoutTier::Standard,
        true,
    );

    assert!(frame.compact(frame.regions.status).contains("LOADING…"));
    // The list keeps showing what it had rather than being replaced by a
    // placeholder.
    assert!(frame.compact(frame.regions.main).contains("Rock"));
}

#[test]
fn rebinding_a_key_moves_the_hint_with_it() {
    let mut keymap = Keymap::default();
    keymap.rebind(Action::ShowQueue, Key::Char('z')).unwrap();
    let owner = user();
    let frame = draw(
        ListView::Message(""),
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
    );

    let status = frame.compact(frame.regions.status);
    assert!(status.contains("QUEUE12[Z]"), "{status}");
    assert!(!status.contains("[3]"), "{status}");
}

#[test]
fn the_too_small_screen_explains_itself() {
    let keymap = Keymap::default();
    let owner = user();
    let area = Rect::new(0, 0, 40, 10);
    let state = UiState {
        primary: Primary::Library,
        section: Section::Playlists,
        focus: Focus::Main,
        session: SessionView::Ready { user: &owner },
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
        tier: LayoutTier::TooSmall,
        cover_size: CoverSize::Normal,
    };
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| render(frame, &state, &Theme::default()))
        .unwrap();

    let text: String = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| terminal.backend().buffer().cell((x, y)).unwrap().symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(text.contains("Terminal size too small"), "{text}");
    assert!(text.contains("40×10"), "{text}");
    assert!(text.contains("80×24"), "{text}");
}

// ---------------------------------------------------------------------------
// Display width with Chinese content.
//
// Every one of these passes trivially on ASCII content, because there a
// character and a column are the same thing. They only mean something once the
// interface is showing Chinese, which for this application is the normal case.
// ---------------------------------------------------------------------------

#[test]
fn chinese_content_keeps_every_row_inside_its_region() {
    let keymap = Keymap::default();
    let owner = user();

    for notice in [None, Some("播放控制尚未接入，当前版本仅支持浏览")] {
        let songs = vec![long_chinese_song()];
        let frame = draw_with(
            ListView::Songs {
                playlist_name: "一个名字非常非常长的中文歌单名字用来测试标题行的截断行为",
                songs: &songs,
                selected: 0,
                scroll: 0,
            },
            &keymap,
            &owner,
            LayoutTier::Standard,
            false,
            notice,
            Focus::Main,
        );

        for y in 0..AREA.height {
            assert_eq!(
                row_width(&frame.buffer, AREA, y),
                AREA.width as usize,
                "row {y} does not span the frame: {:?}",
                row_text(&frame.buffer, AREA, y)
            );
        }
    }
}

#[test]
fn a_long_chinese_title_leaves_the_duration_column_in_place() {
    let keymap = Keymap::default();
    let owner = user();
    let songs = vec![long_chinese_song()];
    let frame = draw(
        ListView::Songs {
            playlist_name: "很长的中文歌单名字",
            songs: &songs,
            selected: 0,
            scroll: 0,
        },
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
    );

    let main = frame.regions.main;
    let credit_row = (main.y..main.y + main.height)
        .map(|y| row_text(&frame.buffer, main, y))
        .find(|row| row.contains("04:21"))
        .expect("the duration column disappeared");

    // Flush against the pane's right edge, not stranded mid-row.
    assert!(credit_row.trim_end().ends_with("04:21"), "{credit_row:?}");
    // And the credit it displaced says so rather than just stopping.
    assert!(credit_row.contains('…'), "{credit_row:?}");
}

#[test]
fn a_chinese_notice_never_leaves_a_half_printed_reminder() {
    let keymap = Keymap::default();
    let owner = user();
    let frame = draw_with(
        ListView::Message(""),
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
        Some("播放控制尚未接入，当前版本仅支持浏览"),
        Focus::Main,
    );

    let status = frame.compact(frame.regions.status);
    assert!(status.contains("播放控制尚未接入"), "{status}");

    // Reminders are either wholly present or wholly absent — never cut off at
    // the edge of the row.
    for fragment in ["MOVE", "OPEN", "FOCUS", "QUIT"] {
        if let Some(start) = status.find(fragment) {
            assert!(
                status[start..].contains(']'),
                "the reminder {fragment:?} was clipped: {status}"
            );
        }
    }
}
