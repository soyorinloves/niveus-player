//! The current-track pane.
//!
//! Three sections stacked and separated by rules: song identity, the lyrics or
//! metadata panel, and the progress bar. Nothing here is boxed.

use super::{
    display_width, format_duration, horizontal_rule, line_of, split_line, truncate, UNKNOWN,
};
use crate::layout::{pane_rows, panel_blocks, Regions};
use crate::state::{ArtworkInk, ArtworkView, LyricWindow, LyricsView, NowPlayingView, UiState};
use crate::theme::Theme;
use niveus_config::Action;
use niveus_music_core::Song;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// The knob that marks the play position.
const KNOB: &str = "●";
/// The track the knob runs along.
const TRACK: &str = "─";

pub(crate) fn render(frame: &mut Frame, regions: &Regions, state: &UiState, theme: &Theme) {
    let Some(area) = regions.now_playing else {
        return;
    };
    if area.width == 0 || area.height < 6 {
        return;
    }

    // The rows come from the layout module, which is also where the app asks
    // how tall the panel is: drawing from one description and sizing a cover or
    // a lyric window from another is how the two drift apart.
    let rows = pane_rows(area, state.cover_size);

    frame.render_widget(
        Paragraph::new(line_of("NOW PLAYING", theme.heading)),
        rows.title,
    );
    horizontal_rule(frame, rows.title_rule, theme);

    // Until a picture is ready the region stays empty rather than showing a
    // placeholder.
    if let (Some(band), Some(artwork)) = (rows.artwork, state.now_playing.artwork) {
        draw_artwork(frame.buffer_mut(), band, artwork, theme);
    }

    render_identity(frame, rows.identity, state.now_playing.song, theme);
    horizontal_rule(frame, rows.identity_rule, theme);
    render_section(frame, rows.section, state, theme);
    render_panel(frame, rows.panel, state, theme);
    horizontal_rule(frame, rows.panel_rule, theme);
    render_progress(frame, rows.progress, state, theme);
}

fn render_identity(frame: &mut Frame, area: Rect, song: Option<&Song>, theme: &Theme) {
    let (title, artist, album) = match song {
        Some(song) => (
            song.title.as_str(),
            song.artist_names(),
            song.album_name().to_string(),
        ),
        None => (UNKNOWN, UNKNOWN.to_string(), UNKNOWN.to_string()),
    };
    // Shortened to the pane like any other row: a long Chinese title would
    // otherwise be cut mid-thought at the edge.
    let width = area.width as usize;
    let lines = vec![
        line_of(truncate(title, width), theme.emphasis),
        line_of(truncate(&artist, width), theme.normal),
        line_of(truncate(&album, width), theme.muted),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

/// Puts a picture in the middle of the art region.
///
/// Written straight into the buffer rather than through a widget: a picture is
/// a grid of cells with a colour in each half, which is exactly what the buffer
/// holds, and any widget would have to take it apart and put it back together.
fn draw_artwork(buffer: &mut Buffer, area: Rect, artwork: &ArtworkView, theme: &Theme) {
    if artwork.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }

    let left = area.x + area.width.saturating_sub(artwork.width()) / 2;
    let top = area.y + area.height.saturating_sub(artwork.height()) / 2;

    for y in 0..artwork.height() {
        for x in 0..artwork.width() {
            let Some(source) = artwork.cell(x, y) else {
                continue;
            };
            let (px, py) = (left + x, top + y);
            if px >= area.x + area.width || py >= area.y + area.height {
                continue;
            }
            let Some(target) = buffer.cell_mut((px, py)) else {
                continue;
            };
            target.set_symbol(&source.symbol.to_string());
            target.set_fg(ink_colour(source.fg, theme));
            target.set_bg(ink_colour(source.bg, theme));
        }
    }
}

/// The terminal colour an ink asks for.
fn ink_colour(ink: ArtworkInk, theme: &Theme) -> Color {
    match ink {
        ArtworkInk::Rgb(r, g, b) => Color::Rgb(r, g, b),
        ArtworkInk::Indexed(index) => Color::Indexed(index),
        ArtworkInk::Classic(index) => classic_colour(index),
        // The default cover has no colours of its own: it is drawn in the same
        // accent as the progress bar and the current row.
        ArtworkInk::Accent => theme.accent.fg.unwrap_or(Color::Reset),
        ArtworkInk::None => Color::Reset,
    }
}

/// The sixteen classic colours, by index.
///
/// These are named rather than numbered on purpose: the numbered form is the
/// palette's first sixteen entries, which is a different thing — those are
/// whatever the user's theme has put there.
fn classic_colour(index: u8) -> Color {
    const CLASSIC: [Color; 16] = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    CLASSIC[index as usize % CLASSIC.len()]
}

/// The row naming what the panel below is.
///
/// `LYRICS` is a caption rather than a control — there is nothing to press to
/// "show lyrics", because turning the metadata off is what shows them
/// (SPEC §12.1). `METADATA [M]` is the switch, and it is drawn at full weight
/// while it is on.
fn render_section(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let metadata = Span::styled(
        format!("METADATA {}", state.hint(Action::ToggleMetadata)),
        if state.now_playing.metadata {
            theme.emphasis
        } else {
            theme.muted
        },
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("LYRICS", theme.muted),
            Span::raw("    "),
            metadata,
        ])),
        area,
    );
}

/// Draws the panel: the lyrics, and the metadata block under them when it is on.
///
/// The block is pinned to the bottom of the panel and the lyrics take what is
/// left, centred in it. With the largest cover the block is the whole panel,
/// which is why turning it on there looks like a switch to another view — the
/// same thing the key does in every preset (SPEC §12.1).
fn render_panel(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let blocks = panel_blocks(area, state.now_playing.metadata);
    if let Some(rule) = blocks.rule {
        // The same rule as the pane's other sections, because it is doing the
        // same job (SPEC §10 / §36 第 9 条).
        horizontal_rule(frame, rule, theme);
    }
    if let Some(block) = blocks.metadata {
        frame.render_widget(
            Paragraph::new(metadata_lines(&state.now_playing, area.width, theme)),
            block,
        );
    }
    let lyrics = blocks.lyrics;

    let lines = lyrics_lines(&state.now_playing.lyrics, area.width, theme);
    if lines.is_empty() || lyrics.height == 0 {
        return;
    }
    // Centred rather than pinned to the top: the window is normally shorter
    // than the room it has, and the rows left over read as the gap between the
    // lyrics and the block below (SPEC §12.1).
    let rows = lines.len() as u16;
    let block = Rect {
        y: lyrics.y + lyrics.height.saturating_sub(rows) / 2,
        height: rows.min(lyrics.height),
        ..lyrics
    };
    frame.render_widget(Paragraph::new(lines), block);
}

/// The lyric panel: the window of lines, or the one line saying why there is
/// nothing in it.
fn lyrics_lines(lyrics: &LyricsView<'_>, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    match lyrics {
        // Nothing is known yet. An empty panel is the only truthful thing to
        // draw: anything else would claim to know how the song ends.
        LyricsView::Pending => Vec::new(),
        LyricsView::Window(window) => window_lines(window, width, theme),
        // A notice is a whole line of the panel too, so it sits on the same
        // centre line the lyrics do (SPEC §13.3).
        LyricsView::TranslationOnly => vec![centred("暂无原文歌词", theme.muted, width)],
        LyricsView::Instrumental => vec![centred("♪ 纯音乐", theme.muted, width)],
        // The one failure the panel reports. What went wrong is in the log; the
        // panel says only that the lyrics are not usable (SPEC §13.4, §33).
        LyricsView::Failed => vec![centred("歌词加载失败", theme.error, width)],
    }
}

/// A window drawn: a line every other row, centred across the width, with the
/// line being sung at full weight (SPEC §13.1, §13.3).
fn window_lines(window: &LyricWindow<'_>, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(window.rows());
    for slot in 0..window.line_count() {
        let style = if window.is_current(slot) {
            theme.accent
        } else {
            theme.muted
        };
        lines.push(centred(window.line(slot).unwrap_or(""), style, width));
        // The blank row between this line and the next one. It is part of the
        // window, not spacing around it: the line being sung is in the middle
        // *because* every line has a row of its own and a row after it.
        if slot + 1 < window.line_count() {
            lines.push(Line::from(""));
        }
    }
    lines
}

/// One line of lyrics, centred across the region.
///
/// Measured in columns, like everything else that is laid out: half a line's
/// width in characters would centre Chinese text off to one side, and the error
/// is invisible in an English song.
fn centred(text: &str, style: Style, width: u16) -> Line<'static> {
    let text = truncate(text, width as usize);
    let pad = width as usize - display_width(&text);
    Line::from(vec![
        Span::raw(" ".repeat(pad / 2)),
        Span::styled(text, style),
    ])
}

/// The metadata panel: one row per field, value against the trailing edge.
///
/// An absent value is a dash rather than an empty space — the row still says
/// what the field is, so the panel reads the same whether or not the service
/// had anything to put in it.
fn metadata_lines(state: &NowPlayingView<'_>, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let source = state.source;
    let field = |label: &str, value: Option<String>| {
        metadata_line(
            label,
            value.unwrap_or_else(|| UNKNOWN.to_string()),
            width,
            theme,
        )
    };

    vec![
        field(
            "ALBUM",
            state
                .song
                .and_then(|song| song.album.as_ref())
                .map(|album| album.name.clone()),
        ),
        // The player's own figure wins, exactly as it does on the progress row
        // below: it is what the file turned out to be, not what was promised.
        metadata_line("DURATION", format_duration(state.duration_ms), width, theme),
        field(
            "QUALITY",
            source
                .and_then(|s| s.level)
                .map(|level| level.label().to_string()),
        ),
        field("FORMAT", source.and_then(|s| s.format.clone())),
        // Five fields for the five rows the panel reserves (SPEC §14).
        field("SONG ID", state.song.map(|song| song.id.get().to_string())),
    ]
}

fn metadata_line(label: &str, value: String, width: u16, theme: &Theme) -> Line<'static> {
    // The label is short and fixed and the value is what gives way when the two
    // cannot both fit — an album title can run to any length, and `split_line`
    // drops the left side entirely when the right will not fit beside it. For a
    // field whose whole job is to say which field it is, that is the wrong half
    // to lose.
    let room = (width as usize).saturating_sub(display_width(label) + 1);
    let value = truncate(&value, room);

    // The label is the caption and the value is the information, which is the
    // opposite way round from the settings list, where the value is a setting's
    // current state rather than the thing being read.
    split_line(
        vec![Span::styled(label.to_string(), theme.muted)],
        vec![Span::styled(value, theme.normal)],
        width,
    )
}

/// The progress bar: elapsed time, a track with the knob on it, total time.
///
/// The track is sized to fill exactly what the two labels leave, rather than to
/// a fixed budget, so the bar reaches the edge of the pane instead of trailing
/// off with a gap.
fn render_progress(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let elapsed = format_duration(state.now_playing.position_ms);
    let total = format_duration(state.now_playing.duration_ms);
    // Everything except the track itself: the two labels and the spaces that
    // separate them from it. The knob is not counted, because it takes the
    // place of one of the track's own characters rather than sitting beside it.
    let reserved = display_width(&elapsed) + display_width(&total) + 2;
    let track = (area.width as usize).saturating_sub(reserved).max(1);

    let line = Line::from(parts(
        &elapsed,
        &total,
        track,
        progress_ratio(state.now_playing.position_ms, state.now_playing.duration_ms),
        theme,
    ));

    frame.render_widget(Paragraph::new(line), area);
}

/// How far along the track the knob sits, 0.0 to 1.0.
///
/// `None` when there is no length to measure against, which puts the knob at
/// the start — that reads as "not started" rather than as an arbitrary
/// position, which is the truthful thing to show.
fn progress_ratio(position: Option<u64>, duration: Option<u64>) -> Option<f64> {
    let (position, duration) = (position?, duration?);
    if duration == 0 {
        return None;
    }
    Some((position as f64 / duration as f64).clamp(0.0, 1.0))
}

fn parts(
    elapsed: &str,
    total: &str,
    track: usize,
    ratio: Option<f64>,
    theme: &Theme,
) -> Vec<Span<'static>> {
    // The knob replaces one of the track's own cells, so the row is exactly as
    // wide as it was budgeted for.
    let knob_at = match ratio {
        Some(ratio) => (ratio * (track.saturating_sub(1)) as f64).round() as usize,
        None => 0,
    };
    let knob_at = knob_at.min(track.saturating_sub(1));

    vec![
        Span::styled(elapsed.to_string(), theme.muted),
        Span::raw(" "),
        Span::styled(TRACK.repeat(knob_at), theme.muted),
        Span::styled(KNOB, theme.accent),
        Span::styled(TRACK.repeat(track - knob_at - 1), theme.muted),
        Span::raw(" "),
        Span::styled(total.to_string(), theme.muted),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{compute, LayoutTier, METADATA_ROWS};
    use crate::render::render;
    use crate::state::{
        ListView, LyricWindow, LyricsView, NowPlayingView, Page, SessionView, StatusView, UiState,
    };
    use crate::testing::{row_text, row_width};
    use niveus_config::{CoverSize, Keymap};
    use niveus_music_core::{
        Album, AlbumId, Artist, ArtistId, AudioQuality, AudioSource, PlaybackMode, PlaybackStatus,
        Song, SongId, User, UserId,
    };
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Modifier;
    use ratatui::Terminal;

    /// The terminal every test draws into.
    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 40,
    };

    fn regions(cover_size: CoverSize) -> crate::layout::Regions {
        compute(AREA, LayoutTier::Standard, cover_size)
    }

    fn pane_area() -> Rect {
        pane_area_for(CoverSize::Normal)
    }

    fn pane_area_for(cover_size: CoverSize) -> Rect {
        regions(cover_size)
            .now_playing
            .expect("standard tier shows the pane")
    }

    fn art_area() -> Rect {
        compute(
            Rect::new(0, 0, 120, 40),
            LayoutTier::Standard,
            CoverSize::Normal,
        )
        .artwork
        .expect("standard tier has room for art")
    }

    struct Showing<'a> {
        song: Option<&'a Song>,
        metadata: bool,
        artwork: Option<&'a ArtworkView>,
        position: Option<u64>,
        duration: Option<u64>,
        lyrics: LyricsView<'a>,
        source: Option<&'a AudioSource>,
        cover_size: CoverSize,
    }

    impl Default for Showing<'_> {
        fn default() -> Self {
            Self {
                song: None,
                metadata: false,
                artwork: None,
                position: None,
                duration: None,
                lyrics: LyricsView::Pending,
                source: None,
                cover_size: CoverSize::Normal,
            }
        }
    }

    fn draw(showing: Showing<'_>) -> Buffer {
        let keymap = Keymap::default();
        let user = User {
            id: UserId(1),
            name: "n".into(),
        };
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Ready { user: &user },
            list: ListView::Message(""),
            now_playing: NowPlayingView {
                song: showing.song,
                metadata: showing.metadata,
                position_ms: showing.position,
                duration_ms: showing.duration,
                artwork: showing.artwork,
                lyrics: showing.lyrics,
                source: showing.source,
            },
            status: StatusView {
                loading: false,
                notice: None,
                volume: 72,
                playback_mode: PlaybackMode::Sequential,
                queue_len: 0,
                playback: PlaybackStatus::Stopped,
                // The status bar has its own tests; this harness is about the pane.
                bitrate: None,
            },
            keymap: &keymap,
            tier: LayoutTier::Standard,
            cover_size: showing.cover_size,
        };
        let mut terminal = Terminal::new(TestBackend::new(AREA.width, AREA.height)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &crate::theme::Theme::default()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn text_of(buffer: &Buffer, area: Rect) -> String {
        (area.y..area.y + area.height)
            .map(|y| {
                (area.x..area.x + area.width)
                    .map(|x| buffer.cell((x, y)).unwrap().symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    static SONG: std::sync::LazyLock<Song> = std::sync::LazyLock::new(|| Song {
        id: SongId(1),
        title: "Sunny Day".into(),
        artists: vec![Artist {
            id: ArtistId(2),
            name: "Jay".into(),
        }],
        album: Some(Album {
            id: AlbumId(3),
            name: "Ye Huimei".into(),
            cover_url: None,
        }),
        duration_ms: Some(269_000),
    });

    #[test]
    fn the_pane_states_its_purpose_and_what_the_panel_can_show() {
        let buffer = draw(Showing::default());
        let text = text_of(&buffer, pane_area());

        assert!(text.contains("NOW PLAYING"), "{text}");
        // Lyrics are a caption, not a key: turning the metadata off is what
        // shows them (SPEC §12.1).
        assert!(text.contains("LYRICS"), "{text}");
        assert!(!text.contains("LYRICS ["), "{text}");
        assert!(text.contains("METADATA [M]"), "{text}");
    }

    #[test]
    fn missing_values_are_dashed_rather_than_blank() {
        let buffer = draw(Showing::default());
        let text = text_of(&buffer, pane_area());
        assert!(text.matches('—').count() >= 3, "{text}");
    }

    #[test]
    fn a_track_supplies_title_artist_and_album() {
        let buffer = draw(Showing {
            song: Some(&SONG),
            ..Showing::default()
        });
        let text = text_of(&buffer, pane_area());
        assert!(text.contains("Sunny Day"), "{text}");
        assert!(text.contains("Jay"), "{text}");
        assert!(text.contains("Ye Huimei"), "{text}");
    }

    /// A track whose title runs well past the pane.
    static LONG_TITLE_SONG: std::sync::LazyLock<Song> = std::sync::LazyLock::new(|| Song {
        id: SongId(2),
        title: "这是一首名字特别特别长的中文歌曲标题专门用来测试截断行为是否按显示列宽正确工作"
            .into(),
        artists: vec![Artist {
            id: ArtistId(7),
            name: "演唱者名字也很长的歌手名字同样很长的歌手".into(),
        }],
        album: Some(Album {
            id: AlbumId(8),
            name: "专辑名字同样很长的专辑名字同样很长".into(),
            cover_url: None,
        }),
        duration_ms: Some(261_000),
    });

    #[test]
    fn every_row_of_the_pane_stays_inside_it() {
        for song in [None, Some(&*SONG), Some(&*LONG_TITLE_SONG)] {
            let buffer = draw(Showing {
                song,
                ..Showing::default()
            });
            let area = pane_area();
            for y in 0..area.height {
                assert_eq!(
                    row_width(&buffer, area, y),
                    area.width as usize,
                    "row {y} does not span the pane"
                );
            }
        }
    }

    #[test]
    fn a_long_chinese_title_is_shortened_rather_than_cut() {
        let buffer = draw(Showing {
            song: Some(&*LONG_TITLE_SONG),
            ..Showing::default()
        });
        let area = pane_area();

        // Find the identity row by its content rather than by a fixed offset,
        // so the test does not also depend on how much room the artwork takes.
        let title_row = (0..area.height)
            .map(|offset| row_text(&buffer, area, area.y + offset))
            .find(|row| row.trim_end().starts_with("这是一首"))
            .expect("the title row is missing");

        assert!(
            title_row.contains('…'),
            "not marked as shortened: {title_row:?}"
        );
    }

    #[test]
    fn the_progress_row_fills_its_pane() {
        let buffer = draw(Showing::default());
        let area = pane_area();
        let y = area.y + area.height - 1;
        let row = row_text(&buffer, area, y);

        assert_eq!(row_width(&buffer, area, y), area.width as usize);
        // The total time sits against the right edge.
        assert!(row.ends_with('—'), "{row:?}");
        assert!(row.contains('━') || row.contains('─'), "{row:?}");
    }

    #[test]
    fn the_sections_are_separated_by_full_width_rules() {
        let buffer = draw(Showing::default());
        let area = pane_area();
        let rule_rows: Vec<u16> = (area.y..area.y + area.height)
            .filter(|y| {
                let row: String = (area.x..area.x + area.width)
                    .map(|x| buffer.cell((x, *y)).unwrap().symbol())
                    .collect();
                !row.is_empty() && row.chars().all(|c| c == '─')
            })
            .collect();
        assert!(
            rule_rows.len() >= 3,
            "expected three section rules, found {}",
            rule_rows.len()
        );
    }

    // ---- the panel -------------------------------------------------------

    /// A window with a line on either side of the one being sung.
    fn window(slots: &[Option<&'static str>]) -> LyricsView<'static> {
        LyricsView::Window(LyricWindow::new(slots.to_vec()))
    }

    fn three_lines() -> LyricsView<'static> {
        window(&[Some("上一句歌词"), Some("正在唱的这句"), Some("下一句歌词")])
    }

    /// The row of the pane a piece of text is drawn on.
    fn row_of(buffer: &Buffer, needle: &str) -> u16 {
        pane_rows(buffer)
            .into_iter()
            .find(|(_, row)| row.contains(needle))
            .map(|(y, _)| y)
            .unwrap_or_else(|| panic!("{needle:?} is not on screen:\n{}", pane_text(buffer)))
    }

    /// The column, within the pane, that a piece of text starts at.
    fn column_of(buffer: &Buffer, needle: &str) -> usize {
        let (_, row) = pane_rows(buffer)
            .into_iter()
            .find(|(_, row)| row.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} is not on screen:\n{}", pane_text(buffer)));
        let byte = row.find(needle).expect("just found it");
        // The prefix may itself hold Chinese, so it is measured in columns.
        display_width(&row[..byte])
    }

    /// The cell the first character of `needle` is drawn in.
    fn cell_of(buffer: &Buffer, needle: &str) -> ratatui::buffer::Cell {
        let area = pane_area();
        let y = row_of(buffer, needle);
        buffer
            .cell((area.x + column_of(buffer, needle) as u16, y))
            .unwrap()
            .clone()
    }

    fn pane_rows(buffer: &Buffer) -> Vec<(u16, String)> {
        let area = pane_area();
        (area.y..area.y + area.height)
            .map(|y| (y, row_text(buffer, area, y)))
            .collect()
    }

    fn pane_text(buffer: &Buffer) -> String {
        pane_rows(buffer)
            .into_iter()
            .map(|(_, row)| row)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The panel's own region, as the renderer divides it.
    fn panel_area() -> Rect {
        panel_area_for(CoverSize::Normal)
    }

    fn panel_area_for(cover_size: CoverSize) -> Rect {
        regions(cover_size)
            .panel
            .expect("standard tier has a panel")
    }

    fn showing_lyrics(lyrics: LyricsView<'static>) -> Showing<'static> {
        Showing {
            song: Some(&SONG),
            lyrics,
            ..Showing::default()
        }
    }

    // ---- the window ------------------------------------------------------

    #[test]
    fn a_line_every_other_row_puts_the_line_being_sung_in_the_middle() {
        // Three lines take five rows: the one being sung is the third of them,
        // and the rows between are blank. A three-row window could not do this —
        // its middle row would be a blank one.
        let buffer = draw(showing_lyrics(three_lines()));
        let previous = row_of(&buffer, "上一句歌词");
        let current = row_of(&buffer, "正在唱的这句");
        let next = row_of(&buffer, "下一句歌词");

        assert_eq!(current, previous + 2, "the lines are not two rows apart");
        assert_eq!(next, current + 2, "the lines are not two rows apart");
        assert!(row_text(&buffer, pane_area(), previous + 1)
            .trim()
            .is_empty());
        assert!(row_text(&buffer, pane_area(), current + 1)
            .trim()
            .is_empty());
        // Equal room above and below, which is what "in the middle" means.
        assert_eq!(current - previous, next - current);
    }

    #[test]
    fn a_missing_neighbour_leaves_its_row_empty_rather_than_moving_the_others() {
        // The song has just started: there is nothing before this line. The
        // rows must not shuffle up to fill the gap, or the whole panel would
        // jump as the song advances.
        let with_both = draw(showing_lyrics(three_lines()));
        let buffer = draw(showing_lyrics(window(&[None, Some("正在唱的这句"), None])));

        assert_eq!(
            row_of(&buffer, "正在唱的这句"),
            row_of(&with_both, "正在唱的这句"),
            "the middle row moved when its neighbours went missing"
        );
    }

    #[test]
    fn the_line_being_sung_is_the_one_in_the_accent_colour() {
        let buffer = draw(showing_lyrics(three_lines()));

        let accent = Theme::default()
            .accent
            .fg
            .expect("the theme accents in a colour");
        assert_eq!(
            cell_of(&buffer, "正在唱的这句").fg,
            accent,
            "the line being sung is not drawn in the accent colour"
        );
        for neighbour in ["上一句歌词", "下一句歌词"] {
            assert_ne!(
                cell_of(&buffer, neighbour).fg,
                accent,
                "{neighbour} is drawn at the same weight as the line being sung"
            );
        }
    }

    #[test]
    fn every_lyric_is_centred_across_the_pane() {
        let buffer = draw(showing_lyrics(window(&[
            Some("a short line"),
            Some("正在唱的这句"),
            Some("another line"),
        ])));
        let width = pane_area().width as usize;

        for line in ["a short line", "正在唱的这句", "another line"] {
            let expected = (width - display_width(line)) / 2;
            assert_eq!(
                column_of(&buffer, line),
                expected,
                "{line:?} is not centred"
            );
        }
        // The whole point: counting characters would have put the Chinese line
        // three columns further right, and nothing on an English screen shows it.
        assert_ne!(
            column_of(&buffer, "正在唱的这句"),
            (width - "正在唱的这句".chars().count()) / 2
        );
    }

    #[test]
    fn an_odd_column_of_slack_goes_to_the_right_so_the_line_never_leans_left() {
        // A width that does not halve evenly has one column left over, and
        // which side it falls on has to be decided rather than left to chance:
        // it goes on the right, so a centred line is never pulled leftwards.
        let line = "正在唱的这句";
        let buffer = draw(showing_lyrics(window(&[None, Some(line), None])));
        let width = panel_area().width as usize;

        assert_eq!(column_of(&buffer, line), (width - display_width(line)) / 2);
        assert!(
            width > display_width(line),
            "the pane is too narrow to centre in"
        );
    }

    #[test]
    fn a_window_hangs_in_the_middle_of_the_room_it_has() {
        // The panel is taller than a five-row window, and the leftover rows are
        // split above and below rather than all falling under it.
        let buffer = draw(showing_lyrics(three_lines()));
        let area = panel_area();
        let top = row_of(&buffer, "上一句歌词");
        let bottom = row_of(&buffer, "下一句歌词");

        let above = top - area.y;
        let below = (area.y + area.height) - 1 - bottom;
        assert!(
            above.abs_diff(below) <= 1,
            "{above} rows above the window and {below} below it"
        );
    }

    #[test]
    fn a_long_chinese_lyric_is_shortened_to_the_pane_rather_than_cut() {
        let long = "这一句歌词非常非常长需要按终端显示列宽截断而不能溢出到旁边去";
        let buffer = draw(showing_lyrics(window(&[None, Some(long), None])));
        let area = pane_area();
        let y = row_of(&buffer, "这一句歌词");

        let row = row_text(&buffer, area, y);
        assert!(row.contains('…'), "not marked as shortened: {row:?}");
        assert_eq!(row_width(&buffer, area, y), area.width as usize);
        // Whole characters only: half a glyph would leave a stray column.
        assert_eq!(display_width(row.trim_end_matches('…')) % 2, 0, "{row:?}");
    }

    #[test]
    fn the_panel_tells_the_three_kinds_of_nothing_apart() {
        // Each of these is a different thing to say, and none of them is the
        // same as the panel merely being empty.
        let cases: [(LyricsView<'static>, &str); 3] = [
            (LyricsView::Instrumental, "♪ 纯音乐"),
            (LyricsView::TranslationOnly, "暂无原文歌词"),
            (LyricsView::Failed, "歌词加载失败"),
        ];
        for (lyrics, expected) in cases.iter() {
            let seen = pane_text(&draw(showing_lyrics(lyrics.clone())));
            assert!(
                seen.contains(expected),
                "{expected:?} is missing from:\n{seen}"
            );
            for (other, _) in cases.iter().filter(|(case, _)| case != lyrics) {
                if let LyricsView::Window(_) = other {
                    continue;
                }
                let _ = other;
            }
        }
        // And each message appears only for its own state.
        for (lyrics, expected) in cases.iter() {
            let seen = pane_text(&draw(showing_lyrics(lyrics.clone())));
            for (_, other) in cases.iter().filter(|(_, other)| other != expected) {
                assert!(!seen.contains(other), "{seen}");
            }
        }
    }

    #[test]
    fn a_notice_is_centred_on_the_same_line_the_lyrics_use() {
        // The notices are whole lines of the panel, so they sit on the same
        // centre line as a lyric would (SPEC §13.3).
        let width = panel_area().width as usize;
        for (lyrics, notice) in [
            (LyricsView::Instrumental, "♪ 纯音乐"),
            (LyricsView::TranslationOnly, "暂无原文歌词"),
            (LyricsView::Failed, "歌词加载失败"),
        ] {
            let buffer = draw(showing_lyrics(lyrics));
            assert_eq!(
                column_of(&buffer, notice),
                (width - display_width(notice)) / 2,
                "{notice:?} is not centred"
            );
        }
    }

    #[test]
    fn lyrics_still_on_their_way_are_not_claimed_to_be_anything() {
        // Saying a track is instrumental before the answer arrives would be a
        // guess, and a wrong one on most tracks.
        let seen = pane_text(&draw(showing_lyrics(LyricsView::Pending)));
        for claim in ["♪ 纯音乐", "暂无原文歌词", "歌词加载失败"] {
            assert!(!seen.contains(claim), "{seen}");
        }
    }

    #[test]
    fn a_lyric_failure_is_drawn_as_a_failure() {
        // The one state that is an error says so in the colour the theme keeps
        // for failures; the two ordinary ones do not.
        let failed = draw(showing_lyrics(LyricsView::Failed));
        assert_eq!(cell_of(&failed, "歌词加载失败").fg, Color::Red);
    }

    // ---- the section labels ----------------------------------------------

    #[test]
    fn the_panel_names_itself_and_offers_the_metadata_switch() {
        let text = pane_text(&draw(showing_lyrics(three_lines())));
        assert!(text.contains("LYRICS"), "{text}");
        assert!(text.contains("METADATA [M]"), "{text}");
    }

    #[test]
    fn the_metadata_label_is_highlighted_while_the_block_is_showing() {
        // It is a switch, so the row says which way it is thrown.
        let off = draw(Showing {
            song: Some(&SONG),
            ..Showing::default()
        });
        let on = draw(Showing {
            song: Some(&SONG),
            metadata: true,
            ..Showing::default()
        });

        // Full weight while it is on, de-emphasised while it is off.
        let label = "METADATA";
        assert!(
            cell_of(&on, label).modifier.contains(Modifier::BOLD),
            "the switch is not highlighted while it is on"
        );
        assert!(
            !cell_of(&off, label).modifier.contains(Modifier::BOLD),
            "the switch looks thrown when it is not"
        );
    }

    // ---- metadata --------------------------------------------------------

    fn lossless() -> AudioSource {
        AudioSource {
            url: "https://example.invalid/song.flac".into(),
            level: Some(AudioQuality::Lossless),
            format: Some("flac".into()),
            bitrate: Some(999_000),
            is_trial: false,
        }
    }

    /// A track playing with the metadata block showing.
    fn playing(source: &AudioSource) -> Showing<'_> {
        Showing {
            song: Some(&SONG),
            metadata: true,
            source: Some(source),
            duration: Some(181_000),
            ..Showing::default()
        }
    }

    #[test]
    fn the_metadata_block_names_all_five_fields_and_their_values() {
        let source = AudioSource {
            url: "https://example.invalid/song.mp3".into(),
            level: Some(AudioQuality::Exhigh),
            format: Some("mp3".into()),
            bitrate: Some(320_000),
            is_trial: false,
        };
        let text = pane_text(&draw(playing(&source)));

        for label in ["ALBUM", "DURATION", "QUALITY", "FORMAT", "SONG ID"] {
            assert!(text.contains(label), "{label} is missing from:\n{text}");
        }
        assert!(text.contains("Ye Huimei"), "{text}");
        assert!(
            text.contains("03:01"),
            "the player's own duration is missing:\n{text}"
        );
        assert!(text.contains("exhigh"), "{text}");
        assert!(text.contains("mp3"), "{text}");
        assert!(
            text.contains(&SONG.id.get().to_string()),
            "the song id is missing:\n{text}"
        );
    }

    #[test]
    fn the_block_takes_the_bottom_five_rows_of_the_panel() {
        let buffer = draw(playing(&lossless()));
        let area = panel_area();

        let top = row_of(&buffer, "ALBUM");
        assert_eq!(
            top,
            area.y + area.height - METADATA_ROWS,
            "the block is not at the bottom"
        );
        assert_eq!(row_of(&buffer, "SONG ID"), area.y + area.height - 1);
    }

    #[test]
    fn the_lyrics_keep_the_room_above_the_block_and_stay_centred_in_it() {
        let buffer = draw(Showing {
            song: Some(&SONG),
            metadata: true,
            lyrics: three_lines(),
            ..Showing::default()
        });
        let area = panel_area();
        let lyrics = panel_blocks(area, true).lyrics;

        // Every lyric row is above the block, and inside the room left for it.
        for line in ["上一句歌词", "正在唱的这句", "下一句歌词"] {
            let y = row_of(&buffer, line);
            assert!(
                y < area.y + area.height - METADATA_ROWS,
                "{line} is under the block"
            );
            assert!(
                y >= lyrics.y && y < lyrics.y + lyrics.height,
                "{line} is outside the lyric area"
            );
        }

        let top = row_of(&buffer, "上一句歌词");
        let bottom = row_of(&buffer, "下一句歌词");
        let above = top - lyrics.y;
        let below = (lyrics.y + lyrics.height) - 1 - bottom;
        assert!(above.abs_diff(below) <= 1, "{above} above, {below} below");
    }

    #[test]
    fn a_rule_separates_the_lyrics_from_the_block() {
        let buffer = draw(Showing {
            song: Some(&SONG),
            metadata: true,
            lyrics: three_lines(),
            ..Showing::default()
        });
        let block = panel_blocks(panel_area(), true).metadata.expect("showing");
        let rule = block.y - 1;
        let row = row_text(&buffer, pane_area(), rule);

        // The same character and colour as every other section rule, and it
        // spans the panel.
        assert!(row.chars().all(|c| c == '─'), "{row:?}");
        assert_eq!(
            row_width(&buffer, pane_area(), rule),
            pane_area().width as usize
        );
        let first = buffer
            .cell((pane_area().x, rule))
            .expect("a cell on the rule");
        assert_eq!(first.symbol(), "─");
        assert_eq!(
            first.fg,
            Theme::default().divider.fg.unwrap_or(Color::Reset),
            "the rule is not drawn in the divider colour"
        );
        // The lyrics are above it, the block below.
        assert!(row_of(&buffer, "下一句歌词") < rule);
        assert!(row_of(&buffer, "ALBUM") > rule);
    }

    #[test]
    fn no_rule_is_drawn_when_the_block_has_the_panel_to_itself() {
        // The largest cover leaves exactly five rows, so the block fills the
        // panel and there is nothing above it to separate (SPEC §12.1).
        let buffer = draw(Showing {
            song: Some(&SONG),
            metadata: true,
            cover_size: CoverSize::Large,
            lyrics: three_lines(),
            ..Showing::default()
        });
        let panel = panel_area_for(CoverSize::Large);
        assert_eq!(
            panel.height, METADATA_ROWS,
            "this test is about the short panel"
        );

        // Every row of the panel belongs to the block: five fields, no rule.
        let rows: Vec<String> = (panel.y..panel.y + panel.height)
            .map(|y| row_text(&buffer, panel, y))
            .collect();
        assert!(rows[0].contains("ALBUM"), "{:?}", rows[0]);
        assert!(
            !rows
                .iter()
                .any(|row| row.chars().all(|c| c == '─' || c == ' ')),
            "a rule was drawn inside the block: {rows:?}"
        );
    }

    #[test]
    fn nothing_is_claimed_about_the_metadata_while_the_switch_is_off() {
        let buffer = draw(Showing {
            song: Some(&SONG),
            lyrics: three_lines(),
            ..Showing::default()
        });
        let text = pane_text(&buffer);
        for label in ["ALBUM", "DURATION", "QUALITY", "FORMAT", "SONG ID"] {
            assert!(
                !text.contains(label),
                "{label} is shown with the block off:\n{text}"
            );
        }
        // The lyrics take the whole panel: nothing of them is pushed out.
        for line in ["上一句歌词", "正在唱的这句", "下一句歌词"] {
            assert!(text.contains(line), "{line} is missing:\n{text}");
        }
    }

    #[test]
    fn a_metadata_field_the_service_did_not_report_is_a_dash() {
        // Nothing playing and nothing reported: every field still says which
        // field it is, and says it does not know.
        let buffer = draw(Showing {
            metadata: true,
            ..Showing::default()
        });
        let text = pane_text(&buffer);

        for label in ["ALBUM", "DURATION", "QUALITY", "FORMAT", "SONG ID"] {
            let row = pane_rows(&buffer)
                .into_iter()
                .find(|(_, row)| row.contains(label))
                .map(|(_, row)| row)
                .unwrap_or_else(|| panic!("{label} is missing from:\n{text}"));
            assert!(
                row.contains('—'),
                "{label} does not say it does not know: {row:?}"
            );
        }
    }

    #[test]
    fn a_long_chinese_album_keeps_its_label_visible() {
        // An album title can be longer than the pane; the label is short and
        // fixed, and it is the one that has to survive the shortening.
        static LONG_ALBUM: std::sync::LazyLock<Song> = std::sync::LazyLock::new(|| Song {
            album: Some(Album {
                id: AlbumId(4),
                name: "一个非常非常长的中文专辑名字用来测试元数据面板的截断行为是否按列宽生效"
                    .into(),
                cover_url: None,
            }),
            ..SONG.clone()
        });
        let buffer = draw(Showing {
            song: Some(&*LONG_ALBUM),
            metadata: true,
            ..Showing::default()
        });

        let area = pane_area();
        let y = row_of(&buffer, "ALBUM");
        let row = row_text(&buffer, area, y);
        assert!(row.contains('…'), "not marked as shortened: {row:?}");
        assert_eq!(row_width(&buffer, area, y), area.width as usize);
    }

    #[test]
    fn every_metadata_row_fills_the_pane() {
        let buffer = draw(playing(&lossless()));

        for label in ["ALBUM", "DURATION", "QUALITY", "FORMAT", "SONG ID"] {
            let y = row_of(&buffer, label);
            assert_eq!(
                row_width(&buffer, pane_area(), y),
                pane_area().width as usize,
                "the {label} row does not span the pane"
            );
        }
    }

    // ---- progress --------------------------------------------------------

    #[test]
    fn the_knob_sits_where_playback_has_got_to() {
        let track = 20;
        let theme = crate::theme::Theme::default();
        let row = |position, duration| {
            super::parts(
                "01:00",
                "02:00",
                track,
                progress_ratio(position, duration),
                &theme,
            )
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
        };

        assert!(row(Some(30_000), Some(120_000)).contains("●"));
        // A quarter of the way along: the knob is in the left half of the track.
        let quarter = row(Some(30_000), Some(120_000));
        let half = row(Some(60_000), Some(120_000));
        assert!(
            quarter.find('●').unwrap() < half.find('●').unwrap(),
            "the knob did not move with the position"
        );
        // The row is the same width wherever the knob is.
        assert_eq!(quarter.chars().count(), half.chars().count());
    }

    #[test]
    fn a_position_with_no_length_leaves_the_knob_at_the_start() {
        // Nothing is playing yet: the knob at the start reads as "not started",
        // which is the truth, rather than as some position it is not at.
        assert_eq!(progress_ratio(None, Some(120_000)), None);
        assert_eq!(progress_ratio(Some(1_000), None), None);
        assert_eq!(progress_ratio(Some(1_000), Some(0)), None);
    }

    #[test]
    fn a_position_past_the_end_does_not_run_off_the_row() {
        assert_eq!(progress_ratio(Some(999_000), Some(1_000)), Some(1.0));
        let theme = crate::theme::Theme::default();
        let spans = super::parts("01:00", "02:00", 10, Some(1.0), &theme);
        let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(crate::render::display_width(&text), 10 + 2 + 5 * 2);
        assert!(text.ends_with("02:00"));
    }

    #[test]
    fn the_times_shown_are_the_players_own() {
        let buffer = draw(Showing {
            song: Some(&SONG),
            // The track says 04:29, but the player found 03:01 in the file.
            position: Some(97_000),
            duration: Some(181_000),
            ..Showing::default()
        });
        let area = pane_area();
        let row = row_text(&buffer, area, area.y + area.height - 1);

        assert!(row.starts_with("01:37"), "{row:?}");
        assert!(row.ends_with("03:01"), "{row:?}");
    }

    // ---- artwork ---------------------------------------------------------

    /// A picture two cells wide, filled with a recognisable character.
    fn picture() -> ArtworkView {
        let cell = |symbol: char| crate::state::ArtworkCell {
            symbol,
            fg: ArtworkInk::Rgb(255, 0, 0),
            bg: ArtworkInk::Rgb(0, 0, 255),
        };
        ArtworkView::new(2, 2, vec![cell('X'), cell('X'), cell('X'), cell('X')])
    }

    #[test]
    fn artwork_is_drawn_centred_in_its_region() {
        let artwork = picture();
        let buffer = draw(Showing {
            artwork: Some(&artwork),
            ..Showing::default()
        });

        let area = art_area();
        let left = area.x + (area.width - 2) / 2;
        let top = area.y + (area.height - 2) / 2;

        for y in 0..2 {
            for x in 0..2 {
                let cell = buffer.cell((left + x, top + y)).unwrap();
                assert_eq!(cell.symbol(), "X", "at {x},{y}");
                // The upper and lower halves keep their own colours, which is
                // what puts two samples in one cell.
                assert_eq!(cell.fg, Color::Rgb(255, 0, 0));
                assert_eq!(cell.bg, Color::Rgb(0, 0, 255));
            }
        }
    }

    #[test]
    fn artwork_stays_inside_its_region() {
        // A picture wider than the region must be clipped, not written over the
        // rest of the pane.
        let wide = ArtworkView::new(
            200,
            1,
            vec![
                crate::state::ArtworkCell {
                    symbol: 'X',
                    fg: ArtworkInk::Accent,
                    bg: ArtworkInk::Accent,
                };
                200
            ],
        );
        let buffer = draw(Showing {
            artwork: Some(&wide),
            ..Showing::default()
        });

        let area = art_area();
        // Nothing of the picture escaped into the track information below.
        for y in area.y + area.height..pane_area().y + pane_area().height {
            let row = row_text(&buffer, pane_area(), y);
            assert!(
                !row.contains('X'),
                "the picture ran past its region: {row:?}"
            );
        }
    }

    #[test]
    fn the_accent_ink_is_drawn_in_the_theme_accent() {
        let theme = crate::theme::Theme::default();
        assert_eq!(
            ink_colour(ArtworkInk::Accent, &theme),
            theme.accent.fg.unwrap()
        );
        assert_eq!(ink_colour(ArtworkInk::None, &theme), Color::Reset);
    }

    #[test]
    fn the_classic_colours_are_named_not_numbered() {
        // Color::Indexed(1) is whatever the user's theme has put in the palette
        // slot; Color::Red is the classic escape and is not the same thing.
        assert_eq!(classic_colour(1), Color::Red);
        assert_ne!(classic_colour(1), Color::Indexed(1));
        assert_eq!(classic_colour(15), Color::White);
        // An out-of-range index wraps rather than panicking.
        assert_eq!(classic_colour(16), Color::Black);
    }

    #[test]
    fn a_picture_smaller_than_its_declared_size_draws_blank_rather_than_wrapping() {
        let view = ArtworkView::new(2, 2, vec![]);
        assert!(!view.is_empty());
        assert_eq!(view.cell(0, 0).unwrap().symbol, ' ');
        assert!(view.cell(2, 0).is_none());
    }
}
