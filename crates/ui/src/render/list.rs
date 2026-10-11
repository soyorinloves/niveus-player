//! The browsable pane.
//!
//! Rows are built for the visible window only, so a playlist of a thousand
//! tracks costs the same as one of twenty.

use super::{
    empty_note, format_date, format_duration, horizontal_rule, line_of, split_line, truncate,
    truncate_spans, CURSOR, UNKNOWN,
};
use crate::state::{ListView, Setting, UiState};
use crate::theme::Theme;
use niveus_config::{Action, Key};
use niveus_music_core::{Playlist, PlaylistKind, Song};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Width of the cursor column, including the space after the marker.
const CURSOR_WIDTH: usize = 2;

/// Rows a single list entry occupies in each view.
pub(crate) const SONG_ROWS: usize = 2;

/// Marks the row of the track playing now in the queue view.
const PLAYING_MARK: &str = "正在播放";

pub(crate) fn render(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    if area.width == 0 || area.height < 2 {
        return;
    }

    let [title_area, rule_area, content_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);

    frame.render_widget(
        Paragraph::new(title_line(state, title_area.width, theme)),
        title_area,
    );
    horizontal_rule(frame, rule_area, theme);

    let lines = content_lines(state, content_area.width, content_area.height, theme);
    frame.render_widget(Paragraph::new(lines), content_area);
}

/// The pane's heading, which also tells the user how to get back.
///
/// A playlist name can be any length, so the heading is shortened to the pane
/// like any other row.
fn title_line(state: &UiState, width: u16, theme: &Theme) -> Line<'static> {
    let spans = match &state.list {
        ListView::Playlists { .. } => vec![Span::styled("PLAYLISTS", theme.heading)],
        ListView::Songs { playlist_name, .. } => vec![
            Span::styled("CURRENT PLAYLIST", theme.heading),
            Span::styled(format!("  ‹ {playlist_name}"), theme.normal),
        ],
        ListView::Likes { .. } => vec![Span::styled("FAVORITES", theme.heading)],
        ListView::History { .. } => vec![Span::styled("HISTORY", theme.heading)],
        // The heading carries the queue's length, which is what tells the user
        // how much is waiting.
        ListView::Queue { songs, .. } => vec![
            Span::styled("QUEUE", theme.heading),
            Span::styled(format!("  {}", songs.len()), theme.normal),
        ],
        ListView::Settings { .. } => vec![Span::styled("SETTINGS", theme.heading)],
        // The same shape as a playlist's heading: where this is, and how to get
        // back out of it.
        ListView::Keybindings { .. } => vec![
            Span::styled("SETTINGS", theme.heading),
            Span::styled("  ‹ KEYBINDINGS", theme.normal),
        ],
        ListView::Message(_) => Vec::new(),
    };
    Line::from(truncate_spans(spans, width as usize))
}

fn content_lines(state: &UiState, width: u16, height: u16, theme: &Theme) -> Vec<Line<'static>> {
    let capacity = height as usize;
    let mut rows = Vec::with_capacity(capacity);

    match &state.list {
        ListView::Playlists {
            created,
            collected,
            selected,
            scroll,
        } => {
            let total = created.len() + collected.len();
            if total == 0 {
                return placeholder(state, width, theme);
            }
            let end = (*scroll + capacity).min(total);
            for index in *scroll..end {
                match entry(created, collected, index) {
                    Entry::Created(playlist) => {
                        if index == 0 {
                            rows.push(line_of(PlaylistKind::Created.heading(), theme.muted));
                        }
                        rows.push(playlist_line(playlist, index == *selected, width, theme));
                    }
                    Entry::Collected { playlist, first } => {
                        if first {
                            rows.push(Line::default());
                            rows.push(line_of(PlaylistKind::Collected.heading(), theme.muted));
                        }
                        rows.push(playlist_line(playlist, index == *selected, width, theme));
                    }
                }
            }
        }
        ListView::Songs {
            songs,
            selected,
            scroll,
            ..
        }
        | ListView::Likes {
            songs,
            selected,
            scroll,
        } => {
            if songs.is_empty() {
                return placeholder(state, width, theme);
            }
            let per_screen = (capacity / SONG_ROWS).max(1);
            let end = (*scroll + per_screen).min(songs.len());
            for index in *scroll..end {
                push_song_rows(
                    &mut rows,
                    &songs[index],
                    index,
                    index == *selected,
                    width,
                    theme,
                );
            }
        }
        ListView::History {
            entries,
            selected,
            scroll,
        } => {
            if entries.is_empty() {
                return placeholder(state, width, theme);
            }
            let end = (*scroll + capacity).min(entries.len());
            for index in *scroll..end {
                let entry = &entries[index];
                rows.push(history_line(
                    &entry.song,
                    entry.played_at,
                    index == *selected,
                    width,
                    theme,
                ));
            }
        }
        ListView::Queue {
            songs,
            playing,
            selected,
            scroll,
        } => {
            if songs.is_empty() {
                return placeholder(state, width, theme);
            }
            let end = (*scroll + capacity).min(songs.len());
            for index in *scroll..end {
                rows.push(queue_line(
                    &songs[index],
                    index,
                    index == *selected,
                    *playing == Some(index),
                    width,
                    theme,
                ));
            }
        }
        ListView::Settings {
            values,
            selected,
            editing,
        } => {
            // The rows come from the settings' own declaration, so the drawing
            // code holds no list of its own to fall out of step with it.
            for (index, (setting, name)) in Setting::ALL.iter().enumerate() {
                let value = values.value(*setting);
                // The row being edited brackets its value, which is the only
                // sign that the arrows now change it rather than the cursor.
                let value = match editing {
                    Some(editing) if editing == setting => format!("‹ {value} ›"),
                    _ => value,
                };
                rows.push(setting_line(name, &value, index == *selected, width, theme));
            }
        }
        ListView::Keybindings {
            selected,
            scroll,
            capturing,
        } => {
            let end = (*scroll + capacity).min(Action::ALL.len());
            for index in *scroll..end {
                let action = Action::ALL[index];
                rows.push(keybinding_line(
                    action,
                    state.keymap.key_for(action),
                    index == *selected,
                    *capturing && index == *selected,
                    width,
                    theme,
                ));
            }
        }
        ListView::Message(message) => {
            rows.push(line_of(truncate(message, width as usize), theme.muted));
        }
    }

    rows.truncate(capacity);
    rows
}

/// Shown in place of a list that has nothing in it.
///
/// While a request is in flight the pane stays blank, as the design requires:
/// an empty list and a loading list must not look the same.
fn placeholder(state: &UiState, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    if state.status.loading {
        return Vec::new();
    }
    let note = empty_note(&state.list);
    if note.is_empty() {
        return Vec::new();
    }
    let _ = width;
    vec![line_of(note, theme.muted)]
}

enum Entry<'a> {
    Created(&'a Playlist),
    Collected { playlist: &'a Playlist, first: bool },
}

/// Resolves a flat selection index over the two groups.
fn entry<'a>(created: &'a [Playlist], collected: &'a [Playlist], index: usize) -> Entry<'a> {
    if index < created.len() {
        Entry::Created(&created[index])
    } else {
        let offset = index - created.len();
        Entry::Collected {
            playlist: &collected[offset],
            first: offset == 0,
        }
    }
}

fn playlist_line(playlist: &Playlist, selected: bool, width: u16, theme: &Theme) -> Line<'static> {
    let style = if selected { theme.accent } else { theme.normal };
    let marker = if selected { CURSOR } else { " " };
    let marker_style = if selected { theme.accent } else { theme.normal };
    Line::from(truncate_spans(
        vec![
            Span::styled(format!("{marker} "), marker_style),
            Span::styled(playlist.name.clone(), style),
        ],
        width as usize,
    ))
}

/// Two rows: identity, then artist, album and duration.
fn push_song_rows(
    rows: &mut Vec<Line<'static>>,
    song: &Song,
    index: usize,
    selected: bool,
    width: u16,
    theme: &Theme,
) {
    let style = if selected { theme.accent } else { theme.normal };
    let marker = if selected { CURSOR } else { " " };
    let marker_style = if selected { theme.accent } else { theme.normal };

    rows.push(Line::from(truncate_spans(
        vec![
            Span::styled(format!("{marker} "), marker_style),
            Span::styled(format!("{:02}  ", index + 1), theme.muted),
            Span::styled(song.title.clone(), style),
        ],
        width as usize,
    )));

    // The duration keeps the right-hand column: a long credit line is shortened
    // to make room for it rather than pushing it off the row.
    let indent = " ".repeat(CURSOR_WIDTH + 4);
    let subtitle = format!("{} · {}", song.artist_names(), song.album_name());
    rows.push(split_line(
        vec![Span::raw(indent), Span::styled(subtitle, theme.muted)],
        vec![Span::styled(
            format_duration(song.duration_ms.map(u64::from)),
            theme.muted,
        )],
        width,
    ));
}

/// One row of the history: the track, and the day it was last heard.
fn history_line(
    song: &Song,
    played_at: u64,
    selected: bool,
    width: u16,
    theme: &Theme,
) -> Line<'static> {
    let style = if selected { theme.accent } else { theme.normal };
    let marker = if selected { CURSOR } else { " " };
    let marker_style = if selected { theme.accent } else { theme.normal };

    split_line(
        vec![
            Span::styled(format!("{marker} "), marker_style),
            Span::styled(song.title.clone(), style),
            Span::styled(
                format!("    {} · {}", song.artist_names(), song.album_name()),
                theme.muted,
            ),
        ],
        vec![Span::styled(format_date(played_at), theme.muted)],
        width,
    )
}

/// One row of the keybinding list: the action, and the key it answers to.
///
/// While a rebind is being captured the row says so in place of its key, since
/// the next key pressed will not be the one already shown.
fn keybinding_line(
    action: Action,
    key: Option<Key>,
    selected: bool,
    capturing: bool,
    width: u16,
    theme: &Theme,
) -> Line<'static> {
    let marker = if selected { CURSOR } else { " " };
    let marker_style = if selected { theme.accent } else { theme.normal };
    let name_style = if selected { theme.accent } else { theme.normal };

    let value = match (capturing, key) {
        (true, _) => "按下新按键…".to_string(),
        (false, Some(key)) => key.label(),
        (false, None) => UNKNOWN.to_string(),
    };

    split_line(
        vec![
            Span::styled(format!("{marker} "), marker_style),
            // The action's name as it is written in `config.toml`, so what the
            // page shows is what the file the user may hand-edit says.
            Span::styled(action.name().to_string(), name_style),
        ],
        vec![Span::styled(value, theme.muted)],
        width,
    )
}

/// One row of the queue: its place, its title, and — on the track playing now —
/// a mark saying so.
///
/// The cursor and the playing track are separate things here: the user can move
/// around the queue while a different track plays, so one marker cannot serve
/// for both.
fn queue_line(
    song: &Song,
    index: usize,
    selected: bool,
    playing: bool,
    width: u16,
    theme: &Theme,
) -> Line<'static> {
    let marker = if selected { CURSOR } else { " " };
    let marker_style = if selected { theme.accent } else { theme.normal };
    let title_style = if selected { theme.accent } else { theme.normal };

    let right = if playing {
        vec![Span::styled(PLAYING_MARK, theme.accent)]
    } else {
        Vec::new()
    };

    split_line(
        vec![
            Span::styled(format!("{marker} "), marker_style),
            Span::styled(format!("{:02}  ", index + 1), theme.muted),
            Span::styled(song.title.clone(), title_style),
        ],
        right,
        width,
    )
}

fn setting_line(
    name: &str,
    value: &str,
    selected: bool,
    width: u16,
    theme: &Theme,
) -> Line<'static> {
    let marker = if selected { CURSOR } else { " " };
    let marker_style = if selected { theme.accent } else { theme.normal };
    let name_style = if selected { theme.accent } else { theme.normal };
    split_line(
        vec![
            Span::styled(format!("{marker} "), marker_style),
            Span::styled(name.to_string(), name_style),
        ],
        vec![Span::styled(value.to_string(), theme.muted)],
        width,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{compute, LayoutTier};
    use crate::render::render;
    use crate::state::{
        ListView, LoginView, LyricsView, NowPlayingView, Page, SessionView, Setting,
        SettingsValues, StatusView, UiState,
    };
    use crate::testing::{line_text, row_text, row_width};
    use niveus_config::{Action, CoverSize, Keymap};
    use niveus_music_core::{
        Album, AlbumId, Artist, ArtistId, PlaybackMode, PlaybackStatus, Playlist, PlaylistId,
        PlaylistKind, Song, SongId,
    };
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    fn song(id: u64, title: &str) -> Song {
        Song {
            id: SongId(id),
            title: title.to_string(),
            artists: vec![Artist {
                id: ArtistId(1),
                name: "Artist".into(),
            }],
            album: Some(Album {
                id: AlbumId(1),
                name: "Album".into(),
                cover_url: None,
            }),
            duration_ms: Some(261_000),
        }
    }

    fn playlist(id: u64, name: &str, kind: PlaylistKind) -> Playlist {
        Playlist {
            id: PlaylistId(id),
            name: name.to_string(),
            kind,
        }
    }

    fn draw<'a>(list: ListView<'a>) -> Buffer {
        let area = Rect::new(0, 0, 120, 40);
        draw_in(list, false, area)
    }

    fn draw_in<'a>(list: ListView<'a>, loading: bool, area: Rect) -> Buffer {
        let keymap = Keymap::default();
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Ready {
                user: &niveus_music_core::User {
                    id: niveus_music_core::UserId(1),
                    name: "n".into(),
                },
            },
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
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &crate::theme::Theme::default()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// A wide character spans two cells, and the cell it covers reads back as a
    /// space. Comparison ignores that.
    fn compact(text: &str) -> String {
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn text_of(buffer: &Buffer, area: Rect) -> String {
        (area.y..area.y + area.height)
            .map(|y| {
                (area.x..area.x + area.width)
                    .map(|x| buffer.cell((x, y)).unwrap().symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn list_area() -> Rect {
        compute(
            Rect::new(0, 0, 120, 40),
            LayoutTier::Standard,
            CoverSize::Normal,
        )
        .list
    }

    // ---- Chinese content ---------------------------------------------------
    //
    // A long Chinese title is the ordinary case, not an edge case, and it is
    // where a width mistake turns the duration column invisible.

    /// A track whose title, artist and album are all far too long to fit.
    fn long_chinese_song() -> Song {
        Song {
            id: SongId(2),
            title: "这是一首名字特别特别长的中文歌曲标题用来测试截断行为".into(),
            artists: vec![Artist {
                id: ArtistId(7),
                name: "演唱者名字也很长的歌手".into(),
            }],
            album: Some(Album {
                id: AlbumId(8),
                name: "专辑名字同样很长的专辑".into(),
                cover_url: None,
            }),
            duration_ms: Some(261_000),
        }
    }

    const NARROW: u16 = 40;

    #[test]
    fn a_long_chinese_title_is_shortened_but_keeps_the_duration_visible() {
        let mut rows = Vec::new();
        push_song_rows(
            &mut rows,
            &long_chinese_song(),
            0,
            false,
            NARROW,
            &crate::theme::Theme::default(),
        );

        let title = line_text(&rows[0]);
        assert!(
            rows[0].width() <= NARROW as usize,
            "title overflows: {title:?}"
        );
        assert!(
            title.ends_with('…'),
            "title was not marked as shortened: {title:?}"
        );

        let credit = line_text(&rows[1]);
        assert_eq!(rows[1].width(), NARROW as usize, "{credit:?}");
        assert!(
            credit.ends_with("04:21"),
            "the duration was pushed off the row: {credit:?}"
        );
        assert!(credit.contains('…'), "{credit:?}");
        // The shortened credit never runs into the duration.
        assert!(!credit.contains("…04:21"), "{credit:?}");
    }

    #[test]
    fn a_long_chinese_playlist_name_is_shortened() {
        let playlist = playlist(
            1,
            "一个名字非常非常长的中文歌单名字用来测试截断行为是否正确",
            PlaylistKind::Created,
        );
        let line = playlist_line(&playlist, true, NARROW, &crate::theme::Theme::default());
        let text = line_text(&line);

        assert!(line.width() <= NARROW as usize, "{text:?}");
        assert!(text.starts_with('▶'), "{text:?}");
        assert!(text.ends_with('…'), "{text:?}");
    }

    #[test]
    fn a_long_chinese_history_row_is_shortened() {
        let line = history_line(
            &long_chinese_song(),
            1_700_000_000,
            false,
            NARROW,
            &crate::theme::Theme::default(),
        );
        let text = line_text(&line);

        assert!(line.width() <= NARROW as usize, "{text:?}");
        // A long Chinese title is shortened to make room for the date rather
        // than pushing it off the row, so the date is what the row keeps.
        assert!(text.contains('…'), "{text:?}");
        assert!(text.ends_with("11-14"), "{text:?}");
    }

    #[test]
    fn a_history_row_shows_the_day_the_track_was_heard() {
        // 2023-11-14, 22:13:20 UTC.
        let line = history_line(
            &song(1, "Sunny Day"),
            1_700_000_000,
            false,
            NARROW,
            &crate::theme::Theme::default(),
        );
        let text = line_text(&line);
        assert_eq!(crate::render::format_date(1_700_000_000), "11-14");
        assert!(text.contains("Sunny Day"), "{text:?}");
        assert!(text.ends_with("11-14"), "{text:?}");
    }

    #[test]
    fn a_long_chinese_message_is_shortened() {
        let message =
            "加载歌单失败：网络请求失败，请检查网络连接后重试，如果问题持续存在请查看日志文件";
        let buffer = draw(ListView::Message(message));
        let area = list_area();

        for y in 0..area.height {
            assert_eq!(
                row_width(&buffer, area, y),
                area.width as usize,
                "row {y} does not span the pane"
            );
        }
        let text = row_text(&buffer, area, area.y + 2);
        assert!(text.contains('…'), "{text:?}");
    }

    #[test]
    fn a_chinese_row_still_fills_its_pane() {
        let songs = vec![long_chinese_song()];
        let buffer = draw(ListView::Songs {
            playlist_name: "一个名字非常非常长的中文歌单名字用来测试标题行的截断行为是否生效",
            songs: &songs,
            selected: 0,
            scroll: 0,
        });
        let area = list_area();

        for y in 0..area.height {
            assert_eq!(
                row_width(&buffer, area, y),
                area.width as usize,
                "row {y} does not span the pane"
            );
        }
        // The title row survives with its heading shortened too.
        let title = row_text(&buffer, area, area.y);
        assert!(title.contains('…'), "{title:?}");
    }

    #[test]
    fn playlists_are_grouped_with_headings() {
        let created = vec![playlist(1, "Rock", PlaylistKind::Created)];
        let collected = vec![playlist(2, "Post-rock", PlaylistKind::Collected)];
        let buffer = draw(ListView::Playlists {
            created: &created,
            collected: &collected,
            selected: 0,
            scroll: 0,
        });
        let text = text_of(&buffer, list_area());

        assert!(text.starts_with("PLAYLISTS"), "{text}");
        assert!(text.contains("CREATED"), "{text}");
        assert!(text.contains("COLLECTED"), "{text}");
        // The cursor sits on the selected playlist only.
        assert!(text.contains("▶ Rock"), "{text}");
        assert!(text.contains("  Post-rock"), "{text}");
    }

    #[test]
    fn the_selection_can_move_into_the_collected_group() {
        let created = vec![playlist(1, "Rock", PlaylistKind::Created)];
        let collected = vec![playlist(2, "Post-rock", PlaylistKind::Collected)];
        let buffer = draw(ListView::Playlists {
            created: &created,
            collected: &collected,
            selected: 1,
            scroll: 0,
        });
        let text = text_of(&buffer, list_area());
        assert!(text.contains("  Rock"), "{text}");
        assert!(text.contains("▶ Post-rock"), "{text}");
    }

    #[test]
    fn a_song_takes_identity_and_credit_rows() {
        let songs = vec![song(1, "Sunny Day")];
        let buffer = draw(ListView::Songs {
            playlist_name: "My list",
            songs: &songs,
            selected: 0,
            scroll: 0,
        });
        let text = text_of(&buffer, list_area());

        assert!(text.contains("CURRENT PLAYLIST  ‹ My list"), "{text}");
        assert!(text.contains("▶ 01  Sunny Day"), "{text}");
        assert!(text.contains("Artist · Album"), "{text}");
        assert!(text.contains("04:21"), "{text}");
    }

    #[test]
    fn an_empty_playlist_says_so_but_only_once_loading_finishes() {
        let songs: Vec<Song> = Vec::new();
        let idle = draw(ListView::Songs {
            playlist_name: "Empty",
            songs: &songs,
            selected: 0,
            scroll: 0,
        });
        assert!(compact(&text_of(&idle, list_area())).contains("该歌单暂无歌曲"));

        // While a request is in flight the pane stays blank rather than
        // claiming the list is empty.
        let loading = draw_in(
            ListView::Songs {
                playlist_name: "Empty",
                songs: &songs,
                selected: 0,
                scroll: 0,
            },
            true,
            Rect::new(0, 0, 120, 40),
        );
        let text = compact(&text_of(&loading, list_area()));
        assert!(!text.contains("该歌单暂无歌曲"), "{text}");
    }

    #[test]
    fn only_the_visible_window_is_drawn() {
        let songs: Vec<Song> = (0..500).map(|i| song(i, &format!("Song {i}"))).collect();
        let buffer = draw(ListView::Songs {
            playlist_name: "Big",
            songs: &songs,
            selected: 400,
            scroll: 400,
        });
        let text = text_of(&buffer, list_area());

        assert!(text.contains("Song 400"), "{text}");
        assert!(!text.contains("Song 0 "), "{text}");
        assert!(!text.contains("Song 499"), "{text}");
    }

    #[test]
    fn the_queue_lists_its_tracks_and_marks_the_one_playing() {
        let songs = vec![song(1, "Song A"), song(2, "Song B"), song(3, "Song C")];
        let buffer = draw(ListView::Queue {
            songs: &songs,
            playing: Some(0),
            selected: 1,
            scroll: 0,
        });
        let text = text_of(&buffer, list_area());

        assert!(text.starts_with("QUEUE  3"), "{text}");
        assert!(text.contains("01  Song A"), "{text}");
        assert!(text.contains("02  Song B"), "{text}");
        // The cursor and the track playing are marked separately: the user can
        // browse the queue while another track plays.
        assert!(text.contains("▶ 02  Song B"), "{text}");
        assert!(text.contains("  01  Song A"), "{text}");
        // Read through `compact`: the cell a wide glyph covers reads back as a
        // space, so the mark arrives with gaps between its characters.
        assert!(compact(&text).contains("正在播放"), "{text}");
    }

    #[test]
    fn nothing_is_marked_as_playing_when_the_track_is_not_in_the_queue() {
        // A track queued with "play next" is not in the list at all, so no row
        // may claim to be it.
        let songs = vec![song(1, "Song A")];
        let buffer = draw(ListView::Queue {
            songs: &songs,
            playing: None,
            selected: 0,
            scroll: 0,
        });
        assert!(!text_of(&buffer, list_area()).contains("正在播放"));
    }

    #[test]
    fn a_long_chinese_title_in_the_queue_keeps_the_mark_visible() {
        // The mark sits in the right-hand column, which must survive any title.
        let songs = vec![Song {
            id: SongId(9),
            title: "这是一首名字特别特别长的中文歌曲标题用来测试队列行的截断行为是否按列宽工作"
                .into(),
            artists: Vec::new(),
            album: None,
            duration_ms: None,
        }];
        let buffer = draw(ListView::Queue {
            songs: &songs,
            playing: Some(0),
            selected: 0,
            scroll: 0,
        });
        let area = list_area();

        for y in 0..area.height {
            assert_eq!(
                row_width(&buffer, area, y),
                area.width as usize,
                "row {y} does not span the pane"
            );
        }
        let row = row_text(&buffer, area, area.y + 2);
        assert!(row.ends_with("正在播放"), "{row:?}");
        assert!(row.contains('…'), "{row:?}");
    }

    #[test]
    fn an_empty_queue_says_so() {
        let songs: Vec<Song> = Vec::new();
        let buffer = draw(ListView::Queue {
            songs: &songs,
            playing: None,
            selected: 0,
            scroll: 0,
        });
        assert!(compact(&text_of(&buffer, list_area())).contains("队列为空"));
    }

    #[test]
    fn settings_show_their_current_values() {
        let buffer = draw(ListView::Settings {
            values: SettingsValues {
                quality: "exhigh",
                mode: "Sequential",
                volume: 72,
                cover_size: "normal",
            },
            selected: 1,
            editing: None,
        });
        let text = text_of(&buffer, list_area());

        assert!(text.contains("KEYBINDINGS"), "{text}");
        assert!(text.contains("AUDIO QUALITY"), "{text}");
        assert!(text.contains("exhigh"), "{text}");
        assert!(text.contains("Sequential"), "{text}");
        assert!(text.contains("72%"), "{text}");
        assert!(text.contains("▶ AUDIO QUALITY"), "{text}");
    }

    #[test]
    fn the_row_being_edited_says_so() {
        // The arrows change a value only while its row is being edited, and
        // nothing else on screen says which row that is.
        let values = |editing| ListView::Settings {
            values: SettingsValues {
                quality: "exhigh",
                mode: "Sequential",
                volume: 72,
                cover_size: "normal",
            },
            selected: 4,
            editing,
        };

        let text = text_of(&draw(values(None)), list_area());
        assert!(text.contains("normal"), "{text}");
        assert!(
            !text.contains("‹ normal ›"),
            "an unedited row is bracketed: {text}"
        );

        let text = text_of(&draw(values(Some(Setting::CoverSize))), list_area());
        assert!(text.contains("‹ normal ›"), "{text}");

        // And only that row, not the whole page.
        assert!(!text.contains("‹ exhigh ›"), "{text}");
    }

    #[test]
    fn every_setting_the_model_declares_is_drawn() {
        // The page is built from the settings' own list, so this is what keeps
        // the declaration and the drawing together.
        let buffer = draw(ListView::Settings {
            values: SettingsValues {
                quality: "exhigh",
                mode: "Sequential",
                volume: 72,
                cover_size: "normal",
            },
            selected: 0,
            editing: None,
        });
        let text = text_of(&buffer, list_area());
        for (_, label) in Setting::ALL {
            assert!(text.contains(label), "{label} is missing from the page");
        }
    }

    #[test]
    fn the_keybinding_list_names_every_action_and_its_key() {
        let buffer = draw(ListView::Keybindings {
            selected: 0,
            scroll: 0,
            capturing: false,
        });
        let text = text_of(&buffer, list_area());

        // The heading says where this is and how to leave it.
        assert!(text.contains("SETTINGS"), "{text}");
        assert!(text.contains("KEYBINDINGS"), "{text}");

        // Every action is listed with the key the keymap holds for it, so the
        // page cannot show a binding that would not fire.
        let keymap = Keymap::default();
        for action in Action::ALL {
            assert!(text.contains(action.name()), "{} is missing", action.name());
            assert!(
                text.contains(&keymap.key_for(action).unwrap().label()),
                "{} is listed without its key",
                action.name()
            );
        }
    }

    #[test]
    fn the_selected_keybinding_is_drawn_even_when_the_pane_is_short() {
        // There are 23 actions and the pane is 18 rows of list at the Minimal
        // tier, so the cursor has to be scrolled into view like any other list.
        // Left unscrolled, the highlight walks off the bottom of the pane and
        // takes the "waiting for a key" prompt with it — the user would then
        // rebind a row they cannot see.
        let area = Rect::new(0, 0, 80, 24);
        let last = Action::ALL.len() - 1;
        let scroll = last - 5;
        let buffer = draw_in(
            ListView::Keybindings {
                selected: last,
                scroll,
                capturing: true,
            },
            false,
            area,
        );

        let rows: Vec<String> = (area.y..area.y + area.height)
            .map(|y| row_text(&buffer, area, y))
            .collect();
        let cursor_row = rows
            .iter()
            .find(|row| row.contains(CURSOR))
            .unwrap_or_else(|| panic!("the cursor is not on screen: {rows:#?}"));
        assert!(
            cursor_row.contains("按下新按键"),
            "the row being rebound is drawn without saying so: {cursor_row:?}"
        );
        // The action it belongs to is there with it.
        assert!(
            cursor_row.contains(Action::ALL[last].name()),
            "{cursor_row:?}"
        );
        // And the rows above the window are not drawn at all.
        assert!(
            !rows.iter().any(|row| row.contains(Action::ALL[0].name())),
            "the list was drawn from the top instead of from the window"
        );
    }

    #[test]
    fn a_row_waiting_for_a_key_says_so_instead_of_its_old_one() {
        let area = list_area();
        let buffer = draw(ListView::Keybindings {
            selected: 0,
            scroll: 0,
            capturing: true,
        });

        // The selected row is the first entry, which is two rows below the
        // heading and its rule.
        let selected_row = row_text(&buffer, area, area.y + 2);
        assert!(selected_row.contains("按下新按键"), "{selected_row:?}");
        // Its own key is replaced rather than shown beside the prompt, which
        // would read as though it were still bound.
        assert!(!selected_row.trim_end().ends_with("1"), "{selected_row:?}");
        let prompts: usize = (area.y..area.y + area.height)
            .filter(|y| row_text(&buffer, area, *y).contains("按下新按键"))
            .count();
        assert_eq!(prompts, 1, "more than the row being rebound is waiting");
    }

    #[test]
    fn a_refused_rebind_says_why() {
        // The reason a binding was refused is a message for the status bar,
        // where every other message this program has for the user goes: a list
        // can be longer than the pane, and an explanation at the bottom of one
        // would be the first thing scrolled out of sight.
        let area = list_area();
        let buffer = draw(ListView::Keybindings {
            selected: 3,
            scroll: 0,
            capturing: false,
        });
        let longest = (area.y..area.y + area.height)
            .map(|y| row_width(&buffer, area, y))
            .max()
            .unwrap();
        assert!(longest <= area.width as usize, "a row overhangs the pane");
        // Nothing about conflicts belongs in the list itself.
        assert!(!text_of(&buffer, area).contains("冲突"));
    }

    #[test]
    fn the_title_is_separated_from_the_rows_by_a_rule() {
        let songs = vec![song(1, "Sunny Day")];
        let buffer = draw(ListView::Songs {
            playlist_name: "My list",
            songs: &songs,
            selected: 0,
            scroll: 0,
        });
        let area = list_area();
        let rule_row: String = (0..area.width)
            .map(|x| buffer.cell((x, area.y + 1)).unwrap().symbol())
            .collect();
        assert!(rule_row.starts_with("─────"), "{rule_row}");
        assert!(rule_row.chars().all(|c| c == '─'), "{rule_row}");
    }

    #[test]
    fn a_message_list_renders_the_message() {
        let buffer = draw(ListView::Message("加载失败: 网络错误"));
        assert!(compact(&text_of(&buffer, list_area())).contains("加载失败:网络错误"));
    }

    #[test]
    fn a_playlist_view_with_no_groups_still_titles_itself() {
        let buffer = draw(ListView::Playlists {
            created: &[],
            collected: &[],
            selected: 0,
            scroll: 0,
        });
        assert!(text_of(&buffer, list_area()).contains("PLAYLISTS"));
    }

    #[test]
    fn the_login_view_takes_over_the_whole_screen() {
        let area = Rect::new(0, 0, 120, 40);
        let grid = crate::qr::encode("https://example.invalid/x").unwrap();
        let keymap = Keymap::default();
        let state = UiState {
            page: Page::Playlist,
            session: SessionView::Login(LoginView::Waiting {
                grid: &grid,
                scanned: false,
            }),
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
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, &crate::theme::Theme::default()))
            .unwrap();
        let text = compact(&text_of(terminal.backend().buffer(), area));

        assert!(text.contains("扫码登录"), "{text}");
        // The navigation row is gone: sign-in owns the screen.
        assert!(!text.contains("PLAYLIST ["), "{text}");
    }
}
