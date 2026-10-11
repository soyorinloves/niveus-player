//! The player facade.
//!
//! Everything below this module — the process, the socket, the wire protocol —
//! is an implementation detail. Callers launch a player, ask it to play, pause,
//! seek or change the volume, and read events off the channel they were given.

use crate::error::PlayerError;
use crate::events::PlayerEvent;
use crate::ipc;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// How long to wait for the player to come up before giving up on it.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
/// How often to retry while waiting.
const STARTUP_RETRY: Duration = Duration::from_millis(100);
/// How long a departing player gets to leave on its own before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(1500);

/// A `sockaddr_un` path, including its terminating NUL, must fit in 108 bytes.
const MAX_SOCKET_PATH: usize = 107;

/// How the player should be started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    /// Play without producing sound.
    ///
    /// For headless runs and tests: the track is decoded and runs to its end
    /// exactly as it would otherwise, it simply never reaches a speaker.
    pub silent: bool,
    /// Volume to start at, 0–100.
    pub volume: Option<u8>,
}

/// Control over one player process.
pub struct Player {
    requests: mpsc::Sender<ipc::Request>,
    session: JoinHandle<()>,
    /// Present when the player was started here, and so has to be stopped here.
    child: Option<Child>,
    socket: Option<PathBuf>,
}

impl Player {
    /// Starts a player and connects to it.
    ///
    /// Any command needs the process to be up and listening, so this does not
    /// return until the connection is established.
    pub async fn launch(
        socket: &Path,
        options: LaunchOptions,
        events: mpsc::Sender<PlayerEvent>,
    ) -> Result<Self, PlayerError> {
        let program = player_path().ok_or(PlayerError::MpvNotFound)?;

        if socket.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(PlayerError::IpcProtocol(format!(
                "the runtime directory path is too long to create the player channel: {}",
                socket.display()
            )));
        }

        // A socket left behind by a previous run would be in the way: the
        // player refuses to start rather than taking over someone else's. If it
        // cannot be removed the start-up will fail, and a log line here is what
        // connects the two.
        if socket.exists() {
            if let Err(error) = std::fs::remove_file(socket) {
                tracing::warn!(
                    path = %socket.display(), %error,
                    "cannot remove a socket left by a previous run"
                );
            }
        }

        let mut command = Command::new(&program);
        command
            .args(arguments(socket, &options))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // A player must not outlive the program that started it, however
            // the program ends.
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(PlayerError::Spawn)?;

        let stream = match connect_within(socket, &mut child).await {
            Ok(stream) => stream,
            Err(error) => {
                let _ = child.kill().await;
                return Err(error);
            }
        };

        Ok(Self::attach(
            stream,
            events,
            Some(child),
            Some(socket.to_path_buf()),
        ))
    }

    /// Connects to a player that is already running.
    ///
    /// Used to reattach to a player started elsewhere; the caller does not own
    /// the process and this facade will not stop it.
    pub async fn connect(
        socket: &Path,
        events: mpsc::Sender<PlayerEvent>,
    ) -> Result<Self, PlayerError> {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(PlayerError::IpcConnection)?;
        Ok(Self::attach(stream, events, None, None))
    }

    fn attach(
        stream: UnixStream,
        events: mpsc::Sender<PlayerEvent>,
        child: Option<Child>,
        socket: Option<PathBuf>,
    ) -> Self {
        let (requests, inbox) = mpsc::channel(32);
        let session = ipc::spawn(stream, inbox, events);
        Self {
            requests,
            session,
            child,
            socket,
        }
    }

    /// Loads `url` and starts playing it, replacing whatever was playing.
    ///
    /// The player keeps its settings across a change of file, and whether it
    /// was paused is one of them: a track chosen while the previous one was
    /// paused would load, sit there, and never make a sound. Asking for a track
    /// is asking to hear it, so the pause is cleared as part of the same
    /// instruction.
    pub async fn play(&self, url: &str) -> Result<(), PlayerError> {
        self.send(ipc::command("loadfile", [json!(url), json!("replace")]))
            .await?;
        self.resume().await
    }

    pub async fn pause(&self) -> Result<(), PlayerError> {
        self.send(ipc::command("set_property", [json!("pause"), json!(true)]))
            .await
    }

    pub async fn resume(&self) -> Result<(), PlayerError> {
        self.send(ipc::command("set_property", [json!("pause"), json!(false)]))
            .await
    }

    /// Toggles playback, whichever way it is currently going.
    ///
    /// Asking the player to toggle rather than computing the new state from a
    /// remembered one keeps a stale view from flipping it the wrong way.
    pub async fn toggle_pause(&self) -> Result<(), PlayerError> {
        self.send(ipc::command("cycle", [json!("pause")])).await
    }

    /// Stops playback without unloading the track.
    pub async fn stop(&self) -> Result<(), PlayerError> {
        self.send(ipc::command("stop", [])).await
    }

    /// Seeks to an absolute position, in seconds.
    pub async fn seek_to(&self, seconds: f64) -> Result<(), PlayerError> {
        let seconds = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        self.send(ipc::command("seek", [json!(seconds), json!("absolute")]))
            .await
    }

    /// Moves the position by `seconds`, forwards or backwards.
    pub async fn seek_by(&self, seconds: f64) -> Result<(), PlayerError> {
        let seconds = if seconds.is_finite() { seconds } else { 0.0 };
        self.send(ipc::command("seek", [json!(seconds), json!("relative")]))
            .await
    }

    /// Sets the output volume, 0–100.
    pub async fn set_volume(&self, percent: u8) -> Result<(), PlayerError> {
        self.send(ipc::command(
            "set_property",
            [json!("volume"), json!(percent.min(100))],
        ))
        .await
    }

    /// Stops the player and cleans up after it.
    ///
    /// Every step is best effort: the interface must be able to leave even when
    /// the player has already gone, the socket has already been removed, or the
    /// process refuses to answer.
    pub async fn shutdown(mut self) {
        // Ask politely first, so the player can flush and remove its own socket.
        let _ = tokio::time::timeout(
            Duration::from_millis(500),
            ipc::request(&self.requests, ipc::command("quit", [])),
        )
        .await;
        let _ = self.requests.send(ipc::Request::Close).await;

        if let Some(child) = self.child.as_mut() {
            if tokio::time::timeout(SHUTDOWN_GRACE, child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }

        // The session ends on its own once the connection closes; it is not
        // worth blocking the exit on.
        self.session.abort();
        if let Some(socket) = &self.socket {
            let _ = std::fs::remove_file(socket);
        }
    }

    async fn send(&self, args: Vec<serde_json::Value>) -> Result<(), PlayerError> {
        ipc::request(&self.requests, args).await
    }
}

/// The arguments the player is started with.
///
/// Kept apart from starting it so the settings playback depends on can be
/// asserted directly, rather than inferred from what a running player does.
fn arguments(socket: &Path, options: &LaunchOptions) -> Vec<String> {
    let mut args = vec![
        // The user's own player configuration is deliberately not loaded: a
        // personal `loop`, `keep-open` or saved playback position would break
        // playback in ways that are invisible here and baffling to diagnose —
        // a looping track, for one, never reports its end, so the queue would
        // stop advancing. Niveus controls playback; the player only performs it.
        "--no-config".to_string(),
        "--idle=yes".to_string(),
        "--no-video".to_string(),
        // The player must never touch the terminal the interface is using.
        "--no-terminal".to_string(),
        format!("--input-ipc-server={}", socket.display()),
        // The application owns the volume, so the player's own ceiling must not
        // silently disagree with it.
        "--volume-max=100".to_string(),
        // Restated even without a configuration file, because these are the
        // settings the queue's correctness rests on.
        "--loop=no".to_string(),
        "--loop-file=no".to_string(),
        "--keep-open=no".to_string(),
        "--no-resume-playback".to_string(),
    ];
    if options.silent {
        args.push("--ao=null".to_string());
    }
    if let Some(volume) = options.volume {
        args.push(format!("--volume={}", volume.min(100)));
    }
    args
}

/// Waits for the player to create its socket and accept a connection.
async fn connect_within(socket: &Path, child: &mut Child) -> Result<UnixStream, PlayerError> {
    let attempts = (STARTUP_TIMEOUT.as_millis() / STARTUP_RETRY.as_millis()).max(1);

    for _ in 0..attempts {
        match UnixStream::connect(socket).await {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                // A player that has already exited will never listen, and
                // retrying for the full timeout would just delay the report.
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(PlayerError::IpcConnection(std::io::Error::other(format!(
                        "the player process exited ({status})"
                    ))));
                }
                if !matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) {
                    return Err(PlayerError::IpcConnection(error));
                }
            }
        }
        tokio::time::sleep(STARTUP_RETRY).await;
    }

    Err(PlayerError::IpcConnection(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "the player did not start within the timeout",
    )))
}

/// The player binary, looked up on `PATH`.
///
/// Deliberately not `/usr/bin/mpv`: the distribution decides where its packages
/// live, and a user may well run a locally built player.
pub fn player_path() -> Option<PathBuf> {
    search_path(std::env::var_os("PATH")?, "mpv")
}

fn search_path(path: std::ffi::OsString, name: &str) -> Option<PathBuf> {
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory holding a fake player, for exercising the lookup.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("niveus-player-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_player_is_looked_up_on_the_path() {
        use std::os::unix::fs::PermissionsExt;

        let first = scratch("first");
        let second = scratch("second");
        let binary = second.join("mpv");
        std::fs::write(&binary, "x").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();

        // A directory earlier on PATH that does not hold the player is searched
        // through, and the one that does is the answer.
        let path = std::env::join_paths([&first, &second]).unwrap();
        assert_eq!(search_path(path, "mpv"), Some(binary.clone()));

        // Without it, nothing is found — there is no fallback path.
        let path = std::env::join_paths([&first]).unwrap();
        assert_eq!(search_path(path, "mpv"), None);

        std::fs::remove_dir_all(&first).unwrap();
        std::fs::remove_dir_all(&second).unwrap();
    }

    #[test]
    fn the_players_own_configuration_is_not_loaded() {
        let args = arguments(
            Path::new("/run/user/1000/niveus-mpv.sock"),
            &LaunchOptions::default(),
        );
        assert!(args.iter().any(|a| a == "--no-config"), "{args:?}");

        // A personal `loop` setting would mean the track never ends, and the
        // queue would never advance. These are stated outright rather than left
        // to the player's defaults.
        for setting in [
            "--loop=no",
            "--loop-file=no",
            "--keep-open=no",
            "--no-resume-playback",
        ] {
            assert!(
                args.iter().any(|a| a == setting),
                "{setting} missing: {args:?}"
            );
        }
    }

    #[test]
    fn the_socket_and_the_optionals_are_passed_through() {
        let options = LaunchOptions {
            silent: true,
            volume: Some(42),
        };
        let args = arguments(Path::new("/tmp/niveus-mpv.sock"), &options);

        assert!(args.contains(&"--input-ipc-server=/tmp/niveus-mpv.sock".to_string()));
        assert!(args.contains(&"--idle=yes".to_string()));
        assert!(args.contains(&"--no-video".to_string()));
        assert!(args.contains(&"--no-terminal".to_string()));
        assert!(args.contains(&"--ao=null".to_string()));
        assert!(args.contains(&"--volume=42".to_string()));

        // Nothing is asked for silently when it was not requested.
        let plain = arguments(Path::new("/tmp/x.sock"), &LaunchOptions::default());
        assert!(!plain.iter().any(|a| a.starts_with("--ao=")), "{plain:?}");
        assert!(
            !plain.iter().any(|a| a.starts_with("--volume=")),
            "{plain:?}"
        );
    }

    #[test]
    fn a_file_that_cannot_be_run_is_not_the_player() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("unrunnable");
        let binary = dir.join("mpv");
        std::fs::write(&binary, "x").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o644)).unwrap();

        let path = std::env::join_paths([&dir]).unwrap();
        assert_eq!(search_path(path, "mpv"), None);

        // As is a directory that happens to carry the name.
        std::fs::remove_file(&binary).unwrap();
        std::fs::create_dir(binary).unwrap();
        let path = std::env::join_paths([&dir]).unwrap();
        assert_eq!(search_path(path, "mpv"), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
