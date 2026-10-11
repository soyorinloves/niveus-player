//! Whole-frame checks.
//!
//! The design makes two structural demands that are easy to satisfy by accident
//! and easy to break by accident: every section is separated from its
//! neighbours by a horizontal rule spanning the full width, and those rules are
//! never replaced by a box drawn around a section. Both are asserted here on a
//! real rendered frame rather than on a single widget.

use niveus_config::CoverSize;
use niveus_config::{Action, Key, Keymap};
use niveus_music_core::{
    Album, AlbumId, Artist, ArtistId, PlaybackMode, PlaybackStatus, Playlist, PlaylistId,
    PlaylistKind, Song, SongId, User, UserId,
};
use niveus_ui::{
    compute, render, LayoutTier, ListView, LyricsView, NowPlayingView, Page, SessionView,
    StatusView, Theme, UiState,
};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Terminal;

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 120,
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
}

fn draw<'a>(
    list: ListView<'a>,
    keymap: &'a Keymap,
    user: &'a User,
    tier: LayoutTier,
    loading: bool,
) -> Frame {
    draw_with(list, keymap, user, tier, loading, None)
}

fn draw_with<'a>(
    list: ListView<'a>,
    keymap: &'a Keymap,
    user: &'a User,
    tier: LayoutTier,
    loading: bool,
    notice: Option<&'a str>,
) -> Frame {
    let state = UiState {
        page: Page::Playlist,
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

#[test]
fn the_sections_are_separated_by_full_width_rules() {
    let keymap = Keymap::default();
    let owner = user();
    let created = vec![playlist(1, "Rock", PlaylistKind::Created)];
    let collected = vec![playlist(2, "Post-rock", PlaylistKind::Collected)];
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
fn the_two_panes_are_split_by_a_vertical_rule() {
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

    let rule = frame.regions.pane_rule.expect("two panes are shown");
    let body = frame.regions.body;
    for y in body.y..body.y + body.height {
        assert_eq!(
            frame.buffer.cell((rule.x, y)).unwrap().symbol(),
            "│",
            "the pane rule is broken at row {y}"
        );
    }
}

#[test]
fn the_standard_layout_shows_every_region() {
    let keymap = Keymap::default();
    let owner = user();
    let playlists = vec![playlist(1, "Rock", PlaylistKind::Created)];
    let frame = draw(
        ListView::Playlists {
            created: &playlists,
            collected: &[],
            selected: 0,
            scroll: 0,
        },
        &keymap,
        &owner,
        LayoutTier::Standard,
        false,
    );

    let nav = frame.compact(frame.regions.nav);
    assert!(nav.contains("PLAYLIST[1]"), "{nav}");
    assert!(nav.contains("FAVORITES[2]"), "{nav}");
    assert!(nav.contains("HISTORY[3]"), "{nav}");
    assert!(nav.contains("SETTINGS[4]"), "{nav}");

    let list = frame.compact(frame.regions.list);
    assert!(list.contains("PLAYLISTS"), "{list}");
    assert!(list.contains("CREATED"), "{list}");
    assert!(list.contains("▶Rock"), "{list}");

    let right = frame.compact(frame.regions.now_playing.unwrap());
    assert!(right.contains("NOWPLAYING"), "{right}");
    // Lyrics are the caption for the region, the metadata is the switch
    // (SPEC §12.1) — and only the switch carries a key.
    assert!(right.contains("LYRICSMETADATA[M]"), "{right}");
    assert!(!right.contains("LYRICS["), "{right}");

    let status = frame.compact(frame.regions.status);
    assert!(status.contains("VOL72%[-/=]"), "{status}");
    assert!(status.contains("QUEUE12[U]"), "{status}");
    assert!(status.contains("SEQUENTIAL[O]"), "{status}");
    assert!(status.contains("PLAY[Space]"), "{status}");
}

#[test]
fn the_minimal_layout_hides_the_now_playing_pane() {
    let keymap = Keymap::default();
    let owner = user();
    let area = Rect::new(0, 0, 100, 30);
    let state = UiState {
        page: Page::Playlist,
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

    // The navigation survives...
    assert!(frame.compact(frame.regions.nav).contains("PLAYLIST[1]"));
    // ...and the second pane is gone entirely.
    assert!(frame.regions.now_playing.is_none());
    assert!(!frame.compact(frame.buffer.area).contains("NOWPLAYING"));
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

    let list = frame.compact(frame.regions.list);
    assert!(list.contains("CURRENTPLAYLIST‹Rock"), "{list}");
    assert!(list.contains("01SunnyDay"), "{list}");
    assert!(list.contains("Artist·Album"), "{list}");
    assert!(list.contains("03:35"), "{list}");
    // The cursor marks the second row only.
    assert!(list.contains("▶02RainyNight"), "{list}");
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
    assert!(frame.compact(frame.regions.list).contains("Rock"));
}

#[test]
fn rebinding_a_key_moves_the_hint_with_it() {
    let mut keymap = Keymap::default();
    keymap.rebind(Action::ToggleQueue, Key::Char('z')).unwrap();
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
    assert!(!status.contains("[U]"), "{status}");
}

#[test]
fn the_too_small_screen_explains_itself() {
    let keymap = Keymap::default();
    let owner = user();
    let area = Rect::new(0, 0, 40, 10);
    let state = UiState {
        page: Page::Playlist,
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

    let list = frame.regions.list;
    let credit_row = (list.y..list.y + list.height)
        .map(|y| row_text(&frame.buffer, list, y))
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
    );

    let status = frame.compact(frame.regions.status);
    assert!(status.contains("播放控制尚未接入"), "{status}");

    // Reminders are either wholly present or wholly absent — never cut off at
    // the edge of the row.
    for fragment in ["MOVE", "OPEN", "QUEUE", "QUIT"] {
        let start = status.find(fragment);
        if let Some(start) = start {
            let rest = &status[start..];
            assert!(
                rest.contains(']'),
                "the reminder {fragment:?} was clipped: {status}"
            );
        }
    }
}
