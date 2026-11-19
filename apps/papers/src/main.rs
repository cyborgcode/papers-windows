//! Papers: a fast PDF reader.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod settings;
mod updates;
mod view;

use std::path::PathBuf;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mut files = Vec::new();
    let mut page = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--page" | "-p") => page = args.next().and_then(|v| v.to_str().and_then(|s| s.parse().ok())),
            Some("--version" | "-V") => {
                println!("papers {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Some(s) if s.starts_with("--page=") => page = s["--page=".len()..].parse().ok(),
            _ => files.push(absolute(PathBuf::from(arg))),
        }
    }

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Papers")
        .with_app_id("papers")
        .with_inner_size([1100.0, 860.0])
        .with_min_inner_size([420.0, 320.0])
        .with_drag_and_drop(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, persist_window: true, ..Default::default() };
    eframe::run_native("Papers", options, Box::new(move |cc| Ok(Box::new(app::PapersApp::new(cc, files, page)))))
}

fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}
