//! The Papers window: toolbar, sidebar, search bar, dialogs and keyboard shortcuts.

use crate::settings::{self, Fit, Recent, Settings, Theme};
use crate::updates::{self, Updates};
use crate::view::{Action, DocView};
use egui::{Align, Align2, Color32, Key, KeyboardShortcut, Layout, Modifiers, RichText, Ui, vec2};
use papers_render::{DocId, Engine, Event, NightStyle, OutlineItem, Request};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const PDF_EXTENSIONS: &[&str] = &["pdf", "ai"];

struct Opening {
    id: DocId,
    path: PathBuf,
    restore: Option<Recent>,
}

struct PasswordPrompt {
    path: PathBuf,
    restore: Option<Recent>,
    text: String,
    wrong: bool,
}

pub struct PapersApp {
    engine: Engine,
    settings: Settings,
    doc: Option<DocView>,
    opening: Option<Opening>,
    next_id: DocId,
    password: Option<PasswordPrompt>,
    message: Option<(String, Instant)>,
    engine_error: Option<String>,
    updates: Updates,
    show_about: bool,
    show_properties: bool,
    show_shortcuts: bool,
    page_text: String,
    page_focus: bool,
    fullscreen: bool,
    title: String,
    watched: Option<(PathBuf, Option<SystemTime>, Instant)>,
    applied_theme: Option<egui::ThemePreference>,
    #[cfg(windows)]
    hwnd: isize,
}

impl PapersApp {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>, page: Option<u32>) -> Self {
        let settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, settings::KEY)).unwrap_or_default();
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        style(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let engine = Engine::start(library_dirs(), move || ctx.request_repaint());
        #[cfg(windows)]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match cc.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
                _ => 0,
            }
        };
        let mut app = PapersApp {
            engine,
            settings,
            doc: None,
            opening: None,
            next_id: 1,
            password: None,
            message: None,
            engine_error: None,
            updates: Updates::new(),
            show_about: false,
            show_properties: false,
            show_shortcuts: false,
            page_text: String::new(),
            page_focus: false,
            fullscreen: false,
            title: String::new(),
            watched: None,
            applied_theme: None,
            #[cfg(windows)]
            hwnd,
        };
        let mut files = files.into_iter();
        if let Some(first) = files.next() {
            let mut restore = app.settings.recent_for(&first).cloned();
            if let Some(page) = page {
                restore = Some(Recent {
                    page: page.saturating_sub(1),
                    offset: 0.0,
                    ..restore.unwrap_or_else(|| default_recent(&first))
                });
            }
            app.open(first, restore);
        }
        for other in files {
            open_in_new_window(Some(&other));
        }
        app
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), Instant::now()));
    }

    fn night(&self) -> Option<NightStyle> {
        self.settings.night()
    }

    // ----- documents -----

    fn open(&mut self, path: PathBuf, restore: Option<Recent>) {
        self.open_with_password(path, restore, None);
    }

    fn open_with_password(&mut self, path: PathBuf, restore: Option<Recent>, password: Option<String>) {
        let id = self.next_id;
        self.next_id += 1;
        let restore = restore.or_else(|| self.settings.recent_for(&path).cloned());
        self.engine.send(Request::Open { id, path: path.clone(), password });
        self.opening = Some(Opening { id, path, restore });
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open Document")
            .add_filter("PDF documents", PDF_EXTENSIONS)
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.doc.as_ref().and_then(|d| d.info.path.parent().map(Path::to_path_buf)) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open(path, None);
        }
    }

    fn close_document(&mut self) {
        if let Some(doc) = self.doc.take() {
            self.settings.remember(doc.recent());
            self.engine.send(Request::Close { id: doc.id() });
        }
        self.watched = None;
        self.show_properties = false;
    }

    fn reload(&mut self) {
        if let Some(doc) = &self.doc {
            let restore = doc.recent();
            let path = doc.info.path.clone();
            self.open(path, Some(restore));
        }
    }

    fn process_events(&mut self) {
        while let Some(event) = self.engine.poll() {
            match event {
                Event::Opened(info) => {
                    let Some(opening) = self.opening.take_if(|o| o.id == info.id) else {
                        self.engine.send(Request::Close { id: info.id });
                        continue;
                    };
                    let reloading = self.doc.as_ref().is_some_and(|d| d.info.path == info.path);
                    let search = self.doc.as_ref().filter(|_| reloading).map(|d| (d.search.open, d.search.query.clone()));
                    self.close_document();
                    let mut view = DocView::new(info, opening.restore.as_ref());
                    view.set_dual(self.settings.dual);
                    if let Some((open, query)) = search {
                        view.search.open = open;
                        view.search.query = query;
                        view.search_changed();
                    }
                    let mtime = std::fs::metadata(&view.info.path).and_then(|m| m.modified()).ok();
                    self.watched = Some((view.info.path.clone(), mtime, Instant::now()));
                    self.settings.remember(view.recent());
                    self.doc = Some(view);
                }
                Event::OpenFailed { id, path, error, needs_password } => {
                    let Some(opening) = self.opening.take_if(|o| o.id == id) else { continue };
                    if needs_password {
                        let wrong = self.password.is_some();
                        self.password = Some(PasswordPrompt { path, restore: opening.restore, text: String::new(), wrong });
                    } else {
                        self.notify(format!("Could not open {}: {error}", settings::file_name(&path)));
                    }
                }
                Event::Tile { doc, key, night, image } => {
                    if let Some(view) = self.doc.as_mut().filter(|v| v.id() == doc) {
                        view.tile_arrived(key, night, image);
                    }
                }
                Event::TileFailed { .. } => {}
                Event::Extras { doc, page, extras } => {
                    if let Some(view) = self.doc.as_mut().filter(|v| v.id() == doc) {
                        view.extras_arrived(page, extras);
                    }
                }
                Event::SearchHits { doc, generation, page, matches } => {
                    if let Some(view) = self.doc.as_mut().filter(|v| v.id() == doc) {
                        view.search_hits(generation, page, matches);
                    }
                }
                Event::SearchProgress { doc, generation, searched, total } => {
                    if let Some(view) = self.doc.as_mut().filter(|v| v.id() == doc) {
                        view.search_progress(generation, searched, total);
                    }
                }
                Event::SearchDone { doc, generation } => {
                    if let Some(view) = self.doc.as_mut().filter(|v| v.id() == doc) {
                        view.search_done(generation);
                    }
                }
                Event::EngineFailed { error } => self.engine_error = Some(error),
                Event::PrintDone { result } => match result {
                    Ok(true) => self.notify("Sent to the printer."),
                    Ok(false) => {}
                    Err(e) => self.notify(format!("Printing failed: {e}")),
                },
            }
        }
    }

    /// Reload the document when it changes on disk (handy when writing LaTeX).
    fn watch_file(&mut self, ctx: &egui::Context) {
        let Some((path, mtime, checked)) = &mut self.watched else { return };
        if checked.elapsed() < Duration::from_millis(1500) {
            ctx.request_repaint_after(Duration::from_millis(1600));
            return;
        }
        *checked = Instant::now();
        ctx.request_repaint_after(Duration::from_millis(1600));
        let now = std::fs::metadata(&*path).and_then(|m| m.modified()).ok();
        if now.is_some() && now != *mtime {
            *mtime = now;
            if self.opening.is_none() {
                self.reload();
            }
        }
    }

    // ----- appearance -----

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let pref = if self.settings.night {
            egui::ThemePreference::Dark
        } else {
            match self.settings.theme {
                Theme::System => egui::ThemePreference::System,
                Theme::Light => egui::ThemePreference::Light,
                Theme::Dark => egui::ThemePreference::Dark,
            }
        };
        if self.applied_theme != Some(pref) {
            ctx.set_theme(pref);
            self.applied_theme = Some(pref);
        }
    }

    fn toggle_night(&mut self) {
        self.settings.night = !self.settings.night;
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match &self.doc {
            Some(d) => {
                let name = d.info.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| settings::file_name(&d.info.path));
                format!("{name} — Papers")
            }
            None => "Papers".to_string(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    // ----- keyboard -----

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let cmd = Modifiers::COMMAND;
        let shortcut = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
        let key = |k: Key| !typing && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));

        if shortcut(cmd, Key::O) {
            self.open_dialog();
        }
        if shortcut(cmd, Key::N) {
            open_in_new_window(None);
        }
        if shortcut(cmd, Key::Q) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if shortcut(cmd, Key::I) {
            self.toggle_night();
        }
        if shortcut(Modifiers::NONE, Key::F11) {
            self.set_fullscreen(ctx, !self.fullscreen);
        }
        if shortcut(Modifiers::NONE, Key::F9) {
            self.settings.sidebar = !self.settings.sidebar;
        }
        if (self.show_properties || self.show_shortcuts || self.show_about) && shortcut(Modifiers::NONE, Key::Escape) {
            self.show_properties = false;
            self.show_shortcuts = false;
            self.show_about = false;
            return;
        }
        let Some(doc) = self.doc.as_mut() else { return };
        if doc.presenting {
            let mut stop = false;
            ctx.input_mut(|i| {
                for k in [Key::ArrowRight, Key::ArrowDown, Key::PageDown, Key::Space, Key::Enter, Key::N] {
                    if i.consume_key(Modifiers::NONE, k) {
                        doc.next_page();
                    }
                }
                for k in [Key::ArrowLeft, Key::ArrowUp, Key::PageUp, Key::Backspace, Key::P] {
                    if i.consume_key(Modifiers::NONE, k) {
                        doc.prev_page();
                    }
                }
                if i.consume_key(Modifiers::NONE, Key::Home) {
                    doc.go_to_page(0);
                }
                if i.consume_key(Modifiers::NONE, Key::End) {
                    doc.go_to_page(u32::MAX);
                }
                stop = i.consume_key(Modifiers::NONE, Key::Escape) || i.consume_key(Modifiers::NONE, Key::F5);
            });
            if stop {
                doc.presenting = false;
                self.set_fullscreen(ctx, false);
            }
            return;
        }
        if shortcut(cmd, Key::W) {
            self.close_document();
            return;
        }
        if shortcut(cmd, Key::R) {
            self.reload();
            return;
        }
        if shortcut(cmd, Key::P) {
            self.print();
            return;
        }
        if shortcut(Modifiers::NONE, Key::F5) {
            doc.presenting = true;
            self.set_fullscreen(ctx, true);
            return;
        }
        let doc = self.doc.as_mut().expect("checked above");
        if shortcut(cmd, Key::F) {
            doc.search.open = true;
            doc.search.focus = true;
        }
        if doc.search.open {
            if shortcut(Modifiers::SHIFT, Key::F3) {
                doc.search_step(false, &self.engine);
            } else if shortcut(Modifiers::NONE, Key::F3) {
                doc.search_step(true, &self.engine);
            }
        } else if shortcut(Modifiers::NONE, Key::F3) {
            doc.search.open = true;
            doc.search.focus = true;
        }
        if shortcut(Modifiers::NONE, Key::Escape) {
            if doc.has_selection() {
                doc.clear_selection();
            } else if doc.search.open {
                doc.close_search(&self.engine);
            } else if self.fullscreen {
                self.set_fullscreen(ctx, false);
                return;
            }
        }
        let doc = self.doc.as_mut().expect("checked above");
        if !typing {
            if shortcut(cmd, Key::C)
                && let Some(text) = doc.selected_text()
            {
                ctx.copy_text(text);
            }
            if ctx.input_mut(|i| i.consume_key(cmd, Key::A)) {
                doc.select_all_on_page();
            }
        }
        if shortcut(cmd, Key::Plus) || shortcut(cmd, Key::Equals) || shortcut(cmd.plus(Modifiers::SHIFT), Key::Equals) {
            doc.zoom_in();
        }
        if shortcut(cmd, Key::Minus) {
            doc.zoom_out();
        }
        if shortcut(cmd, Key::Num0) {
            doc.set_fit(Fit::Width);
        }
        if shortcut(cmd, Key::Num9) {
            doc.set_fit(Fit::Page);
        }
        if shortcut(cmd, Key::Num1) {
            doc.set_zoom(1.0, None);
        }
        if shortcut(cmd, Key::ArrowLeft) {
            doc.rotate(false);
        }
        if shortcut(cmd, Key::ArrowRight) {
            doc.rotate(true);
        }
        if shortcut(cmd, Key::D) {
            self.settings.dual = !self.settings.dual;
            doc.set_dual(self.settings.dual);
        }
        if shortcut(cmd, Key::L) || shortcut(cmd, Key::G) {
            self.page_focus = true;
        }
        if shortcut(Modifiers::ALT, Key::Enter) {
            self.show_properties = true;
        }
        if shortcut(cmd, Key::PageDown) || key(Key::N) {
            doc.next_page();
        }
        if shortcut(cmd, Key::PageUp) || key(Key::P) {
            doc.prev_page();
        }
        if key(Key::PageDown) || key(Key::Space) {
            doc.page_down();
        }
        if key(Key::PageUp) || (!typing && shortcut(Modifiers::SHIFT, Key::Space)) || key(Key::Backspace) {
            doc.page_up();
        }
        if key(Key::Home) || shortcut(cmd, Key::Home) {
            doc.home();
        }
        if key(Key::End) || shortcut(cmd, Key::End) {
            doc.end();
        }
        if key(Key::ArrowDown) {
            doc.scroll_by(vec2(0.0, 48.0));
        }
        if key(Key::ArrowUp) {
            doc.scroll_by(vec2(0.0, -48.0));
        }
        if key(Key::ArrowRight) {
            doc.scroll_by(vec2(48.0, 0.0));
        }
        if key(Key::ArrowLeft) {
            doc.scroll_by(vec2(-48.0, 0.0));
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    fn print(&mut self) {
        let Some(doc) = &self.doc else { return };
        #[cfg(windows)]
        {
            let title = doc.info.title.clone().unwrap_or_else(|| settings::file_name(&doc.info.path));
            self.engine.send(Request::Print { doc: doc.id(), owner: self.hwnd, title });
        }
        #[cfg(not(windows))]
        {
            let _ = doc;
            self.notify("Printing is available in the Windows build.");
        }
    }

    // ----- panels -----

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.button("🗁").on_hover_text("Open a document (Ctrl+O)").clicked() {
                self.open_dialog();
            }
            let has_doc = self.doc.is_some();
            if ui
                .add_enabled(has_doc, egui::Button::new("◧").selected(self.settings.sidebar))
                .on_hover_text("Sidebar (F9)")
                .clicked()
            {
                self.settings.sidebar = !self.settings.sidebar;
            }
            if let Some(doc) = self.doc.as_mut() {
                ui.separator();
                if ui.button("⏶").on_hover_text("Previous page").clicked() {
                    doc.prev_page();
                }
                if ui.button("⏷").on_hover_text("Next page").clicked() {
                    doc.next_page();
                }
                let page_id = ui.scope_id().with("page-entry");
                let editing = ui.memory(|m| m.has_focus(page_id));
                if !editing {
                    self.page_text = (doc.current_page + 1).to_string();
                }
                let entry = ui.add(
                    egui::TextEdit::singleline(&mut self.page_text)
                        .id(page_id)
                        .desired_width(44.0)
                        .align(Align2::CENTER_CENTER),
                );
                if std::mem::take(&mut self.page_focus) {
                    entry.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), page_id) {
                        let all = egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(self.page_text.chars().count()),
                        );
                        state.cursor.set_char_range(Some(all));
                        state.store(ui.ctx(), page_id);
                    }
                }
                if entry.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
                    && let Ok(n) = self.page_text.trim().parse::<u32>() {
                        doc.go_to_page(n.saturating_sub(1));
                    }
                ui.label(format!("of {}", doc.page_count()));
                ui.separator();
                if ui.button("−").on_hover_text("Zoom out (Ctrl+−)").clicked() {
                    doc.zoom_out();
                }
                let label = match doc.fit {
                    Fit::Width => "Fit width".to_string(),
                    Fit::Page => "Fit page".to_string(),
                    Fit::None => doc.zoom_label(),
                };
                egui::ComboBox::from_id_salt("zoom").selected_text(label).width(96.0).show_ui(ui, |ui| {
                    if ui.selectable_label(doc.fit == Fit::Width, "Fit width").clicked() {
                        doc.set_fit(Fit::Width);
                    }
                    if ui.selectable_label(doc.fit == Fit::Page, "Fit page").clicked() {
                        doc.set_fit(Fit::Page);
                    }
                    ui.separator();
                    for z in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 8.0] {
                        let selected = doc.fit == Fit::None && (doc.zoom - z).abs() < 0.001;
                        if ui.selectable_label(selected, format!("{:.0}%", z * 100.0)).clicked() {
                            doc.set_zoom(z, None);
                        }
                    }
                });
                if ui.button("+").on_hover_text("Zoom in (Ctrl++)").clicked() {
                    doc.zoom_in();
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.main_menu(ui);
                let night = self.settings.night;
                let moon = if night { "☀" } else { "🌙" };
                let tip = if night { "Day mode (Ctrl+I)" } else { "Night mode (Ctrl+I)" };
                if ui.add(egui::Button::new(moon).selected(night)).on_hover_text(tip).clicked() {
                    self.toggle_night();
                }
                if let Some(doc) = self.doc.as_mut()
                    && ui.add(egui::Button::new("🔍").selected(doc.search.open)).on_hover_text("Find (Ctrl+F)").clicked()
                {
                    if doc.search.open {
                        doc.close_search(&self.engine);
                    } else {
                        doc.search.open = true;
                        doc.search.focus = true;
                    }
                }
                if let Some(opening) = &self.opening {
                    ui.spinner().on_hover_text(format!("Opening {}", settings::file_name(&opening.path)));
                }
            });
        });
    }

    fn main_menu(&mut self, ui: &mut Ui) {
        ui.menu_button("☰", |ui| {
            ui.set_min_width(230.0);
            if ui.add(egui::Button::new("Open…").shortcut_text("Ctrl+O")).clicked() {
                self.open_dialog();
            }
            ui.menu_button("Open Recent", |ui| {
                ui.set_min_width(260.0);
                if self.settings.recent.is_empty() {
                    ui.label("No recent documents");
                }
                let mut chosen = None;
                for r in self.settings.recent.iter().take(15) {
                    if ui.button(r.display_name()).on_hover_text(r.path.display().to_string()).clicked() {
                        chosen = Some(r.clone());
                    }
                }
                if let Some(r) = chosen {
                    self.open(r.path.clone(), Some(r));
                }
                if !self.settings.recent.is_empty() {
                    ui.separator();
                    if ui.button("Clear list").clicked() {
                        self.settings.recent.clear();
                    }
                }
            });
            if ui.add(egui::Button::new("New Window").shortcut_text("Ctrl+N")).clicked() {
                open_in_new_window(None);
            }
            let has_doc = self.doc.is_some();
            if ui.add_enabled(has_doc, egui::Button::new("Reload").shortcut_text("Ctrl+R")).clicked() {
                self.reload();
            }
            if ui.add_enabled(has_doc, egui::Button::new("Print…").shortcut_text("Ctrl+P")).clicked() {
                self.print();
            }
            if ui.add_enabled(has_doc, egui::Button::new("Close").shortcut_text("Ctrl+W")).clicked() {
                self.close_document();
            }
            ui.separator();
            let mut dual = self.settings.dual;
            if ui.add_enabled(has_doc, egui::Checkbox::new(&mut dual, "Dual pages  (Ctrl+D)")).changed() {
                self.settings.dual = dual;
                if let Some(d) = self.doc.as_mut() {
                    d.set_dual(dual);
                }
            }
            if let Some(doc) = self.doc.as_mut() {
                if ui.add(egui::Button::new("Rotate Left").shortcut_text("Ctrl+←")).clicked() {
                    doc.rotate(false);
                }
                if ui.add(egui::Button::new("Rotate Right").shortcut_text("Ctrl+→")).clicked() {
                    doc.rotate(true);
                }
            }
            let mut fullscreen = self.fullscreen;
            if ui.checkbox(&mut fullscreen, "Fullscreen  (F11)").changed() {
                self.set_fullscreen(ui.ctx(), fullscreen);
            }
            if ui.add_enabled(has_doc, egui::Button::new("Presentation").shortcut_text("F5")).clicked() {
                if let Some(d) = self.doc.as_mut() {
                    d.presenting = true;
                }
                self.set_fullscreen(ui.ctx(), true);
            }
            ui.separator();
            ui.menu_button("Night Mode Style", |ui| {
                for style in NightStyle::ALL {
                    if ui.radio(self.settings.night_style() == style, style.label()).clicked() {
                        self.settings.night_style = style.id().to_string();
                        self.settings.night = true;
                    }
                }
            });
            ui.menu_button("Interface", |ui| {
                for (theme, label) in [(Theme::System, "Follow Windows"), (Theme::Light, "Light"), (Theme::Dark, "Dark")] {
                    if ui.radio(self.settings.theme == theme, label).clicked() {
                        self.settings.theme = theme;
                    }
                }
                ui.label(RichText::new("Night mode always uses the dark interface.").small().weak());
            });
            ui.separator();
            if ui.add_enabled(has_doc, egui::Button::new("Properties").shortcut_text("Alt+Enter")).clicked() {
                self.show_properties = true;
            }
            if ui.button("Keyboard Shortcuts").clicked() {
                self.show_shortcuts = true;
            }
            if ui.button("Check for Updates…").clicked() {
                self.updates.check(ui.ctx(), true);
            }
            if ui.button("About Papers").clicked() {
                self.show_about = true;
            }
        });
    }

    fn search_bar(&mut self, ui: &mut Ui) {
        let Some(doc) = self.doc.as_mut() else { return };
        ui.horizontal(|ui| {
            let id = ui.scope_id().with("search-entry");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut doc.search.query)
                    .id(id)
                    .hint_text("Find in document")
                    .desired_width(260.0),
            );
            if std::mem::take(&mut doc.search.focus) {
                edit.request_focus();
            }
            if edit.changed() {
                doc.search_changed();
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let forward = !ui.input(|i| i.modifiers.shift);
                doc.search_step(forward, &self.engine);
                edit.request_focus();
            }
            if ui.button("⏶").on_hover_text("Previous match (Shift+Enter)").clicked() {
                doc.search_step(false, &self.engine);
            }
            if ui.button("⏷").on_hover_text("Next match (Enter)").clicked() {
                doc.search_step(true, &self.engine);
            }
            let mut changed = ui.checkbox(&mut doc.search.match_case, "Match case").changed();
            changed |= ui.checkbox(&mut doc.search.whole_word, "Whole words").changed();
            if changed {
                doc.search_changed();
            }
            let count = doc.search.count();
            if doc.search.is_running() {
                ui.spinner();
                ui.label(format!("{count} found, page {} of {}", doc.search.searched, doc.search.total_pages.max(1)));
            } else if !doc.search.query.trim().is_empty() && doc.search.done {
                match (count, doc.search.position()) {
                    (0, _) => ui.colored_label(ui.visuals().warn_fg_color, "Not found"),
                    (n, Some(p)) => ui.label(format!("{p} of {n}")),
                    (n, None) => ui.label(format!("{n} found")),
                };
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("✖").on_hover_text("Close (Esc)").clicked() {
                    doc.close_search(&self.engine);
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let night = self.night();
        let Some(doc) = self.doc.as_mut() else { return };
        let has_outline = !doc.info.outline.is_empty();
        if !has_outline {
            self.settings.sidebar_tab = 1;
        }
        ui.horizontal(|ui| {
            if has_outline && ui.selectable_label(self.settings.sidebar_tab == 0, "Outline").clicked() {
                self.settings.sidebar_tab = 0;
            }
            if ui.selectable_label(self.settings.sidebar_tab == 1, "Pages").clicked() {
                self.settings.sidebar_tab = 1;
            }
        });
        ui.separator();
        if self.settings.sidebar_tab == 0 && has_outline {
            let outline = doc.info.outline.clone();
            let current = doc.current_page;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                outline_ui(ui, &outline, doc, current, 0, &mut vec![]);
            });
        } else {
            let n = doc.page_count() as usize;
            let aspect = (0..n.min(1)).map(|p| doc.thumb_aspect(p as u32)).fold(1.414f32, |_, a| a).clamp(0.3, 3.0);
            let row = crate::view::THUMB_W * aspect + 30.0;
            let current = doc.current_page;
            let mut go = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row, n, |ui, range| {
                for p in range {
                    let p = p as u32;
                    ui.vertical_centered(|ui| {
                        let box_size = vec2(crate::view::THUMB_W, crate::view::THUMB_W * aspect);
                        let (rect, response) = ui.allocate_exact_size(box_size, egui::Sense::click());
                        let a = doc.thumb_aspect(p);
                        let size = if a > aspect { vec2(box_size.y / a, box_size.y) } else { vec2(box_size.x, box_size.x * a) };
                        let img_rect = egui::Rect::from_center_size(rect.center(), size);
                        let paper = night.map_or(Color32::WHITE, |s| s.paper());
                        ui.painter().rect_filled(img_rect, 0.0, paper);
                        if let Some((tex, _)) = doc.thumbnail(p, night) {
                            ui.painter().image(tex, img_rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        let stroke = if p == current {
                            egui::Stroke::new(2.5, ui.visuals().selection.bg_fill)
                        } else {
                            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
                        };
                        ui.painter().rect_stroke(img_rect, 0.0, stroke, egui::StrokeKind::Outside);
                        if response.clicked() {
                            go = Some(p);
                        }
                        ui.label(RichText::new((p + 1).to_string()).small());
                    });
                }
            });
            if let Some(p) = go {
                doc.go_to_page(p);
            }
        }
    }

    fn welcome(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.18).max(20.0));
            ui.heading(RichText::new("Papers").size(30.0));
            ui.label(RichText::new("A fast PDF reader with a smooth night mode").weak());
            ui.add_space(18.0);
            if let Some(error) = &self.engine_error {
                ui.colored_label(ui.visuals().error_fg_color, format!("The PDF engine could not be loaded: {error}"));
                ui.add_space(12.0);
            }
            if ui.add(egui::Button::new(RichText::new("Open a Document…").size(16.0)).min_size(vec2(200.0, 36.0))).clicked() {
                self.open_dialog();
            }
            ui.label(RichText::new("or drop a PDF file here").weak());
            if !self.settings.recent.is_empty() {
                ui.add_space(24.0);
                ui.label(RichText::new("Recent documents").strong());
                ui.add_space(4.0);
                let mut open = None;
                let mut forget = None;
                egui::ScrollArea::vertical().max_height(ui.available_height() - 20.0).show(ui, |ui| {
                    for r in self.settings.recent.iter().take(12) {
                        ui.horizontal(|ui| {
                            let width = 420.0f32.min(ui.available_width());
                            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                            let exists = r.path.exists();
                            let label = format!("{}   ·   page {}", r.display_name(), r.page + 1);
                            let b = ui.add_enabled(exists, egui::Button::new(label).min_size(vec2(width - 30.0, 26.0)));
                            if b.on_hover_text(r.path.display().to_string()).clicked() {
                                open = Some(r.clone());
                            }
                            if ui.small_button("✖").on_hover_text("Remove from the list").clicked() {
                                forget = Some(r.path.clone());
                            }
                        });
                    }
                });
                if let Some(r) = open {
                    self.open(r.path.clone(), Some(r));
                }
                if let Some(p) = forget {
                    self.settings.forget(&p);
                }
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(prompt) = &mut self.password {
            let mut submit = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("password")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.heading("Password Required");
                ui.label(format!("“{}” is protected with a password.", settings::file_name(&prompt.path)));
                if prompt.wrong {
                    ui.colored_label(ui.visuals().error_fg_color, "That password is not correct.");
                }
                let edit = ui.add(egui::TextEdit::singleline(&mut prompt.text).password(true).hint_text("Password").desired_width(f32::INFINITY));
                edit.request_focus();
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    submit = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Unlock").clicked() {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        cancel = true;
                    }
                });
            });
            if submit {
                let p = self.password.as_mut().expect("present");
                let (path, restore, password) = (p.path.clone(), p.restore.clone(), std::mem::take(&mut p.text));
                p.wrong = true;
                self.open_with_password(path, restore, Some(password));
            } else if cancel {
                self.password = None;
            }
        }
        if self.doc.is_some() && self.password.as_ref().is_some_and(|p| self.doc.as_ref().is_some_and(|d| d.info.path == p.path)) {
            self.password = None;
        }

        if self.show_properties
            && let Some(doc) = &self.doc
        {
            egui::Window::new("Properties").open(&mut self.show_properties).resizable(true).default_width(420.0).show(ctx, |ui| {
                egui::Grid::new("props").num_columns(2).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
                    for (k, v) in &doc.info.properties {
                        ui.label(RichText::new(k).strong());
                        ui.add(egui::Label::new(v).wrap().selectable(true));
                        ui.end_row();
                    }
                });
            });
        }

        if self.show_shortcuts {
            egui::Window::new("Keyboard Shortcuts").open(&mut self.show_shortcuts).resizable(false).show(ctx, |ui| {
                egui::Grid::new("keys").num_columns(2).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
                    for (k, v) in SHORTCUTS {
                        ui.label(RichText::new(*k).monospace());
                        ui.label(*v);
                        ui.end_row();
                    }
                });
            });
        }

        if self.show_about {
            egui::Window::new("About Papers").open(&mut self.show_about).resizable(false).collapsible(false).show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Papers");
                    ui.label(format!("Version {}", updates::VERSION));
                    ui.add_space(6.0);
                    ui.label("A fast document viewer written in Rust.\nPages render on a background thread, so scrolling,\nzooming and night mode never block the window.");
                    ui.add_space(6.0);
                    ui.label(RichText::new("PDF rendering by PDFium (BSD-3-Clause) · interface by egui (MIT/Apache-2.0)").small());
                    ui.label(RichText::new("License: GPL-2.0-or-later").small());
                    if ui.link("Source code and releases").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(format!("https://github.com/{}", papers_update::REPOSITORY)));
                    }
                });
            });
        }

        let mut auto = self.settings.check_updates;
        updates::window(ctx, &mut self.updates, &mut auto);
        self.settings.check_updates = auto;

        if let Some((text, at)) = &self.message {
            if at.elapsed() > Duration::from_secs(7) {
                self.message = None;
            } else {
                let text = text.clone();
                egui::Area::new(egui::Id::unique("toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
                    .interactable(true)
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(text);
                                if ui.small_button("✖").clicked() {
                                    self.message = None;
                                }
                            });
                        });
                    });
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
    }

    fn dropped_files(&mut self, ctx: &egui::Context) {
        let files: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        let mut files = files.into_iter();
        if let Some(first) = files.next() {
            self.open(first, None);
        }
        for f in files {
            open_in_new_window(Some(&f));
        }
    }
}

impl eframe::App for PapersApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_events();
        self.apply_theme(&ctx);
        self.updates.poll(&self.settings.skipped_version);
        self.updates.auto_check(&ctx, self.settings.check_updates, &mut self.settings.last_update_check);
        if let updates::Next::Quit = self.updates.finish(self.doc.as_ref().map(|d| d.info.path.clone())) {
            if let Some(doc) = &self.doc {
                self.settings.remember(doc.recent());
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.dropped_files(&ctx);
        self.shortcuts(&ctx);
        self.watch_file(&ctx);
        self.update_title(&ctx);
        let night = self.night();

        if let Some(doc) = self.doc.as_mut().filter(|d| d.presenting) {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ui, |ui| {
                for action in doc.ui(ui, &self.engine, night) {
                    run_action(&ctx, action);
                }
            });
            if !doc.presenting {
                self.set_fullscreen(&ctx, false);
            }
            return;
        }

        if !(self.fullscreen && self.doc.is_some()) || ui.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y < 40.0)) {
            egui::Panel::top("toolbar").frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 5))).show(ui, |ui| {
                self.toolbar(ui);
            });
        }
        if self.doc.as_ref().is_some_and(|d| d.search.open) {
            egui::Panel::top("search").frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 4))).show(ui, |ui| {
                self.search_bar(ui);
            });
        }
        let show_banner = self.updates.outcome.is_some()
            || (!self.updates.banner_dismissed && matches!(self.updates.state, updates::State::Available(_) | updates::State::Downloading { .. }));
        if show_banner {
            egui::Panel::top("updates").frame(egui::Frame::new()).show(ui, |ui| {
                if let Some(skip) = updates::banner(ui, &mut self.updates) {
                    self.settings.skipped_version = skip;
                }
            });
        }
        if self.settings.sidebar && self.doc.is_some() && !self.fullscreen {
            egui::Panel::left("sidebar").resizable(true).default_size(210.0).size_range(150.0..=480.0).show(ui, |ui| {
                self.sidebar(ui);
            });
        }
        let bg = if night.is_some() || ui.visuals().dark_mode { Color32::from_gray(38) } else { Color32::from_gray(222) };
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg)).show(ui, |ui| {
            if let Some(doc) = self.doc.as_mut() {
                for action in doc.ui(ui, &self.engine, night) {
                    run_action(&ctx, action);
                }
            } else {
                egui::Frame::new().fill(ui.visuals().panel_fill).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    self.welcome(ui);
                });
            }
        });
        self.dialogs(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(doc) = &self.doc {
            self.settings.remember(doc.recent());
        }
        eframe::set_value(storage, settings::KEY, &self.settings);
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(20)
    }
}

fn run_action(ctx: &egui::Context, action: Action) {
    match action {
        Action::OpenUrl(url) => {
            let lower = url.to_ascii_lowercase();
            // Only web and mail links; never launch local programs from a document.
            if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
        }
    }
}

fn outline_ui(ui: &mut Ui, items: &[OutlineItem], doc: &mut DocView, current: u32, depth: usize, path: &mut Vec<usize>) {
    for (i, item) in items.iter().enumerate() {
        path.push(i);
        let title = if item.title.is_empty() { "(untitled)" } else { item.title.as_str() };
        let is_here = item.dest.is_some_and(|d| d.page == current);
        let text = if is_here { RichText::new(title).strong() } else { RichText::new(title) };
        if item.children.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                let r = ui.add(egui::Button::selectable(false, text).frame(false));
                if r.clicked()
                    && let Some(d) = item.dest
                {
                    doc.go_to(d);
                }
            });
        } else {
            let id = ui.make_persistent_id(("outline", path.clone()));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth == 0 && items.len() < 12)
                .show_header(ui, |ui| {
                    let r = ui.add(egui::Button::selectable(false, text).frame(false));
                    if r.clicked()
                        && let Some(d) = item.dest
                    {
                        doc.go_to(d);
                    }
                })
                .body(|ui| outline_ui(ui, &item.children, doc, current, depth + 1, path));
        }
        path.pop();
    }
}

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+O", "Open a document"),
    ("Ctrl+N", "New window"),
    ("Ctrl+W", "Close the document"),
    ("Ctrl+R", "Reload"),
    ("Ctrl+P", "Print"),
    ("Ctrl+I", "Night mode on/off"),
    ("Ctrl+F, F3", "Find; Enter / Shift+Enter for next / previous"),
    ("Ctrl+C", "Copy selected text"),
    ("Ctrl+A", "Select the text of the current page"),
    ("Ctrl+L, Ctrl+G", "Go to page"),
    ("Ctrl+ +/−", "Zoom in / out (or Ctrl+wheel)"),
    ("Ctrl+0", "Fit width"),
    ("Ctrl+9", "Fit page"),
    ("Ctrl+1", "Actual size"),
    ("Ctrl+←/→", "Rotate"),
    ("Ctrl+D", "Dual pages"),
    ("N, P", "Next / previous page"),
    ("Space, PgDn / PgUp", "Scroll a screen"),
    ("Home, End", "Start / end of the document"),
    ("F9", "Sidebar"),
    ("F11", "Fullscreen"),
    ("F5", "Presentation"),
    ("Alt+Enter", "Properties"),
    ("Esc", "Clear selection, close find, leave fullscreen"),
];

fn style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.spacing.button_padding = vec2(7.0, 3.0);
        style.spacing.interact_size.y = 24.0;
    });
}

/// Folders searched for the PDFium library: next to the executable first.
fn library_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("PDFIUM_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
        dirs.push(dir.join("lib"));
    }
    dirs
}

fn default_recent(path: &Path) -> Recent {
    Recent { path: path.to_path_buf(), page: 0, offset: 0.0, zoom: 1.0, fit: Fit::Width, rotation: 0, title: None }
}

fn open_in_new_window(file: Option<&Path>) {
    if let Ok(exe) = std::env::current_exe() {
        let mut command = std::process::Command::new(exe);
        if let Some(f) = file {
            command.arg(f);
        }
        if let Err(e) = command.spawn() {
            log::error!("could not open a new window: {e}");
        }
    }
}
