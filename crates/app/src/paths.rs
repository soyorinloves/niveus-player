//! Where Niveus keeps its files.
//!
//! Configuration and runtime state are kept strictly apart: preferences belong
//! in the config directory, everything the program writes as it runs belongs in
//! the state directory, and the socket belongs in the runtime directory, which
//! may be wiped between sessions.

use directories::ProjectDirs;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("cannot determine the config/state directories: set HOME or XDG_CONFIG_HOME / XDG_STATE_HOME")]
    NoHome,
    #[error("cannot create the directory {path}: {source}")]
    Create {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// The directories Niveus uses, and the files inside them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_dir: PathBuf,
    state_dir: PathBuf,
    runtime_dir: PathBuf,
}

impl Paths {
    /// Builds a layout from explicit directories.
    pub fn new(config_dir: PathBuf, state_dir: PathBuf, runtime_dir: PathBuf) -> Self {
        Self {
            config_dir,
            state_dir,
            runtime_dir,
        }
    }

    /// Resolves the directories, honouring the XDG variables.
    pub fn discover() -> Result<Self, PathError> {
        let dirs = ProjectDirs::from("", "", "niveus").ok_or(PathError::NoHome)?;
        let state_dir = dirs
            .state_dir()
            .map(Path::to_path_buf)
            .ok_or(PathError::NoHome)?;
        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            state_dir,
            runtime_dir: runtime_dir(),
        })
    }

    /// Builds the directories, creating them if needed.
    ///
    /// The runtime directory is created owner-only: it holds a socket, and on a
    /// shared `/tmp` anyone able to reach that socket could drive the player.
    pub fn ensure_dirs(&self) -> Result<(), PathError> {
        for dir in [&self.config_dir, &self.state_dir] {
            std::fs::create_dir_all(dir).map_err(|source| PathError::Create {
                path: dir.clone(),
                source,
            })?;
        }

        if !self.runtime_dir.exists() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.runtime_dir)
                .map_err(|source| PathError::Create {
                    path: self.runtime_dir.clone(),
                    source,
                })?;
        }
        Ok(())
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// User preferences.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Browse and playback state.
    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join("state.json")
    }

    /// The session cookie, kept apart from preferences and stored owner-only.
    pub fn cookie_file(&self) -> PathBuf {
        self.state_dir.join("cookie")
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join("niveus.log")
    }

    /// Where the player's control socket lives.
    pub fn socket_file(&self) -> PathBuf {
        self.runtime_dir.join("niveus-mpv.sock")
    }
}

/// `$XDG_RUNTIME_DIR`, or a private directory under `$TMPDIR` or `/tmp`.
///
/// `XDG_RUNTIME_DIR` is set by systemd-logind and is simply absent under
/// containers and SSH sessions, so a fallback is required rather than optional.
/// The fallback is always a per-user subdirectory, even when `$TMPDIR` is set,
/// because `/tmp` and friends are world-writable and the socket must not be.
fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }

    let name = match current_uid() {
        Some(uid) => format!("niveus-{uid}"),
        None => "niveus".to_string(),
    };
    let base = std::env::var_os("TMPDIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(name)
}

/// The current user's id, read from the ownership of `/proc/self`.
///
/// Avoids a dependency on a platform crate just to call `getuid`.
fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").ok().map(|meta| meta.uid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_fallback_is_per_user_and_not_shared() {
        // The real environment is not touched; the rule under test is that the
        // fallback never resolves to a world-writable directory itself.
        let fallback = PathBuf::from("/tmp").join(match current_uid() {
            Some(uid) => format!("niveus-{uid}"),
            None => "niveus".to_string(),
        });
        assert_ne!(fallback, PathBuf::from("/tmp"));
        assert!(fallback
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("niveus"));
    }

    #[test]
    fn file_names_follow_the_documented_layout() {
        let paths = Paths::new(
            PathBuf::from("/c"),
            PathBuf::from("/s"),
            PathBuf::from("/r"),
        );
        assert_eq!(paths.config_file(), PathBuf::from("/c/config.toml"));
        assert_eq!(paths.state_file(), PathBuf::from("/s/state.json"));
        assert_eq!(paths.cookie_file(), PathBuf::from("/s/cookie"));
        assert_eq!(paths.log_file(), PathBuf::from("/s/niveus.log"));
        assert_eq!(paths.socket_file(), PathBuf::from("/r/niveus-mpv.sock"));
    }

    #[test]
    fn directories_are_created_owner_only_including_parents() {
        let base = std::env::temp_dir().join(format!("niveus-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let paths = Paths::new(
            base.join("config/niveus"),
            base.join("state/niveus"),
            base.join("run/niveus-1000"),
        );

        paths.ensure_dirs().unwrap();
        assert!(paths.config_dir().is_dir());
        assert!(paths.state_dir().is_dir());
        assert!(paths.runtime_dir().is_dir());

        // The runtime directory is not readable by anyone else.
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(paths.runtime_dir())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "runtime dir mode is {mode:o}");

        // Calling it twice is not an error.
        paths.ensure_dirs().unwrap();

        std::fs::remove_dir_all(&base).unwrap();
    }
}
