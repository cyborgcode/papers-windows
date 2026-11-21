//! Update checks and installation from inside Papers. All network work runs on
//! background threads; the UI only polls channels.

use papers_update::{ApplyPlan, InstallKind, Manifest};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const DAY: u64 = 24 * 60 * 60;

pub enum State {
    Idle,
    Checking { rx: Receiver<Result<Option<Manifest>, String>>, manual: bool },
    UpToDate,
    Available(Manifest),
    Downloading {
        manifest: Manifest,
        done: Arc<AtomicU64>,
        total: u64,
        cancel: Arc<AtomicBool>,
        rx: Receiver<Result<PathBuf, String>>,
    },
    Failed(String),
}

pub struct Updates {
    pub state: State,
    /// Show the result of a manual check in a window.
    pub window: bool,
    pub banner_dismissed: bool,
    /// Message about the previous update (shown once after a restart).
    pub outcome: Option<papers_update::Outcome>,
}

/// What the app must do after the updater has been started.
pub enum Next {
    Nothing,
    /// Close the window so the updater can replace the files.
    Quit,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Updates {
    pub fn new() -> Self {
        // Clean up leftovers of an earlier portable update.
        if let InstallKind::Portable { dir } = InstallKind::current() {
            papers_update::cleanup(&dir);
        }
        let outcome = papers_update::take_outcome();
        Updates { state: State::Idle, window: false, banner_dismissed: false, outcome }
    }

    /// Check once a day in the background, if enabled.
    pub fn auto_check(&mut self, ctx: &egui::Context, enabled: bool, last: &mut u64) {
        if !enabled || !matches!(self.state, State::Idle) || std::env::var_os("PAPERS_NO_UPDATE_CHECK").is_some() {
            return;
        }
        let now = now();
        if now.saturating_sub(*last) < DAY {
            return;
        }
        *last = now;
        self.check(ctx, false);
    }

    pub fn check(&mut self, ctx: &egui::Context, manual: bool) {
        if manual {
            self.window = true;
            self.banner_dismissed = false;
        }
        if matches!(self.state, State::Checking { .. } | State::Downloading { .. }) {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = papers_update::check(VERSION).map_err(|e| e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.state = State::Checking { rx, manual };
    }

    pub fn poll(&mut self, skipped_version: &str) {
        match &mut self.state {
            State::Checking { rx, manual } => {
                let manual = *manual;
                if let Ok(result) = rx.try_recv() {
                    self.state = match result {
                        Ok(Some(m)) if manual || m.version != skipped_version => State::Available(m),
                        Ok(_) => State::UpToDate,
                        Err(e) if manual => State::Failed(e),
                        Err(e) => {
                            log::info!("automatic update check failed: {e}");
                            State::Idle
                        }
                    };
                }
            }
            State::Downloading { rx, .. } => {
                if let Ok(Err(e)) = rx.try_recv() {
                    self.state = State::Failed(e);
                    self.window = true;
                }
            }
            _ => {}
        }
    }

    /// Start downloading the available update.
    pub fn download(&mut self, ctx: &egui::Context) {
        let State::Available(manifest) = &self.state else { return };
        let kind = InstallKind::current();
        let Some(asset) = manifest.asset_for(&kind).cloned() else {
            // Not an installed or portable copy: send the person to the download page.
            let url = if manifest.notes_url.is_empty() { papers_update::RELEASES_URL.to_string() } else { manifest.notes_url.clone() };
            ctx.open_url(egui::OpenUrl::new_tab(url));
            return;
        };
        let manifest = manifest.clone();
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let total = asset.size;
        {
            let (done, cancel, ctx) = (done.clone(), cancel.clone(), ctx.clone());
            std::thread::spawn(move || {
                let mut last_repaint = 0;
                let result = papers_update::download(
                    &asset,
                    &papers_update::state_dir(),
                    &mut |d, _| {
                        done.store(d, Ordering::Relaxed);
                        if d - last_repaint > 256 * 1024 {
                            last_repaint = d;
                            ctx.request_repaint();
                        }
                    },
                    &cancel,
                )
                .map_err(|e| e.to_string());
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }
        self.state = State::Downloading { manifest, done, total, cancel, rx };
    }

    pub fn cancel(&mut self) {
        if let State::Downloading { cancel, manifest, .. } = &self.state {
            cancel.store(true, Ordering::Relaxed);
            self.state = State::Available(manifest.clone());
        }
    }

    /// When a download has finished: start the updater and tell the app to close.
    pub fn finish(&mut self, open: Option<PathBuf>) -> Next {
        let State::Downloading { rx, manifest, .. } = &self.state else { return Next::Nothing };
        let Ok(Ok(package)) = rx.try_recv() else { return Next::Nothing };
        let kind = InstallKind::current();
        let Some(asset) = manifest.asset_for(&kind).cloned() else { return Next::Nothing };
        let exe = std::env::current_exe().ok();
        let updater = exe.as_ref().and_then(|e| e.parent()).map(|d| d.join(papers_update::UPDATER_EXE));
        let plan = ApplyPlan {
            package,
            asset,
            version: manifest.version.clone(),
            kind,
            wait_pid: Some(std::process::id()),
            relaunch: exe,
            open: open.into_iter().collect(),
        };
        match updater.ok_or_else(|| papers_update::Error("Papers' folder is unknown.".into())).and_then(|u| papers_update::spawn_updater(&u, &plan)) {
            Ok(()) => Next::Quit,
            Err(e) => {
                self.state = State::Failed(e.to_string());
                self.window = true;
                Next::Nothing
            }
        }
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.state, State::Checking { .. } | State::Downloading { .. })
    }
}

/// The Adwaita banner shown under the header bar. Returns the version to skip, if chosen.
pub fn banner(ui: &mut egui::Ui, updates: &mut Updates) -> Option<String> {
    use crate::adw;
    let mut skip = None;
    if let Some(outcome) = updates.outcome.clone() {
        let text = if outcome.ok {
            format!("Papers was updated to {}", outcome.version)
        } else {
            format!("The update to {} failed: {}", outcome.version, outcome.message)
        };
        if adw::banner(ui, &text, Some("Dismiss")) {
            updates.outcome = None;
        }
    }
    if updates.banner_dismissed {
        return None;
    }
    let ctx = ui.ctx().clone();
    match &updates.state {
        State::Available(m) => {
            let version = m.version.clone();
            let label = match InstallKind::current() {
                InstallKind::Unmanaged => "Download…",
                _ => "Update and Restart",
            };
            let p = adw::Palette::of(ui);
            let fill = adw::blend(p.headerbar_bg, p.accent_bg, if p.dark { 0.25 } else { 0.15 });
            let mut start = false;
            egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_centered(|ui| {
                    ui.set_min_height(34.0);
                    ui.label(format!("Papers {version} is available"));
                    start = adw::text_button(ui, label, true, true).clicked();
                    if adw::text_button(ui, "Skip This Version", false, true).clicked() {
                        skip = Some(version.clone());
                        updates.banner_dismissed = true;
                    }
                    if adw::icon_button_sized(ui, adw::Icon::WindowClose, "Remind Me Later", false, egui::vec2(34.0, 34.0), true).clicked() {
                        updates.banner_dismissed = true;
                    }
                });
            });
            if start {
                updates.download(&ctx);
            }
        }
        State::Downloading { manifest, done, total, .. } => {
            let fraction = done.load(Ordering::Relaxed) as f32 / (*total).max(1) as f32;
            let version = manifest.version.clone();
            let p = adw::Palette::of(ui);
            let fill = adw::blend(p.headerbar_bg, p.accent_bg, if p.dark { 0.25 } else { 0.15 });
            let mut cancel = false;
            egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_centered(|ui| {
                    ui.set_min_height(34.0);
                    ui.label(format!("Downloading Papers {version}…"));
                    ui.add(egui::ProgressBar::new(fraction).desired_width(200.0).desired_height(6.0).fill(p.accent_bg));
                    cancel = adw::text_button(ui, "Cancel", false, true).clicked();
                });
            });
            if cancel {
                updates.cancel();
            }
        }
        _ => {}
    }
    skip
}

/// The "Software Updates" dialog for a manual check.
pub fn window(ctx: &egui::Context, updates: &mut Updates, auto_check: &mut bool) {
    use crate::adw;
    if !updates.window {
        return;
    }
    let dark = ctx.global_style().visuals.dark_mode;
    let mut close = false;
    let mut start = false;
    let mut check = false;
    let response = egui::Modal::new(egui::Id::unique("updates-dialog")).frame(adw::dialog_frame(dark)).show(ctx, |ui| {
        let p = adw::Palette::of(ui);
        ui.set_width(380.0);
        close = adw::dialog_header(ui, "Software Updates");
        egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 18, top: 0, bottom: 18 }).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                let (title, detail): (String, String) = match &updates.state {
                    State::Idle => ("Not Checked Yet".into(), String::new()),
                    State::Checking { .. } => ("Checking for Updates…".into(), String::new()),
                    State::UpToDate => ("Papers Is Up to Date".into(), format!("Version {VERSION} is the latest.")),
                    State::Available(m) => (format!("Papers {} Is Available", m.version), m.notes.clone()),
                    State::Downloading { manifest, .. } => (format!("Downloading Papers {}…", manifest.version), String::new()),
                    State::Failed(e) => ("Could Not Check for Updates".into(), e.clone()),
                };
                ui.add_space(6.0);
                if matches!(updates.state, State::Checking { .. } | State::Downloading { .. }) {
                    ui.add(egui::Spinner::new().size(32.0));
                    ui.add_space(8.0);
                }
                ui.label(egui::RichText::new(title).font(adw::bold(20.0)));
                if !detail.is_empty() {
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(egui::RichText::new(detail).color(p.dim())).wrap());
                }
                if let State::Downloading { done, total, .. } = &updates.state {
                    let f = done.load(Ordering::Relaxed) as f32 / (*total).max(1) as f32;
                    ui.add(egui::ProgressBar::new(f).desired_height(6.0).fill(p.accent_bg));
                }
                ui.add_space(14.0);
                if matches!(updates.state, State::Available(_)) {
                    start = adw::pill_button(ui, "Update and Restart", true).clicked();
                    ui.add_space(10.0);
                }
            });
            adw::boxed_list(ui, |list| {
                list.row("Check Automatically", |ui| {
                    ui.checkbox(auto_check, "");
                });
                list.row(&format!("Installed Version {VERSION} ({})", InstallKind::current().id()), |ui| {
                    if adw::text_button(ui, "Check Now", false, !updates.is_busy()).clicked() {
                        check = true;
                    }
                });
                if list.action("All Releases", "").clicked() {
                    list.ui.ctx().open_url(egui::OpenUrl::new_tab(papers_update::RELEASES_URL));
                }
            });
        });
    });
    if check {
        updates.check(ctx, true);
    }
    if start {
        updates.download(ctx);
    }
    if close || response.should_close() {
        updates.window = false;
    }
}
