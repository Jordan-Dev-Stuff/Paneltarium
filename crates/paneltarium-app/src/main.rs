//! Paneltarium background app: global hotkeys, the numbered profile
//! overlay and the tray icon.
//!
//! Release builds on Windows use the GUI subsystem, so no console window
//! appears. Debug builds keep the console so `cargo run` shows the log.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod bindings;
mod overlay;
mod tray;
mod watcher;
mod worker;

use std::process::ExitCode;

use global_hotkey::GlobalHotKeyEvent;
use pt_core::AppPaths;
use tracing_appender::non_blocking::WorkerGuard;
use tray_icon::menu::MenuEvent;
use winit::event_loop::{ControlFlow, EventLoop};

use app::{App, UserEvent, poster};

fn main() -> ExitCode {
    let paths = match AppPaths::from_env() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("paneltarium-app: {e}");
            return ExitCode::FAILURE;
        }
    };
    let _log = init_logging(&paths);

    match run(paths) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(paths: AppPaths) -> Result<(), String> {
    let Some(_instance) = pt_core::instance::acquire(paths.root(), "app").map_err(|e| format!("lock file: {e}"))?
    else {
        tracing::info!("another Paneltarium app is already running; exiting");
        return Ok(());
    };
    tracing::info!("starting, config in {}", paths.root().display());

    // Linux: tray-icon's menu is GTK, which must be initialised first.
    #[cfg(target_os = "linux")]
    gtk::init().map_err(|e| format!("GTK: {e}"))?;

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| format!("event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();

    // Hotkey and menu callbacks fire inside the OS message loop; forward
    // them into our event loop as user events.
    GlobalHotKeyEvent::set_event_handler(Some(poster(&proxy, UserEvent::Hotkey)));
    MenuEvent::set_event_handler(Some(poster(&proxy, UserEvent::Menu)));

    // Windows (or the Linux desktop) passes --autostart when it starts us at sign-in.
    let at_sign_in = std::env::args().any(|a| a == pt_core::AUTOSTART_ARG);
    if at_sign_in {
        tracing::info!("started at sign-in");
    }
    let mut app = App::new(paths, &proxy, at_sign_in)?;
    event_loop.run_app(&mut app).map_err(|e| format!("event loop: {e}"))?;
    tracing::info!("stopped");
    Ok(())
}

/// Log to `<config>/logs/paneltarium-app.log.<date>`, and to the console in
/// debug builds. `RUST_LOG` overrides the level (default: info).
fn init_logging(paths: &AppPaths) -> Option<WorkerGuard> {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::writer::MakeWriterExt;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // The appender prunes old logs on start-up, which fails if the folder
    // doesn't exist yet.
    let _ = std::fs::create_dir_all(paths.log_dir());
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("paneltarium-app")
        .filename_suffix("log")
        .max_log_files(7)
        .build(paths.log_dir());
    match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let builder = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false);
            if cfg!(debug_assertions) {
                builder.with_writer(writer.and(std::io::stderr)).init();
            } else {
                builder.with_writer(writer).init();
            }
            Some(guard)
        }
        Err(e) => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
            tracing::warn!("could not open log file: {e}");
            None
        }
    }
}
