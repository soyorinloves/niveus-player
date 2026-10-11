//! Draws the whole interface to plain text, with a track playing.
//!
//! ```text
//! cargo run -p niveus-app --example screen
//! cargo run -p niveus-app --example screen -- <cover-url>
//! cargo run -p niveus-app --example screen -- --cover-size large
//! cargo run -p niveus-app --example screen -- --metadata
//! cargo run -p niveus-app --example screen -- --size 110x34
//! cargo run -p niveus-app --example screen -- --keys tab,down,tab,3,u,2
//! ```
//!
//! `--cover-size` and `--metadata` combine, which is how the six arrangements
//! of the panel — three cover sizes, block on or off — can be looked at.
//!
//! `--size` picks the terminal, so the three responsive presets can each be
//! drawn; `--keys` feeds a comma-separated list of key presses through the same
//! event handling the real loop uses, printing a frame after each. That is how
//! the navigation, the focus and the keys that were removed can be looked at
//! without a signed-in account.
//!
//! This is the only place the two halves of a cover meet — the crate that draws
//! one and the crate that draws the screen — so it is the only preview that can
//! show album art at all. Nothing is fetched unless a URL is given: the default
//! cover is drawn from the curve.
//!
//! Colours are not shown; this is for judging the arrangement.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use niveus_app::event::AppEvent;
use niveus_app::paths::Paths;
use niveus_app::state::AppState;
use niveus_artwork::{load, CoverKey, Options, Size};
use niveus_config::{Config, CoverSize};
use niveus_music_core::{
    Album, AlbumId, Artist, ArtistId, AudioQuality, AudioSource, PlaybackMode, PlaybackStatus,
    Playlist, PlaylistId, PlaylistKind, RawLyrics, Song, SongId,
};
use niveus_ncm_provider::NcmProvider;
use niveus_ui::{render, LayoutTier, Theme};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Terminal;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let mut url: Option<String> = None;
    let mut cover_size = CoverSize::Normal;
    let mut metadata = false;
    let mut size = (140u16, 40u16);
    let mut keys: Vec<KeyEvent> = Vec::new();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--cover-size" => {
                let wanted = arguments.next().unwrap_or_default();
                cover_size = CoverSize::ALL
                    .into_iter()
                    .find(|size| size.name() == wanted)
                    .unwrap_or_else(|| {
                        panic!("cover size must be small / normal / large, got {wanted}")
                    });
            }
            "--metadata" => metadata = true,
            "--size" => size = parse_size(&arguments.next().unwrap_or_default()),
            "--keys" => {
                keys = arguments
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter(|name| !name.is_empty())
                    .map(parse_key)
                    .collect();
            }
            other => url = Some(other.to_string()),
        }
    }

    let area = Rect::new(0, 0, size.0, size.1);

    let paths = Paths::discover().expect("a home directory");
    let config = Config {
        cover_size,
        ..Config::default()
    };
    let provider = Arc::new(NcmProvider::anonymous());
    let mut state = AppState::new(paths, config, provider, Vec::new());

    state.size = (area.width, area.height);
    state.user = Some(niveus_music_core::User {
        id: niveus_music_core::UserId(960),
        name: "listener".into(),
    });
    state.opened_playlist = Some(Playlist {
        id: PlaylistId(1),
        name: "我喜欢的摇滚".into(),
        kind: PlaylistKind::Created,
    });
    let tracks = songs();
    state.songs = tracks.clone();
    state.queue.replace_items(tracks.clone());
    // One track waiting behind the rest, so UP NEXT has something to show.
    state.queue.push_next(tracks[3].clone());
    state.queue.playback_mode = PlaybackMode::ListLoop;
    state.queue.select(0);
    state.current_song = Some(state.songs[0].clone());
    state.playback.status = PlaybackStatus::Playing;
    state.playback.position_ms = 97_000;
    state.playback.duration_ms = Some(269_000);
    state.metadata = metadata;
    state.source = Some(AudioSource {
        url: "https://example.invalid/晴天.flac".into(),
        level: Some(AudioQuality::Lossless),
        format: Some("flac".into()),
        bitrate: Some(999_000),
        is_trial: false,
    });
    state.artwork = Some(artwork(area, cover_size, url.as_deref()).await);

    // The lyrics go in through the door the network reply comes through, so
    // what is drawn is what the program would draw rather than a second copy of
    // the same logic written for the preview.
    let (tx, _inbox) = mpsc::channel(4);
    state
        .handle_event(
            AppEvent::Lyrics {
                generation: state.play_generation,
                song_id: state.songs[0].id,
                result: Ok(RawLyrics::original(SAMPLE_LRC)),
            },
            &tx,
        )
        .await;

    print_frame(&state, area, "start");
    // Each key goes through the same path the real loop sends it down, so the
    // frame after it is the frame the program would draw.
    for key in keys {
        state.handle_event(AppEvent::Key(key), &tx).await;
        print_frame(&state, area, &format!("after {key:?}"));
    }
}

/// `140x40`, as `--size` takes it.
fn parse_size(raw: &str) -> (u16, u16) {
    let (w, h) = raw.split_once('x').unwrap_or_else(|| {
        panic!("--size wants WIDTHxHEIGHT, got {raw}");
    });
    (
        w.parse().expect("a numeric width"),
        h.parse().expect("a numeric height"),
    )
}

/// A key by name, for `--keys`.
fn parse_key(name: &str) -> KeyEvent {
    let code = match name {
        "tab" => KeyCode::Tab,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => panic!("unknown key {other}"),
            }
        }
    };
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn print_frame(state: &AppState, area: Rect, label: &str) {
    let ui = state.ui_state();
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| render(frame, &ui, &Theme::default()))
        .unwrap();

    let buffer: &Buffer = terminal.backend().buffer();
    println!("┄┄ {label} · {}×{} ┄┄", area.width, area.height);
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

/// Three lines around the point this preview is paused at (1:37).
///
/// Spaced out so the window is doing something visible: the line being sung is
/// the middle one, its neighbours are the rows either side of it.
const SAMPLE_LRC: &str = "[00:30.00]故事的小黄花\n\
                           [01:36.00]从出生那年就飘着\n\
                           [01:52.00]童年的荡秋千";

/// The picture for the track: its cover if one can be had, and the default sign
/// if not.
async fn artwork(area: Rect, cover_size: CoverSize, url: Option<&str>) -> niveus_ui::ArtworkView {
    let region = niveus_ui::compute(area, LayoutTier::Standard, cover_size)
        .artwork
        .expect("standard tier has room for art");
    let size = Size::new(region.width, region.height);
    let options = Options::default();

    let Some(url) = url else {
        return niveus_app::artwork::view(&niveus_artwork::render_default(size));
    };

    let key = CoverKey::new(url, size, options);
    let frame = match load(&key).await {
        Ok(frame) if !frame.is_empty() => frame,
        Ok(_) => niveus_artwork::render_default(size),
        Err(error) => {
            eprintln!("无法加载封面（{error}），改用默认封面");
            niveus_artwork::render_default(size)
        }
    };
    niveus_app::artwork::view(&frame)
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
            cover_url: Some(format!("https://example.invalid/{index}.jpg")),
        }),
        duration_ms: Some(duration),
    })
    .collect()
}
