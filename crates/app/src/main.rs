//! The `niveus` executable.

use niveus_app::{app, logging, paths::Paths, preflight};
use niveus_config::Config;
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
        eprintln!("niveus: 无法初始化日志：{error}");
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
            eprintln!("niveus: 无法启动异步运行时：{error}");
            return ExitCode::FAILURE;
        }
    };

    // Warnings are handed to the interface rather than printed, since the
    // interface is about to take over the screen and is the only place with
    // room to say anything; it shows them one at a time, in this order.
    let mut warnings = report.warnings;
    warnings.extend(notices.into_iter().map(|notice| notice.message));

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
