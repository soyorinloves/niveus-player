//! The bottom row.
//!
//! Every actionable item carries the key that triggers it, and the key is read
//! from the live keymap so a rebinding cannot leave the display lying.

use super::split_line;
use crate::state::{ListView, UiState};
use crate::theme::Theme;
use niveus_config::Action;
use niveus_music_core::{PlaybackMode, PlaybackStatus};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Separator between status segments, as drawn in the design.
const SEPARATOR: &str = " │ ";

pub(crate) fn render(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let line = compose(state, theme, area.width);
    frame.render_widget(Paragraph::new(line), area);
}

/// Builds the row, deciding what fits.
///
/// Separated from drawing so the decision can be tested directly: it is where a
/// width mistake would show up, and it produces nothing a rendered buffer can
/// easily be asked about.
fn compose(state: &UiState, theme: &Theme, width: u16) -> Line<'static> {
    let left = match state.status.notice {
        // A notice is more important than the usual readout.
        Some(notice) => vec![Span::styled(notice.to_string(), theme.error)],
        None => readout(state, theme, width as usize),
    };

    // Measured in columns, not characters: a Chinese notice is about twice as
    // wide as its character count, and understating it here would decide the
    // reminders fit and let them run off the end of the row.
    let left_width: usize = left.iter().map(Span::width).sum();
    let hints = view_hints(state, theme);
    let hints_width: usize = hints.iter().map(Span::width).sum();

    let right = if width as usize > left_width + hints_width + 1 {
        hints
    } else {
        // Not enough room for both: the readout is the one that matters, so the
        // reminders are dropped whole rather than clipped mid-word.
        Vec::new()
    };

    split_line(left, right, width)
}

/// The readout, with the bitrate segment when the row can hold it.
///
/// Every other segment carries the key that triggers it, so a row too narrow
/// for all of them would clip a reminder of how to work the thing. The bitrate
/// is the one segment that is only information, so it is the one that gives way.
fn readout(state: &UiState, theme: &Theme, width: usize) -> Vec<Span<'static>> {
    if let Some(kbps) = state.status.bitrate.and_then(kilobits) {
        let with_bitrate = readout_spans(state, theme, Some(kbps));
        if with_bitrate.iter().map(Span::width).sum::<usize>() <= width {
            return with_bitrate;
        }
    }
    readout_spans(state, theme, None)
}

fn readout_spans(state: &UiState, theme: &Theme, bitrate: Option<u32>) -> Vec<Span<'static>> {
    let status = &state.status;
    let mut spans: Vec<Span<'static>> = Vec::new();

    let push = |spans: &mut Vec<Span<'static>>, span: Span<'static>| {
        if !spans.is_empty() {
            spans.push(Span::styled(SEPARATOR, theme.divider));
        }
        spans.push(span);
    };

    // Playback state carries no key: it reports, it does not act.
    //
    // Paused is not stopped: a user who has just pressed the pause key needs to
    // see that what they did took effect, and "STOPPED" would say the opposite.
    let (label, style) = match status.playback {
        PlaybackStatus::Playing => ("PLAYING", theme.accent),
        PlaybackStatus::Paused => ("PAUSED", theme.normal),
        PlaybackStatus::Stopped => ("STOPPED", theme.muted),
    };
    push(&mut spans, Span::styled(label, style));

    // What the resource playing is actually encoded at, where the design puts
    // it — between the state and the volume. Absent for a resource that did not
    // report one: a segment that only ever says "—" would cost the row width
    // and tell nobody anything.
    if let Some(bitrate) = bitrate {
        push(
            &mut spans,
            Span::styled(format!("{bitrate} KBPS"), theme.normal),
        );
    }

    // Volume's two directions share one hint, as the design writes them.
    push(
        &mut spans,
        Span::styled(volume_label(status.volume, state), theme.normal),
    );
    push(
        &mut spans,
        Span::styled(
            format!(
                "QUEUE {} {}",
                status.queue_len,
                state.hint(Action::ToggleQueue)
            ),
            theme.normal,
        ),
    );
    push(
        &mut spans,
        Span::styled(
            format!(
                "{} {}",
                playback_mode_label(status.playback_mode),
                state.hint(Action::CyclePlaybackMode)
            ),
            theme.normal,
        ),
    );
    push(
        &mut spans,
        Span::styled(
            format!(
                "{} {}",
                if status.playback == PlaybackStatus::Playing {
                    "PAUSE"
                } else {
                    "PLAY"
                },
                state.hint(Action::PlayPause)
            ),
            theme.accent,
        ),
    );

    if status.loading {
        push(&mut spans, Span::styled("LOADING…", theme.muted));
    }

    spans
}

/// The bitrate in kilobits per second, as the design writes it.
///
/// Rounded rather than truncated, and nothing is shown for a rate too low to be
/// a bitrate at all — a zero would only come from a service reporting nonsense.
fn kilobits(bits_per_second: u32) -> Option<u32> {
    let kbps = (bits_per_second + 500) / 1000;
    (kbps > 0).then_some(kbps)
}

fn volume_label(volume: u8, state: &UiState) -> String {
    let up = key_label(state, Action::VolumeUp);
    let down = key_label(state, Action::VolumeDown);
    format!("VOL {volume}% [{down}/{up}]")
}

fn key_label(state: &UiState, action: Action) -> String {
    state
        .keymap
        .key_for(action)
        .map(|key| key.label())
        .unwrap_or_else(|| " ".to_string())
}

fn playback_mode_label(mode: PlaybackMode) -> String {
    mode.label().to_uppercase()
}

/// Short reminders of what the current view responds to.
fn view_hints(state: &UiState, theme: &Theme) -> Vec<Span<'static>> {
    let items: &[(&str, Action)] = match &state.list {
        ListView::Playlists { .. } => &[
            ("MOVE", Action::MoveUp),
            ("OPEN", Action::Select),
            ("QUEUE", Action::ToggleQueue),
        ],
        ListView::Songs { .. } | ListView::Likes { .. } | ListView::History { .. } => &[
            ("MOVE", Action::MoveUp),
            ("PLAY", Action::Select),
            ("BACK", Action::Back),
            ("QUEUE", Action::ToggleQueue),
        ],
        // The queue's own operations, each with the key that performs it.
        ListView::Queue { .. } => &[
            ("MOVE", Action::MoveUp),
            ("PLAY", Action::Select),
            ("ORDER", Action::ReorderUp),
            ("DEL", Action::DeleteSong),
            ("BACK", Action::Back),
        ],
        ListView::Settings { .. } => &[
            ("MOVE", Action::MoveUp),
            ("EDIT", Action::Select),
            ("BACK", Action::Back),
            ("QUIT", Action::Quit),
        ],
        // Rebinding: pick a row, press Enter, then press the key to give it.
        ListView::Keybindings { .. } => &[
            ("MOVE", Action::MoveUp),
            ("REBIND", Action::Select),
            ("BACK", Action::Back),
        ],
        ListView::Message(_) => &[("QUIT", Action::Quit)],
    };

    let mut spans = Vec::new();
    for (label, action) in items {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        // Moving and reordering are each bound in two directions; show the pair.
        let key = match action {
            Action::MoveUp => format!(
                "{}{}",
                key_label(state, Action::MoveUp),
                key_label(state, Action::MoveDown)
            ),
            Action::ReorderUp => format!(
                "{}{}",
                key_label(state, Action::ReorderUp),
                key_label(state, Action::ReorderDown)
            ),
            other => key_label(state, *other),
        };
        spans.push(Span::styled(format!("{label} [{key}]"), theme.muted));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::super::display_width;
    use super::*;
    use crate::layout::LayoutTier;
    use crate::render::render;
    use crate::state::{
        ListView, LyricsView, NowPlayingView, Page, SessionView, StatusView, UiState,
    };
    use crate::testing::line_text;
    use niveus_config::{CoverSize, Key, Keymap};
    use niveus_music_core::{Song, SongId, User, UserId};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn draw(list: ListView<'static>, loading: bool, notice: Option<&'static str>) -> Buffer {
        let keymap = Keymap::default();
        let user = User {
            id: UserId(1),
            name: "n".into(),
        };
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Ready { user: &user },
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
            keymap: &keymap,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        };
        let area = Rect::new(0, 0, 140, 40);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &crate::theme::Theme::default()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// The whole frame at a given size, so the status row can be read in the
    /// company it is actually drawn in.
    fn draw_at(list: ListView<'static>, notice: Option<&'static str>, area: Rect) -> Buffer {
        let keymap = Keymap::default();
        let user = User {
            id: UserId(1),
            name: "n".into(),
        };
        let state = UiState {
            page: Page::Settings,
            session: SessionView::Ready { user: &user },
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
                loading: false,
                notice,
                volume: 72,
                playback_mode: PlaybackMode::Sequential,
                queue_len: 89,
                playback: PlaybackStatus::Stopped,
                bitrate: None,
            },
            keymap: &keymap,
            tier: LayoutTier::for_size(area.width, area.height),
            cover_size: CoverSize::Normal,
        };
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &crate::theme::Theme::default()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn the_reminders_are_not_overtaken_by_what_they_replaced() {
        // The keybinding view has one reminder fewer than the settings view,
        // and the row is drawn again in place. A narrower terminal is where a
        // row that is rewritten in part shows it: the tail of the old reminder
        // must not survive under the new one.
        let area = Rect::new(0, 0, 80, 24);
        let buffer = draw_at(
            ListView::Keybindings {
                selected: 0,
                scroll: 0,
                capturing: true,
            },
            Some("按下要绑定的按键，Esc 取消"),
            area,
        );

        let status_row = crate::testing::row_text(&buffer, area, area.y + area.height - 1);
        assert!(status_row.contains("REBIND"), "{status_row:?}");
        assert!(
            !status_row.contains("REBPLAY"),
            "an old reminder is still showing through: {status_row:?}"
        );
        // Every row is exactly as wide as the terminal, which is what stops a
        // stale cell from any earlier frame surviving.
        for y in area.y..area.y + area.height {
            assert_eq!(
                crate::testing::row_width(&buffer, area, y),
                area.width as usize,
                "row {y} does not span the terminal"
            );
        }
    }

    /// The same row, with playback in a given state.
    fn draw_with(list: ListView<'static>, playback: PlaybackStatus) -> Buffer {
        let keymap = Keymap::default();
        let user = User {
            id: UserId(1),
            name: "n".into(),
        };
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Ready { user: &user },
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
                loading: false,
                notice: None,
                volume: 72,
                playback_mode: PlaybackMode::Sequential,
                queue_len: 12,
                playback,
                bitrate: None,
            },
            keymap: &keymap,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        };
        let area = Rect::new(0, 0, 140, 40);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &crate::theme::Theme::default()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn status_text(buffer: &Buffer) -> String {
        // The status row is the last one, and the buffer knows its own size.
        let area = buffer.area;
        (0..area.width)
            .map(|x| buffer.cell((x, area.y + area.height - 1)).unwrap().symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn the_readout_matches_the_designed_shape() {
        let text = status_text(&draw(ListView::Message(""), false, None));
        assert!(text.starts_with("STOPPED"), "{text}");
        assert!(text.contains("VOL 72% [-/=]"), "{text}");
        assert!(text.contains("QUEUE 12 [U]"), "{text}");
        assert!(text.contains("SEQUENTIAL [O]"), "{text}");
        assert!(text.contains("PLAY [Space]"), "{text}");
    }

    #[test]
    fn a_paused_track_says_it_is_paused() {
        // "STOPPED" would tell a user who has just pressed the pause key the
        // opposite of what happened.
        let text = status_text(&draw_with(ListView::Message(""), PlaybackStatus::Paused));
        assert!(text.starts_with("PAUSED"), "{text}");
        // And the key offers the way back.
        assert!(text.contains("PLAY [Space]"), "{text}");
    }

    #[test]
    fn a_running_track_offers_to_pause_it() {
        let text = status_text(&draw_with(ListView::Message(""), PlaybackStatus::Playing));
        assert!(text.starts_with("PLAYING"), "{text}");
        assert!(text.contains("PAUSE [Space]"), "{text}");
    }

    #[test]
    fn loading_is_announced_without_blocking_the_view() {
        let text = status_text(&draw(ListView::Message(""), true, None));
        assert!(text.contains("LOADING…"), "{text}");
        // The rest of the readout survives.
        assert!(text.contains("VOL 72%"), "{text}");
    }

    #[test]
    fn a_notice_replaces_the_readout() {
        let text = status_text(&draw(
            ListView::Message(""),
            false,
            Some("该歌曲暂不可播放"),
        ));
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("该歌曲暂不可播放"), "{text}");
        assert!(!text.contains("VOL 72%"), "{text}");
    }

    #[test]
    fn hints_follow_the_view() {
        let browsing = status_text(&draw(ListView::Message(""), false, None));
        assert!(browsing.contains("QUIT [Q]"), "{browsing}");

        let songs = status_text(&draw(
            ListView::Songs {
                playlist_name: "x",
                songs: &[],
                selected: 0,
                scroll: 0,
            },
            false,
            None,
        ));
        assert!(songs.contains("PLAY [Enter]"), "{songs}");
        assert!(songs.contains("BACK [Esc]"), "{songs}");
    }

    #[test]
    fn a_rebound_key_changes_the_hint() {
        let mut keymap = Keymap::default();
        keymap.rebind(Action::ToggleQueue, Key::Char('z')).unwrap();
        let user = User {
            id: UserId(1),
            name: "n".into(),
        };
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Ready { user: &user },
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
                queue_len: 12,
                playback: PlaybackStatus::Stopped,
                bitrate: None,
            },
            keymap: &keymap,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        };
        let area = Rect::new(0, 0, 140, 40);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &crate::theme::Theme::default()))
            .unwrap();
        let text = status_text(terminal.backend().buffer());
        assert!(text.contains("QUEUE 12 [Z]"), "{text}");
    }

    #[test]
    fn hints_are_dropped_before_the_readout_when_space_runs_out() {
        let keymap = Keymap::default();
        let user = User {
            id: UserId(1),
            name: "n".into(),
        };
        // A track list carries the longest hint set, so this is where the two
        // halves of the row first compete for room.
        let songs = [Song {
            id: SongId(1),
            title: "x".into(),
            artists: Vec::new(),
            album: None,
            duration_ms: None,
        }];
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Ready { user: &user },
            list: ListView::Songs {
                playlist_name: "x",
                songs: &songs,
                selected: 0,
                scroll: 0,
            },
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
                queue_len: 12,
                playback: PlaybackStatus::Stopped,
                bitrate: None,
            },
            keymap: &keymap,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        };
        let area = Rect::new(0, 0, 84, 40);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &crate::theme::Theme::default()))
            .unwrap();
        let text = status_text(terminal.backend().buffer());

        // The readout survives intact...
        assert!(text.contains("VOL 72%"), "{text}");
        assert!(text.contains("QUEUE 12 [U]"), "{text}");
        // ...and the reminders give way rather than pushing it off the edge.
        assert!(!text.contains("BACK"), "{text}");
    }

    // ---- Chinese content ---------------------------------------------------
    //
    // A Chinese notice is roughly twice as wide as its character count, which
    // is exactly the gap that let the reminders overrun the row while the
    // fit test still reported there was room.

    /// The longest notice the app can raise, at 36 columns.
    const LONG_NOTICE: &str = "播放控制尚未接入，当前版本仅支持浏览";

    fn composing(
        list: ListView<'static>,
        notice: Option<&'static str>,
        width: u16,
    ) -> Line<'static> {
        composing_with_bitrate(list, notice, None, width)
    }

    fn composing_with_bitrate(
        list: ListView<'static>,
        notice: Option<&'static str>,
        bitrate: Option<u32>,
        width: u16,
    ) -> Line<'static> {
        let keymap = Keymap::default();
        let user = User {
            id: UserId(1),
            name: "n".into(),
        };
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Ready { user: &user },
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
                loading: false,
                notice,
                volume: 72,
                playback_mode: PlaybackMode::Sequential,
                queue_len: 12,
                playback: PlaybackStatus::Stopped,
                bitrate,
            },
            keymap: &keymap,
            tier: LayoutTier::Standard,
            cover_size: CoverSize::Normal,
        };
        super::compose(&state, &crate::theme::Theme::default(), width)
    }

    fn song_list() -> ListView<'static> {
        ListView::Songs {
            playlist_name: "x",
            songs: &[],
            selected: 0,
            scroll: 0,
        }
    }

    #[test]
    fn a_chinese_notice_is_measured_in_columns_not_characters() {
        // Eighteen characters, thirty-six columns: the gap that the fit test
        // used to fall into.
        assert_eq!(LONG_NOTICE.chars().count(), 18);
        assert_eq!(display_width(LONG_NOTICE), 36);
    }

    #[test]
    fn a_wide_chinese_notice_leaves_no_room_for_the_reminders() {
        // 36 columns of notice and 46 of reminders do not fit in 80. Measured in
        // characters the notice would look like 18, the reminders would be
        // rendered, and they would end up clipped against the edge.
        let line = composing(song_list(), Some(LONG_NOTICE), 80);
        let text = line_text(&line);

        // The line holds only the notice; the renderer fills the rest of the
        // region. What matters is that it does not exceed it.
        assert!(line.width() <= 80, "the row overflows: {text:?}");
        assert!(text.starts_with(LONG_NOTICE), "{text:?}");
        assert!(
            !text.contains("QUEUE") && !text.contains("MOVE"),
            "a reminder was clipped into the row instead of being dropped: {text:?}"
        );
    }

    #[test]
    fn a_wide_chinese_notice_keeps_the_reminders_when_there_is_room() {
        let line = composing(song_list(), Some(LONG_NOTICE), 140);
        let text = line_text(&line);

        assert_eq!(line.width(), 140);
        assert!(text.starts_with(LONG_NOTICE));
        // Reminders are whole and flush against the right edge.
        assert!(text.contains("QUEUE [U]"), "{text:?}");
        assert!(text.ends_with(']'), "{text:?}");
    }

    #[test]
    fn the_readout_itself_is_measured_in_columns() {
        // No notice, so the row is all ASCII: it fits at 120 and the reminders
        // survive.
        let line = composing(ListView::Message(""), None, 120);
        let text = line_text(&line);
        assert_eq!(line.width(), 120);
        assert!(text.contains("VOL 72%"), "{text:?}");
    }

    // ---- bitrate -----------------------------------------------------------

    #[test]
    fn the_bitrate_of_the_resource_playing_sits_where_the_design_puts_it() {
        let text = line_text(&composing_with_bitrate(
            ListView::Message(""),
            None,
            Some(320_000),
            140,
        ));

        assert!(text.contains("320 KBPS"), "{text:?}");
        // Between the playback state and the volume, as the design draws it.
        let bitrate = text.find("KBPS").expect("the segment is there");
        assert!(
            bitrate < text.find("VOL").expect("the volume is there"),
            "{text:?}"
        );
        assert!(bitrate > "PLAYING".len(), "{text:?}");
    }

    #[test]
    fn a_bitrate_nobody_reported_leaves_the_segment_out() {
        let text = line_text(&composing_with_bitrate(
            ListView::Message(""),
            None,
            None,
            140,
        ));
        assert!(!text.contains("KBPS"), "{text:?}");
        // The rest of the readout is unaffected.
        assert!(text.contains("VOL 72%"), "{text:?}");
    }

    #[test]
    fn a_row_too_narrow_for_the_bitrate_drops_it_rather_than_a_key_hint() {
        // Every other segment carries the key that triggers it; the bitrate is
        // the only one that is information alone, so it is the one that gives
        // way. Clipping a hint would leave the user with a key they cannot read.
        //
        // The row is 81 columns with the bitrate and 70 without, which makes
        // the boundary between the two exact.
        let narrow = line_text(&composing_with_bitrate(
            ListView::Message(""),
            None,
            Some(320_000),
            80,
        ));
        assert!(
            !narrow.contains("KBPS"),
            "the bitrate squeezed the row: {narrow:?}"
        );
        assert!(narrow.contains("PLAY [Space]"), "{narrow:?}");
        assert!(display_width(&narrow) <= 80, "{narrow:?}");

        let just_fits = line_text(&composing_with_bitrate(
            ListView::Message(""),
            None,
            Some(320_000),
            81,
        ));
        assert!(just_fits.contains("320 KBPS"), "{just_fits:?}");
        assert!(display_width(&just_fits) <= 81, "{just_fits:?}");
    }

    #[test]
    fn bitrates_are_reported_in_whole_kilobits() {
        assert_eq!(kilobits(320_000), Some(320));
        assert_eq!(kilobits(999_000), Some(999));
        assert_eq!(kilobits(128_000), Some(128));
        // Rounded, not truncated: 999.6 kbps is closer to 1000 than to 999.
        assert_eq!(kilobits(999_600), Some(1000));
        // A service reporting nonsense leaves the segment out.
        assert_eq!(kilobits(0), None);
        assert_eq!(kilobits(400), None);
    }
}
