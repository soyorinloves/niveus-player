//! Prints the interface as plain text, so the layout can be inspected without a
//! terminal or a signed-in account.
//!
//! ```text
//! cargo run -p niveus-ui --example preview                    # Standard, playlist list
//! cargo run -p niveus-ui --example preview -- songs           # Standard, track list
//! cargo run -p niveus-ui --example preview -- minimal         # Minimal tier
//! cargo run -p niveus-ui --example preview -- small           # Too small
//! cargo run -p niveus-ui --example preview -- playing         # a track, art and progress
//! cargo run -p niveus-ui --example preview -- queue           # the queue view
//! cargo run -p niveus-ui --example preview -- lyrics          # the lyric window
//! cargo run -p niveus-ui --example preview -- lyrics --metadata  # window + metadata block
//! cargo run -p niveus-ui --example preview -- lyrics-none     # ♪ 纯音乐
//! cargo run -p niveus-ui --example preview -- lyrics-translation  # 暂无原文歌词
//! cargo run -p niveus-ui --example preview -- lyrics-failed   # 歌词加载失败
//! cargo run -p niveus-ui --example preview -- lyrics --cover large
//! cargo run -p niveus-ui --example preview -- history          # the history page
//! cargo run -p niveus-ui --example preview -- settings         # the settings list
//! cargo run -p niveus-ui --example preview -- settings-edit    # a value being changed
//! cargo run -p niveus-ui --example preview -- keybindings      # the keybinding list
//! ```
//!
//! The lyric window is drawn five rows tall here. How tall it really is depends
//! on the panel, which only the app knows: for that, use the app's own preview
//! (`cargo run -p niveus-app --example screen -- --cover-size large --metadata`).
//!
//! Colours and bold/dim are not shown: this prints the characters and the
//! arrangement, which is what the structural rules are about.

use niveus_config::{CoverSize, Keymap};
use niveus_music_core::{
    Album, AlbumId, Artist, ArtistId, AudioQuality, AudioSource, PlaybackMode, PlaybackStatus,
    PlayedSong, Playlist, PlaylistId, PlaylistKind, Song, SongId, User, UserId,
};
use niveus_ui::{
    render, LayoutTier, ListView, LyricWindow, LyricsView, NowPlayingView, Page, SessionView,
    Setting, SettingsValues, StatusView, Theme, UiState,
};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

fn main() {
    // The first argument is what to draw; the rest adjust the pane.
    let mut arguments = std::env::args().skip(1);
    let mode = arguments.next().unwrap_or_else(|| "playlists".into());
    let mut cover_size = CoverSize::Normal;
    let mut metadata = mode == "metadata";
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--metadata" => metadata = true,
            "--cover" => {
                let wanted = arguments.next().unwrap_or_default();
                cover_size = niveus_config::CoverSize::ALL
                    .into_iter()
                    .find(|size| size.name() == wanted)
                    .unwrap_or_else(|| {
                        panic!("封面尺寸应为 small / normal / large，收到 {wanted}")
                    });
            }
            other => panic!("未知参数 {other}"),
        }
    }

    // Held here rather than built inside the arms, so the list can borrow them
    // and the track playing can come from the same place.
    let song_list = songs();
    let long_list = long_songs();
    let created = created();
    let collected = collected();
    let history = history();

    let (area, tier, list) = match mode.as_str() {
        "songs" => (
            Rect::new(0, 0, 120, 32),
            LayoutTier::Standard,
            ListView::Songs {
                playlist_name: "我喜欢的摇滚",
                songs: &song_list,
                selected: 0,
                scroll: 0,
            },
        ),
        // Everything deliberately too long, to show the rows being shortened
        // rather than overrunning their region.
        "long" => (
            Rect::new(0, 0, 120, 32),
            LayoutTier::Standard,
            ListView::Songs {
                playlist_name: "一个名字特别长的中文歌单名字专门用来演示标题行的截断行为是否生效",
                songs: &long_list,
                selected: 1,
                scroll: 0,
            },
        ),
        "minimal" => (
            Rect::new(0, 0, 100, 28),
            LayoutTier::Minimal,
            ListView::Playlists {
                created: &created,
                collected: &collected,
                selected: 0,
                scroll: 0,
            },
        ),
        // A Chinese notice on the status bar: the reminder hints must either
        // fit whole or be dropped, never clipped.
        "notice" => (
            Rect::new(0, 0, 100, 24),
            LayoutTier::Minimal,
            ListView::Songs {
                playlist_name: "我喜欢的摇滚",
                songs: &song_list,
                selected: 0,
                scroll: 0,
            },
        ),
        "small" => (
            Rect::new(0, 0, 60, 16),
            LayoutTier::TooSmall,
            ListView::Message(""),
        ),
        // The queue view, with one row marked as the track playing.
        "queue" => (
            Rect::new(0, 0, 120, 32),
            LayoutTier::Standard,
            ListView::Queue {
                songs: &song_list,
                playing: Some(0),
                selected: 1,
                scroll: 0,
            },
        ),
        // The history page: a playing entry, a long one, and the date column on
        // the right.
        "history" => (
            Rect::new(0, 0, 120, 32),
            LayoutTier::Standard,
            ListView::History {
                entries: &history,
                selected: 1,
                scroll: 0,
            },
        ),
        // The settings list, and the same list with a value being changed.
        "settings" | "settings-edit" => (
            Rect::new(0, 0, 120, 32),
            LayoutTier::Standard,
            ListView::Settings {
                values: SettingsValues {
                    quality: "exhigh",
                    mode: "Sequential",
                    volume: 72,
                    cover_size: "normal",
                },
                selected: 4,
                editing: (mode == "settings-edit").then_some(Setting::CoverSize),
            },
        ),
        // The keybinding list, which is drawn from the live keymap.
        "keybindings" => (
            Rect::new(0, 0, 120, 32),
            LayoutTier::Standard,
            ListView::Keybindings {
                selected: 1,
                scroll: 0,
                capturing: false,
            },
        ),
        // A track playing, with the default cover and a position part way in.
        "playing" => (
            Rect::new(0, 0, 120, 32),
            LayoutTier::Standard,
            ListView::Songs {
                playlist_name: "我喜欢的摇滚",
                songs: &song_list,
                selected: 0,
                scroll: 0,
            },
        ),
        _ => (
            Rect::new(0, 0, 120, 32),
            LayoutTier::Standard,
            ListView::Playlists {
                created: &created,
                collected: &collected,
                selected: 0,
                scroll: 0,
            },
        ),
    };

    let keymap = Keymap::default();
    let user = User {
        id: UserId(1),
        name: "listener".into(),
    };
    // Every mode that wants a track in the pane, including the ones showing the
    // lyrics panel with nothing to put in it: an empty panel in an otherwise
    // empty pane would not show what the state looks like in use.
    let playing = matches!(
        mode.as_str(),
        "playing" | "lyrics" | "lyrics-none" | "lyrics-translation" | "lyrics-failed" | "metadata"
    );
    let now_playing_song = playing.then(|| &song_list[0]);

    // The resource the metadata panel reads its quality and format from, and
    // the bitrate the status bar reports.
    let source = AudioSource {
        url: "https://example.invalid/song.flac".into(),
        level: Some(AudioQuality::Lossless),
        format: Some("flac".into()),
        bitrate: Some(999_000),
        is_trial: false,
    };

    let lyrics = match mode.as_str() {
        "lyrics" | "metadata" => LyricsView::Window(LyricWindow::new(vec![
            Some("故事的小黄花"),
            Some("从出生那年就飘着"),
            Some("童年的荡秋千"),
        ])),
        "lyrics-none" => LyricsView::Instrumental,
        "lyrics-translation" => LyricsView::TranslationOnly,
        "lyrics-failed" => LyricsView::Failed,
        _ => LyricsView::Pending,
    };

    let state = UiState {
        page: Page::Playlist,
        session: SessionView::Ready { user: &user },
        list,
        now_playing: NowPlayingView {
            // Without a track the pane shows its structure and dashes.
            song: now_playing_song,
            metadata,
            position_ms: playing.then_some(97_000),
            duration_ms: playing.then_some(269_000),
            // A picture needs the crate that draws covers, which this crate
            // deliberately does not depend on; the app's own preview shows one.
            artwork: None,
            lyrics,
            source: playing.then_some(&source),
        },
        status: StatusView {
            loading: false,
            notice: match mode.as_str() {
                "notice" => Some("播放控制尚未接入，当前版本仅支持浏览"),
                _ => None,
            },
            volume: 72,
            playback_mode: PlaybackMode::Sequential,
            queue_len: 12,
            playback: if playing {
                PlaybackStatus::Playing
            } else {
                PlaybackStatus::Stopped
            },
            bitrate: playing.then_some(999_000),
        },
        keymap: &keymap,
        tier,
        cover_size,
    };

    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| render(frame, &state, &Theme::default()))
        .unwrap();

    let buffer = terminal.backend().buffer();
    println!("┄┄ {mode} · {}×{} ┄┄", area.width, area.height);
    for y in 0..area.height {
        println!("{}", row_text(buffer, area.width, y));
    }
}

/// One row as text.
///
/// A wide character fills two cells, and reading them one by one would print a
/// gap after every Chinese character — a layout fault that is not there, in the
/// one view whose whole purpose is judging the layout. The cell a glyph covers
/// is stepped over rather than read.
fn row_text(buffer: &ratatui::buffer::Buffer, width: u16, y: u16) -> String {
    let mut text = String::new();
    let mut x = 0;
    while x < width {
        let symbol = buffer
            .cell((x, y))
            .expect("cell inside the buffer")
            .symbol();
        text.push_str(symbol);
        let width = ratatui::text::Span::raw(symbol).width();
        x += if width >= 2 { width as u16 } else { 1 };
    }
    text.trim_end().to_string()
}

/// Tracks whose titles, credits and album names are all far too long.
fn long_songs() -> Vec<Song> {
    [
        (
            "这是一首名字特别特别长的中文歌曲标题专门用来测试截断行为是否按显示列宽正确工作",
            "演唱者名字也很长的歌手组合名称同样很长需要截断处理",
            "专辑名字同样很长的专辑名称也不短需要一起被截断处理",
        ),
        ("Short One", "Brief", "Terse"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (title, artist, album))| Song {
        id: SongId(index as u64 + 1),
        title: title.into(),
        artists: vec![Artist {
            id: ArtistId(index as u64 + 10),
            name: artist.into(),
        }],
        album: Some(Album {
            id: AlbumId(index as u64 + 20),
            name: album.into(),
            cover_url: None,
        }),
        duration_ms: Some(261_000),
    })
    .collect()
}

/// Three tracks heard on three days, newest first.
///
/// 1_700_000_000 is 2023-11-14, so the dates are readable at a glance in the
/// output and in the tests that check the same arithmetic.
fn history() -> Vec<PlayedSong> {
    [
        ("晴天", 1_700_000_000u64),
        (
            "这是一首名字特别特别长的中文歌曲标题专门用来测试截断行为是否按显示列宽正确工作",
            1_699_000_000,
        ),
        ("Yellow", 1_698_000_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (title, played_at))| PlayedSong {
        song: Song {
            id: SongId(index as u64 + 1),
            title: title.into(),
            artists: vec![Artist {
                id: ArtistId(index as u64 + 10),
                name: "周杰伦".into(),
            }],
            album: Some(Album {
                id: AlbumId(index as u64 + 20),
                name: "叶惠美".into(),
                cover_url: None,
            }),
            duration_ms: Some(269_000),
        },
        played_at,
    })
    .collect()
}

fn created() -> Vec<Playlist> {
    vec![
        Playlist {
            id: PlaylistId(1),
            name: "我喜欢的摇滚".into(),
            kind: PlaylistKind::Created,
        },
        Playlist {
            id: PlaylistId(2),
            name: "民谣合集".into(),
            kind: PlaylistKind::Created,
        },
    ]
}

fn collected() -> Vec<Playlist> {
    vec![Playlist {
        id: PlaylistId(3),
        name: "后摇精选".into(),
        kind: PlaylistKind::Collected,
    }]
}

fn songs() -> Vec<Song> {
    [
        ("晴天", "周杰伦", "叶惠美", 269_000u32),
        ("海阔天空", "Beyond", "乐与怒", 326_000),
        ("Yellow", "Coldplay", "Parachutes", 269_000),
        ("夜空中最亮的星", "逃跑计划", "世界", 252_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (title, artist, album, duration))| Song {
        id: SongId(index as u64 + 1),
        title: title.into(),
        artists: vec![Artist {
            id: ArtistId(index as u64 + 10),
            name: artist.into(),
        }],
        album: Some(Album {
            id: AlbumId(index as u64 + 20),
            name: album.into(),
            cover_url: None,
        }),
        duration_ms: Some(duration),
    })
    .collect()
}
