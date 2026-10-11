//! The sign-in screen: a QR code drawn with ordinary characters.
//!
//! The code is painted with an explicit black foreground and white background
//! rather than left to the terminal's default colours, so it stays scannable on
//! both dark and light themes and regardless of how the user's palette maps the
//! standard colours.

use super::{line_of, truncate};
use crate::qr::QrGrid;
use crate::state::{LoginView, SessionView, UiState};
use crate::theme::Theme;
use niveus_config::Action;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Light modules are painted, not left transparent.
const LIGHT: Color = Color::White;
/// Dark modules.
const DARK: Color = Color::Black;
/// Carries one module above and one below.
const HALF_BLOCK: char = '▀';

pub(crate) fn render(frame: &mut Frame, area: Rect, state: &UiState, theme: &Theme) {
    let SessionView::Login(login) = &state.session else {
        return;
    };
    let lines = compose(login, state, area.width, area.height, theme);
    frame.render_widget(Paragraph::new(lines).centered(), area);
}

/// Builds the screen, shedding optional lines until it fits.
///
/// A clipped QR is worse than useless, so the code and the line of text that
/// belongs to it are kept and everything else gives way first. If even those do
/// not fit, the user is told rather than shown a broken code.
fn compose(
    login: &LoginView<'_>,
    state: &UiState,
    width: u16,
    height: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let capacity = height as usize;

    for (instruction, hint) in [(true, true), (true, false), (false, false)] {
        let lines = build(login, state, width, instruction, hint, theme);
        if lines.len() <= capacity {
            return lines;
        }
    }

    vec![
        Line::from(Span::styled("扫码登录", theme.heading)),
        Line::default(),
        Line::from(Span::styled(
            "终端太小，无法完整显示二维码，请放大窗口",
            theme.error,
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("{} 退出", state.hint(Action::Quit)),
            theme.muted,
        )),
    ]
}

fn build(
    login: &LoginView<'_>,
    state: &UiState,
    width: u16,
    instruction: bool,
    hint: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    // Nothing is being scanned yet while the stored session is checked, so the
    // page does not offer a code that has not been asked for.
    if let LoginView::Checking = login {
        let mut lines = vec![
            Line::from(Span::styled("登录", theme.heading)),
            Line::default(),
        ];
        lines.push(Line::from(Span::styled("正在检查登录状态…", theme.muted)));
        if hint {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                format!("{} 退出", state.hint(Action::Quit)),
                theme.muted,
            )));
        }
        return lines;
    }

    let mut lines = vec![Line::from(Span::styled("扫码登录", theme.heading))];

    if instruction {
        lines.push(Line::from(Span::styled(
            "用网易云音乐 App 扫描二维码",
            theme.muted,
        )));
    }

    match login {
        // Handled above, before the heading that only the code page carries.
        LoginView::Checking => {}
        LoginView::Starting => {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled("正在获取二维码…", theme.muted)));
        }
        LoginView::Waiting { grid, scanned } => {
            lines.extend(grid_lines(grid));
            lines.push(Line::from(Span::styled(
                if *scanned {
                    "已扫码，请在手机上确认"
                } else {
                    "等待扫码…"
                },
                if *scanned { theme.accent } else { theme.muted },
            )));
        }
        LoginView::Failed { message } => {
            lines.push(Line::default());
            lines.push(line_of(truncate(message, width as usize), theme.error));
        }
    }

    if hint {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!("{} 退出", state.hint(Action::Quit)),
            theme.muted,
        )));
    }

    lines
}

/// One terminal row per two module rows, sharing a cell via the upper
/// half-block: the foreground paints the module above, the background the one
/// below.
fn grid_lines(grid: &QrGrid) -> Vec<Line<'static>> {
    (0..grid.cell_height())
        .map(|row| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut run_style: Option<Style> = None;
            let mut run_len = 0usize;

            for x in 0..grid.size() {
                let style = cell_style(grid.is_dark(x, row * 2), grid.is_dark(x, row * 2 + 1));
                if run_style == Some(style) {
                    run_len += 1;
                } else {
                    if let Some(previous) = run_style.take() {
                        spans.push(Span::styled(
                            HALF_BLOCK.to_string().repeat(run_len),
                            previous,
                        ));
                    }
                    run_style = Some(style);
                    run_len = 1;
                }
            }
            if let Some(previous) = run_style {
                spans.push(Span::styled(
                    HALF_BLOCK.to_string().repeat(run_len),
                    previous,
                ));
            }
            Line::from(spans)
        })
        .collect()
}

fn cell_style(top_dark: bool, bottom_dark: bool) -> Style {
    Style::default()
        .fg(if top_dark { DARK } else { LIGHT })
        .bg(if bottom_dark { DARK } else { LIGHT })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::LayoutTier;
    use crate::render::render;
    use crate::state::{ListView, LyricsView, NowPlayingView, Page, StatusView};
    use niveus_config::{CoverSize, Keymap};
    use niveus_music_core::{PlaybackMode, PlaybackStatus};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    const PAYLOAD: &str =
        "https://music.163.com/login?codekey=899d9f4c-c55f-4a01-a10d-6229f4570edb";

    fn state<'a>(login: LoginView<'a>, keymap: &'a Keymap) -> UiState<'a> {
        UiState {
            page: Page::Playlist,
            session: SessionView::Login(login),
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

    fn draw(login: LoginView<'_>, area: Rect) -> Buffer {
        let keymap = Keymap::default();
        let state = state(login, &keymap);
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
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn lines_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn checking_the_stored_session_does_not_promise_a_code() {
        // Saying a code is being fetched while the stored cookie is still being
        // checked describes something that is not happening.
        let area = Rect::new(0, 0, 100, 30);
        let buffer = draw(LoginView::Checking, area);
        let text = compact(&text_of(&buffer, area));

        assert!(text.contains("正在检查登录状态"), "{text}");
        assert!(!text.contains("二维码"), "{text}");
        assert!(!text.contains("扫码登录"), "{text}");
    }

    #[test]
    fn the_code_is_drawn_with_painted_cells() {
        let area = Rect::new(0, 0, 120, 40);
        let grid = crate::qr::encode(PAYLOAD).unwrap();
        let buffer = draw(
            LoginView::Waiting {
                grid: &grid,
                scanned: false,
            },
            area,
        );

        let mut saw_light = false;
        let mut saw_dark = false;
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = buffer.cell((x, y)).unwrap();
                if cell.symbol() == HALF_BLOCK.to_string() {
                    match cell.fg {
                        LIGHT => saw_light = true,
                        DARK => saw_dark = true,
                        _ => {}
                    }
                }
            }
        }
        assert!(saw_light && saw_dark, "the code did not use both colours");
    }

    #[test]
    fn the_instructions_and_state_are_shown() {
        let area = Rect::new(0, 0, 120, 40);
        let grid = crate::qr::encode(PAYLOAD).unwrap();
        let text = compact(&text_of(
            &draw(
                LoginView::Waiting {
                    grid: &grid,
                    scanned: false,
                },
                area,
            ),
            area,
        ));
        assert!(text.contains("扫码登录"), "{text}");
        assert!(text.contains("等待扫码…"), "{text}");

        let text = compact(&text_of(
            &draw(
                LoginView::Waiting {
                    grid: &grid,
                    scanned: true,
                },
                area,
            ),
            area,
        ));
        assert!(text.contains("已扫码，请在手机上确认"), "{text}");
    }

    #[test]
    fn a_failure_is_reported_with_a_way_out() {
        let area = Rect::new(0, 0, 120, 40);
        let text = compact(&text_of(
            &draw(
                LoginView::Failed {
                    message: "网络请求失败",
                },
                area,
            ),
            area,
        ));
        assert!(text.contains("网络请求失败"), "{text}");
        assert!(text.contains("[Q]退出"), "{text}");
    }

    #[test]
    fn the_starting_state_says_what_is_happening() {
        let area = Rect::new(0, 0, 120, 40);
        let text = compact(&text_of(&draw(LoginView::Starting, area), area));
        assert!(text.contains("正在获取二维码…"), "{text}");
    }

    #[test]
    fn the_code_and_its_status_survive_the_smallest_terminal() {
        // The tightest layout the app supports: the code must still be whole.
        let height = LayoutTier::MIN_HEIGHT;
        let width = LayoutTier::MIN_WIDTH;
        let grid = crate::qr::encode(PAYLOAD).unwrap();
        let keymap = Keymap::default();
        let holder = state(LoginView::Starting, &keymap);
        let lines = compose(
            &LoginView::Waiting {
                grid: &grid,
                scanned: false,
            },
            &holder,
            width,
            height,
            &crate::theme::Theme::default(),
        );

        assert!(
            lines.len() <= height as usize,
            "sign-in needs {} rows, only {height} available",
            lines.len()
        );
        // The code is present in full, not clipped.
        let code_rows = lines
            .iter()
            .filter(|line| line.spans.iter().any(|s| s.content.contains(HALF_BLOCK)))
            .count();
        assert_eq!(code_rows, grid.cell_height());
    }

    #[test]
    fn the_code_fits_the_narrowest_supported_width() {
        let grid = crate::qr::encode(PAYLOAD).unwrap();
        let rows = grid_lines(&grid);
        let widest = rows
            .iter()
            .map(|line| line.width())
            .max()
            .unwrap_or_default();
        assert!(
            widest <= LayoutTier::MIN_WIDTH as usize,
            "code is {widest} columns wide"
        );
    }

    #[test]
    fn every_row_of_the_sign_in_screen_stays_within_it() {
        // The screen is mostly Chinese, and it is drawn centred, so a row that
        // overran would be pushed off one side rather than simply clipped.
        let area = Rect::new(0, 0, 120, 40);
        let grid = crate::qr::encode(PAYLOAD).unwrap();
        for login in [
            LoginView::Starting,
            LoginView::Waiting {
                grid: &grid,
                scanned: false,
            },
            LoginView::Failed {
                message: "无法校验登录状态：网络请求失败，请检查网络连接后重试",
            },
        ] {
            let buffer = draw(login, area);
            for y in 0..area.height {
                assert_eq!(
                    crate::testing::row_width(&buffer, area, y),
                    area.width as usize,
                    "row {y} does not span the screen"
                );
            }
        }
    }

    #[test]
    fn a_long_failure_message_is_shortened_rather_than_cut() {
        let area = Rect::new(0, 0, 80, 24);
        let keymap = Keymap::default();
        let holder = state(LoginView::Starting, &keymap);
        let lines = compose(
            &LoginView::Failed {
                message: "无法校验登录状态：网络请求失败，请检查网络连接后重试，若问题持续存在请查看日志文件以获取更多信息",
            },
            &holder,
            area.width,
            area.height,
            &crate::theme::Theme::default(),
        );
        let text = lines_text(&lines);

        assert!(text.contains("无法校验登录状态"), "{text}");
        assert!(
            text.contains('…'),
            "the message was cut without a marker: {text}"
        );
    }

    #[test]
    fn a_terminal_too_short_for_the_code_says_so() {
        let width = LayoutTier::MIN_WIDTH;
        let grid = crate::qr::encode(PAYLOAD).unwrap();
        let keymap = Keymap::default();
        let holder = state(LoginView::Starting, &keymap);
        let lines = compose(
            &LoginView::Waiting {
                grid: &grid,
                scanned: false,
            },
            &holder,
            width,
            5,
            &crate::theme::Theme::default(),
        );
        let text = lines_text(&lines);
        assert!(text.contains("终端太小"), "{text}");
        assert!(
            !text.contains(HALF_BLOCK),
            "a clipped code was drawn anyway"
        );
    }
}
