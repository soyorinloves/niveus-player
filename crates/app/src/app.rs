//! The event loop and the terminal's lifecycle.

use crate::event::AppEvent;
use crate::paths::Paths;
use crate::persistence::StateFile;
use crate::state::AppState;
use niveus_config::Config;
use niveus_mpris::MprisService;
use niveus_ncm_provider::NcmProvider;
use niveus_ui::Theme;
use ratatui::layout::Rect;
use ratatui::DefaultTerminal;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// How often the loop is nudged, so transient messages can expire.
const TICK: Duration = Duration::from_millis(250);

/// Runs the interface until the user leaves.
pub async fn run(paths: Paths, config: Config, warnings: Vec<String>) -> anyhow::Result<()> {
    let provider = Arc::new(match read_cookie(&paths) {
        Some(cookie) => NcmProvider::with_cookie(cookie),
        None => NcmProvider::anonymous(),
    });

    let (tx, mut rx) = mpsc::channel(64);
    let mut state = AppState::new(paths, config, provider, warnings);

    // What the last session left behind is taken on before the player starts,
    // so the volume it is given is the volume the interface shows (§29).
    let (restored, error) = StateFile::load(&state.paths.state_file());
    if let Some(error) = error {
        tracing::warn!(%error, "cannot use the saved state, starting fresh");
        state.warn(format!("无法恢复上次的状态：{error}"));
    }
    state.restore(restored);
    state.refresh_artwork(&tx);

    spawn_player(&mut state, &tx).await;
    spawn_mpris(&mut state, &tx).await;

    spawn_input(tx.clone());
    spawn_ticks(tx.clone());
    spawn_signals(tx.clone());

    let mut terminal = ratatui::try_init().map_err(|error| {
        // The usual cause is being started without a terminal attached — from a
        // pipe, a service, or a desktop launcher. The raw OS error says nothing
        // useful to a person, so it is explained instead.
        anyhow::anyhow!(
            "无法初始化终端：{error}。\
             请在终端中直接运行 niveus；若从脚本调用，请为其分配一个伪终端（pty）。"
        )
    })?;
    install_panic_hook();

    state.request_session(&tx);

    let outcome = event_loop(&mut terminal, &mut state, &mut rx, &tx).await;

    // The state is written before the player is stopped, while the position it
    // was at is still the position the player is reporting (§34).
    state.persist_state();

    // Whatever happened above, stop the player, leave the terminal usable and
    // the socket gone.
    if let Some(player) = state.player.take() {
        player.shutdown().await;
    }
    ratatui::restore();
    remove_socket(&state.paths);

    outcome
}

/// Puts Niveus on the session bus, so the desktop can see and drive it.
///
/// A failure here is not fatal and not unusual — a container or a serial
/// console has no session bus, and the program is expected to run without one.
/// What the desktop would have shown is lost, so the user is told.
async fn spawn_mpris(state: &mut AppState, tx: &mpsc::Sender<AppEvent>) {
    let (commands, mut inbox) = mpsc::unbounded_channel();
    match MprisService::start(commands).await {
        Ok(service) => {
            state.mpris = Some(service);
        }
        Err(error) => {
            tracing::warn!(%error, "no desktop integration");
            state.notify(format!("桌面集成不可用：{error}"));
            return;
        }
    }

    // What the desktop asks for arrives on its own channel and joins the one
    // the rest of the program listens on.
    let tx = tx.clone();
    tokio::spawn(async move {
        while let Some(command) = inbox.recv().await {
            if tx.send(AppEvent::Mpris(command)).await.is_err() {
                return;
            }
        }
    });
}

/// Starts the player and pipes what it reports into the event loop.
///
/// A player that will not start is not fatal: browsing works without one, and
/// saying so is more useful than refusing to run.
async fn spawn_player(state: &mut AppState, tx: &mpsc::Sender<AppEvent>) {
    // The same call the recovery path makes when the player dies mid-session,
    // so the two cannot start it differently.
    match state.launch_player(tx).await {
        Ok(()) => tracing::info!("player started"),
        Err(error) => {
            tracing::error!(%error, "cannot start the player");
            state.notify(format!("无法启动播放器：{error}"));
        }
    }
}

async fn event_loop(
    terminal: &mut DefaultTerminal,
    state: &mut AppState,
    rx: &mut mpsc::Receiver<AppEvent>,
    tx: &mpsc::Sender<AppEvent>,
) -> anyhow::Result<()> {
    let theme = Theme::default();

    loop {
        // The frame's own area is authoritative: it is the buffer actually
        // being drawn, which may differ briefly from a reported resize.
        let mut area = Rect::ZERO;
        terminal.draw(|frame| {
            area = frame.area();
            niveus_ui::render(frame, &state.ui_state(), &theme);
        })?;
        state.size = (area.width, area.height);

        let Some(event) = rx.recv().await else {
            // Every sender is gone, which only happens on teardown.
            break;
        };
        state.handle_event(event, tx).await;

        if state.quit {
            break;
        }
    }
    Ok(())
}

/// Reads and sends terminal input.
///
/// A plain thread rather than a task: `crossterm::event::read` blocks, and
/// parking a runtime worker on it would starve the network tasks.
fn spawn_input(tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || loop {
        match crossterm::event::read() {
            Ok(crossterm::event::Event::Key(key)) => {
                // Only presses: crossterm reports releases separately on
                // terminals that support them, and acting twice would be wrong.
                if key.kind == crossterm::event::KeyEventKind::Press
                    && tx.blocking_send(AppEvent::Key(key)).is_err()
                {
                    return;
                }
            }
            Ok(crossterm::event::Event::Resize(..)) => {
                if tx.blocking_send(AppEvent::Resize).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "terminal input ended");
                return;
            }
        }
    });
}

fn spawn_ticks(tx: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        loop {
            interval.tick().await;
            if tx.send(AppEvent::Tick).await.is_err() {
                return;
            }
        }
    });
}

/// Turns SIGINT and SIGTERM into an ordinary quit.
fn spawn_signals(tx: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(stream) => stream,
                Err(error) => {
                    tracing::warn!(%error, "cannot listen for SIGTERM");
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        let _ = tx.send(AppEvent::Interrupt).await;
    });
}

/// Restores the terminal even if the program panics.
///
/// Without this a panic leaves the shell in raw mode with no echo, which for the
/// user looks like the terminal itself has broken.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::try_restore().ok();
        previous(info);
    }));
}

fn read_cookie(paths: &Paths) -> Option<String> {
    let path = paths.cookie_file();
    match std::fs::read_to_string(&path) {
        Ok(cookie) if !cookie.trim().is_empty() => {
            tracing::info!(path = %path.display(), "restored session cookie");
            Some(cookie.trim().to_string())
        }
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cannot read session cookie");
            None
        }
    }
}

/// Removes the player's socket, if one was left behind.
fn remove_socket(paths: &Paths) {
    let socket = paths.socket_file();
    if socket.exists() {
        if let Err(error) = std::fs::remove_file(&socket) {
            tracing::warn!(path = %socket.display(), %error, "cannot remove socket");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_cookie_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("niveus-cookie-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let paths = test_paths(&dir);
        assert_eq!(read_cookie(&paths), None);

        std::fs::write(paths.cookie_file(), "  MUSIC_U=abc  ").unwrap();
        assert_eq!(read_cookie(&paths), Some("MUSIC_U=abc".to_string()));

        // A blank file is treated as no session rather than an empty one.
        std::fs::write(paths.cookie_file(), "   \n").unwrap();
        assert_eq!(read_cookie(&paths), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn removing_a_socket_that_is_not_there_is_harmless() {
        let dir = std::env::temp_dir().join(format!("niveus-socket-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let paths = test_paths(&dir);
        // The first call runs with nothing there at all.
        remove_socket(&paths);

        std::fs::create_dir_all(paths.runtime_dir()).unwrap();
        std::fs::write(paths.socket_file(), "").unwrap();
        remove_socket(&paths);
        assert!(!paths.socket_file().exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Redirects every file location into the test's own directory.
    fn test_paths(dir: &std::path::Path) -> Paths {
        Paths::new(dir.join("config"), dir.to_path_buf(), dir.join("run"))
    }
}
