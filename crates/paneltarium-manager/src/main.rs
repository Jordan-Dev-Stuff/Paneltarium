//! Paneltarium profile manager: a normal window for renaming, numbering,
//! setting hotkeys, saving and deleting profiles. Started from the tray
//! ("Open profile manager…") or directly. The same code runs on Windows and
//! Linux (eframe/egui); it only edits the config files, which the running
//! background app watches.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod keys;
mod model;
mod ui;

use std::process::ExitCode;

use eframe::egui;
use pt_core::{AppPaths, MonitorService};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("paneltarium-manager: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let paths = AppPaths::from_env().map_err(|e| e.to_string())?;
    let Some(_lock) = pt_core::instance::acquire(paths.root(), "manager").map_err(|e| e.to_string())? else {
        // Already open. (Bringing the existing window forward is a later step.)
        return Ok(());
    };
    let backend = pt_platform::backend_from_env().map_err(|e| e.to_string())?;
    let autostart = pt_platform::autostart().ok();
    let model = model::Model::new(MonitorService::new(backend, paths), autostart);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Paneltarium — Profiles")
            .with_app_id(pt_core::text::MANAGER_APP_ID)
            .with_inner_size([940.0, 640.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(egui::IconData {
                rgba: pt_brand::icon_rgba(64),
                width: 64,
                height: 64,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "Paneltarium",
        options,
        Box::new(move |cc| Ok(Box::new(ui::ManagerApp::new(cc, model)))),
    )
    .map_err(|e| e.to_string())
}
