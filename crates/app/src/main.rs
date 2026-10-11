//! The `niveus` executable.

use niveus_app::{app, logging, paths::Paths, preflight};
use niveus_config::{Config, ConfigIssue};
use niveus_ui::strings;
use std::process::ExitCode;

fn main() -> ExitCode {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("niveus: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = paths.ensure_dirs() {
        eprintln!("niveus: {error}");
        return ExitCode::FAILURE;
    }

    // Logging is best effort: without it the program still runs, it simply
    // cannot explain what went wrong afterwards.
    if let Err(error) = logging::init(&paths.log_file()) {
        eprintln!("{}", strings::startup::logging_failed(&error.to_string()));
    }
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "niveus starting");

    let report = match preflight::check(&paths) {
        Ok(report) => report,
        Err(message) => {
            tracing::error!(%message, "start-up check failed");
            eprintln!("niveus: {message}");
            return ExitCode::FAILURE;
        }
    };

    // What could not be used from the file is carried into the interface
    // rather than only logged: a key that does nothing is a mystery otherwise
    // (SPEC §25).
    let (config, notices) = Config::load(&paths.config_file());

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{}", strings::startup::runtime_failed(&error.to_string()));
            return ExitCode::FAILURE;
        }
    };

    // Warnings are handed to the interface rather than printed, since the
    // interface is about to take over the screen and is the only place with
    // room to say anything; it shows them one at a time, in this order.
    //
    // The configuration reports what was wrong as structure; the sentence is
    // the interface's, and is written here (SPEC §39).
    let mut warnings = report.warnings;
    warnings.extend(notices.into_iter().map(describe));

    match runtime.block_on(app::run(paths, config, warnings)) {
        Ok(()) => {
            tracing::info!("niveus stopped");
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(%error, "niveus stopped with an error");
            eprintln!("niveus: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Puts one thing `config.toml` could not give us into a sentence for the
/// user.
fn describe(issue: ConfigIssue) -> String {
    match issue {
        ConfigIssue::Unreadable { error } => strings::config::unreadable(&error),
        ConfigIssue::Unparsable { error } => strings::config::unparsable(&error),
        ConfigIssue::KeybindingIgnored { error } => {
            strings::config::keybinding_ignored(&error.to_string())
        }
        ConfigIssue::UnknownQuality { raw, used } => strings::config::unknown_quality(&raw, used),
    }
}
