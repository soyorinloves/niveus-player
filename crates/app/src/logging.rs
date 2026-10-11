//! Logging to a file.
//!
//! The interface owns the terminal, so nothing is ever written to stdout or
//! stderr while it is running; every diagnostic goes to the log file.

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Mutex;
use tracing_subscriber::EnvFilter;

/// Starts logging to `log_file`.
///
/// Returns an error rather than panicking if the subscriber is already set, so
/// a caller that has one can carry on.
pub fn init(log_file: &Path) -> std::io::Result<()> {
    if let Some(parent) = log_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)?;

    // `RUST_LOG` overrides the level; the default keeps ordinary operation
    // quiet while still recording warnings and failures.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(false)
        .try_init()
        .map_err(|e| std::io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_file_is_created_even_in_a_missing_directory() {
        let dir = std::env::temp_dir().join(format!("niveus-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("nested/niveus.log");

        // The global subscriber may already be installed by another test; only
        // the file creation is under test here.
        let _ = init(&file);
        assert!(file.exists(), "log file was not created");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
