//! The Papers window, laid out like GNOME Papers 50: a split view with a sidebar
//! (its own header bar, thumbnails / outline switched at the bottom, and a find
//! mode), the document header bar with the page selector and document menu,
//! floating zoom buttons, GNOME status pages and Adwaita dialogs.

use crate::adw::{self, Icon, Palette};
use crate::settings::{self, Fit, Recent, Settings};
use crate::updates::{self, Updates};
use crate::view::{Action, DocView};
use egui::{Align, Align2, Color32, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText, Sense, Ui, pos2, vec2};
use papers_render::{DocId, Engine, Event, NightStyle, OutlineItem, Request};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const PDF_EXTENSIONS: &[&str] = &["pdf", "ai"];
const SIDEBAR_WIDTH: f32 = 270.0;
const HELP_URL: &str = "https://help.gnome.org/papers/";

struct Opening {
    id: DocId,
    path: PathBuf,
    restore: Option<Recent>,
    started: Instant,
}

struct PasswordPrompt {
    path: PathBuf,
    restore: Option<Recent>,
    text: String,
    wrong: bool,
    dialog: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum SidebarPage {
    Thumbnails,
    Outline,
}

#[derive(Clone, Copy, PartialEq)]
enum Dialog {
    None,
    Properties,
    Shortcuts,
    About,
}

pub struct PapersApp {
    engine: Engine,
    settings: Settings,
    doc: Option<DocView>,
    opening: Option<Opening>,
    next_id: DocId,
    password: Option<PasswordPrompt>,
    error: Option<(String, PathBuf)>,
    toast: Option<(String, Instant)>,
    engine_error: Option<String>,
    updates: Updates,
    dialog: Dialog,
    page_text: String,
    page_focus: bool,
    find_mode: bool,
    sidebar_page: SidebarPage,
    fullscreen: bool,
    title: String,
    watched: Option<(PathBuf, Option<SystemTime>, Instant)>,
    applied_theme: Option<egui::ThemePreference>,
    icon: Option<egui::TextureHandle>,
    /// GNOME-style client-side decorations: header bars are the title bar.
    csd: bool,
    #[cfg(windows)]
    hwnd: isize,
}

impl PapersApp {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>, page: Option<u32>, csd: bool) -> Self {
        let settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, settings::KEY)).unwrap_or_default();
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        adw::install(&cc.egui_ctx);
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
        let sidebar_page = if settings.sidebar_tab == 1 { SidebarPage::Outline } else { SidebarPage::Thumbnails };
        let mut app = PapersApp {
            engine,
            settings,
            doc: None,
            opening: None,
            next_id: 1,
            password: None,
            error: None,
            toast: None,
            engine_error: None,
            updates: Updates::new(),
            dialog: Dialog::None,
            page_text: String::new(),
            page_focus: false,
            find_mode: false,
            sidebar_page,
            fullscreen: false,
            title: String::new(),
            watched: None,
            applied_theme: None,
            icon: None,
            csd,
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
        self.toast = Some((text.into(), Instant::now()));
    }

    fn night(&self) -> Option<NightStyle> {
        self.settings.night()
    }

    fn csd(&self) -> bool {
        self.csd
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
        self.error = None;
        self.opening = Some(Opening { id, path, restore, started: Instant::now() });
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

    fn save_as(&mut self) {
        let Some(doc) = &self.doc else { return };
        let src = doc.info.path.clone();
        let mut dialog = rfd::FileDialog::new().set_title("Save a Copy").add_filter("PDF documents", PDF_EXTENSIONS);
        if let Some(name) = src.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
        if let Some(dir) = src.parent() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(dest) = dialog.save_file() {
            if dest == src {
                return;
            }
            match std::fs::copy(&src, &dest) {
                Ok(_) => self.notify(format!("Saved as “{}”", settings::file_name(&dest))),
                Err(e) => self.notify(format!("The file could not be saved: {e}")),
            }
        }
    }

    fn open_with(&mut self) {
        let Some(doc) = &self.doc else { return };
        #[cfg(windows)]
        {
            let arg = format!("shell32.dll,OpenAs_RunDLL {}", doc.info.path.display());
            if let Err(e) = std::process::Command::new("rundll32.exe").arg(arg).spawn() {
                self.notify(format!("Could not show “Open With”: {e}"));
            }
        }
        #[cfg(not(windows))]
        {
            let _ = doc;
            self.notify("“Open With” is available on Windows.");
        }
    }

    fn close_document(&mut self) {
        if let Some(doc) = self.doc.take() {
            self.settings.remember(doc.recent());
            self.engine.send(Request::Close { id: doc.id() });
        }
        self.watched = None;
        self.find_mode = false;
        if self.dialog == Dialog::Properties {
            self.dialog = Dialog::None;
        }
    }

    fn reload(&mut self) {
        if let Some(doc) = &self.doc {
            let restore = doc.recent();
            let path = doc.info.path.clone();
            self.open(path, Some(restore));
        }
    }

    fn apply_view_settings(&mut self) {
        let s = &self.settings;
        if let Some(d) = self.doc.as_mut() {
            d.set_layout_options(s.continuous, s.dual, s.odd_left, s.rtl);
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
                    let find_mode = self.find_mode && reloading;
                    self.close_document();
                    self.find_mode = find_mode;
                    self.password = None;
                    let mut view = DocView::new(info, opening.restore.as_ref());
                    let s = &self.settings;
                    view.set_layout_options(s.continuous, s.dual, s.odd_left, s.rtl);
                    if let Some((open, query)) = search {
                        view.search.open = open;
                        view.search.query = query;
                        view.search_changed();
                    }
                    if view.info.outline.is_empty() {
                        self.sidebar_page = SidebarPage::Thumbnails;
                    }
                    let mtime = std::fs::metadata(&view.info.path).and_then(|m| m.modified()).ok();
                    self.watched = Some((view.info.path.clone(), mtime, Instant::now()));
                    self.settings.remember(view.recent());
                    self.doc = Some(view);
                }
                Event::OpenFailed { id, path, error, needs_password } => {
                    let Some(opening) = self.opening.take_if(|o| o.id == id) else { continue };
                    if needs_password {
                        let wrong = self.password.as_ref().is_some_and(|p| p.path == path);
                        self.password = Some(PasswordPrompt { path, restore: opening.restore, text: String::new(), wrong, dialog: true });
                    } else if self.doc.is_some() {
                        self.notify(format!("Unable to open “{}”: {error}", settings::file_name(&path)));
                    } else {
                        self.error = Some((error, path));
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
                    Ok(true) => self.notify("Sent to the printer"),
                    Ok(false) => {}
                    Err(e) => self.notify(format!("Printing failed: {e}")),
                },
            }
        }
    }

    /// Reload the document when it changes on disk, like Papers does.
    fn watch_file(&mut self, ctx: &egui::Context) {
        let Some((path, mtime, checked)) = &mut self.watched else { return };
        ctx.request_repaint_after(Duration::from_millis(1600));
        if checked.elapsed() < Duration::from_millis(1500) {
            return;
        }
        *checked = Instant::now();
        let now = std::fs::metadata(&*path).and_then(|m| m.modified()).ok();
        if now.is_some() && now != *mtime {
            *mtime = now;
            if self.opening.is_none() {
                self.reload();
            }
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let pref = match self.settings.theme {
            settings::Theme::System => egui::ThemePreference::System,
            settings::Theme::Light => egui::ThemePreference::Light,
            settings::Theme::Dark => egui::ThemePreference::Dark,
        };
        if self.applied_theme != Some(pref) {
            ctx.set_theme(pref);
            self.applied_theme = Some(pref);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match &self.doc {
            Some(d) => doc_title(d),
            None => "Document Viewer".to_string(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    fn start_presentation(&mut self, ctx: &egui::Context) {
        if let Some(d) = self.doc.as_mut() {
            d.presenting = true;
            self.set_fullscreen(ctx, true);
        }
    }

    fn open_find(&mut self) {
        if let Some(doc) = self.doc.as_mut() {
            self.find_mode = true;
            self.settings.sidebar = true;
            doc.search.open = true;
            doc.search.focus = true;
        }
    }

    fn close_find(&mut self) {
        self.find_mode = false;
        if let Some(doc) = self.doc.as_mut() {
            doc.close_search(&self.engine);
        }
    }

    fn print(&mut self) {
        let Some(doc) = &self.doc else { return };
        #[cfg(windows)]
        {
            let title = doc_title(doc);
            self.engine.send(Request::Print { doc: doc.id(), owner: self.hwnd, title });
        }
        #[cfg(not(windows))]
        {
            let _ = doc;
            self.notify("Printing is available on Windows");
        }
    }

    // ----- keyboard (the same shortcuts as GNOME Papers) -----

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
            self.settings.night = !self.settings.night;
        }
        if shortcut(Modifiers::NONE, Key::F11) {
            self.set_fullscreen(ctx, !self.fullscreen);
        }
        if shortcut(Modifiers::NONE, Key::F1) {
            ctx.open_url(egui::OpenUrl::new_tab(HELP_URL));
        }
        if shortcut(cmd, Key::Questionmark) || shortcut(cmd.plus(Modifiers::SHIFT), Key::Questionmark) {
            self.dialog = Dialog::Shortcuts;
        }
        if egui::Popup::is_any_open(ctx) {
            // Esc closes the open menu first.
            return;
        }
        if self.dialog != Dialog::None && shortcut(Modifiers::NONE, Key::Escape) {
            self.dialog = Dialog::None;
            return;
        }
        if self.doc.is_none() {
            return;
        }
        if self.doc.as_ref().is_some_and(|d| d.presenting) {
            let doc = self.doc.as_mut().expect("checked");
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
        }
        if shortcut(cmd, Key::S) || shortcut(cmd.plus(Modifiers::SHIFT), Key::S) {
            self.save_as();
        }
        if shortcut(cmd.plus(Modifiers::SHIFT), Key::N)
            && let Some(d) = &self.doc
        {
            open_in_new_window(Some(&d.info.path));
        }
        if shortcut(cmd.plus(Modifiers::SHIFT), Key::O) {
            self.open_with();
        }
        if shortcut(Modifiers::NONE, Key::F5) || shortcut(Modifiers::SHIFT, Key::F5) {
            self.start_presentation(ctx);
            return;
        }
        if shortcut(Modifiers::NONE, Key::F9) {
            self.settings.sidebar = !self.settings.sidebar;
        }
        if shortcut(cmd, Key::F) || key(Key::Slash) {
            self.open_find();
        }
        if shortcut(cmd.plus(Modifiers::SHIFT), Key::G) || shortcut(Modifiers::SHIFT, Key::F3) {
            self.find_step(false);
        } else if shortcut(cmd, Key::G) || shortcut(Modifiers::NONE, Key::F3) {
            self.find_step(true);
        }
        if shortcut(Modifiers::NONE, Key::Escape) {
            let doc = self.doc.as_mut().expect("checked");
            if doc.has_selection() {
                doc.clear_selection();
            } else if self.find_mode {
                self.close_find();
            } else if self.fullscreen {
                self.set_fullscreen(ctx, false);
            }
            return;
        }
        if shortcut(Modifiers::ALT, Key::Enter) {
            self.dialog = Dialog::Properties;
        }
        if shortcut(cmd, Key::L) {
            self.page_focus = true;
        }
        if !typing {
            let mut changed = false;
            if key(Key::C) {
                self.settings.continuous = !self.settings.continuous;
                changed = true;
            }
            if key(Key::D) {
                self.settings.dual = !self.settings.dual;
                changed = true;
            }
            if key(Key::O) {
                self.settings.odd_left = !self.settings.odd_left;
                changed = true;
            }
            if changed {
                self.apply_view_settings();
            }
        }
        let doc = self.doc.as_mut().expect("checked");
        if !typing {
            if (shortcut(cmd, Key::C) || shortcut(cmd, Key::Insert))
                && let Some(text) = doc.selected_text()
            {
                ctx.copy_text(text);
            }
            if ctx.input_mut(|i| i.consume_key(cmd, Key::A)) {
                doc.select_all_on_page();
            }
            if key(Key::W) {
                doc.set_fit(Fit::Width);
            }
            if key(Key::F) {
                doc.set_fit(Fit::Page);
            }
            if key(Key::A) {
                doc.set_fit(Fit::Width);
            }
            if key(Key::Plus) || key(Key::Equals) {
                doc.zoom_in();
            }
            if key(Key::Minus) {
                doc.zoom_out();
            }
        }
        if shortcut(cmd, Key::Plus) || shortcut(cmd, Key::Equals) || shortcut(cmd.plus(Modifiers::SHIFT), Key::Equals) {
            doc.zoom_in();
        }
        if shortcut(cmd, Key::Minus) {
            doc.zoom_out();
        }
        if shortcut(cmd, Key::Num0) {
            doc.set_zoom(1.0, None);
        }
        if shortcut(cmd, Key::ArrowLeft) {
            doc.rotate(false);
        }
        if shortcut(cmd, Key::ArrowRight) {
            doc.rotate(true);
        }
        if shortcut(cmd, Key::PageDown) || key(Key::N) {
            doc.next_page();
        }
        if shortcut(cmd, Key::PageUp) || key(Key::P) {
            doc.prev_page();
        }
        if shortcut(Modifiers::SHIFT, Key::PageDown) {
            doc.next_page();
        }
        if shortcut(Modifiers::SHIFT, Key::PageUp) {
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

    fn find_step(&mut self, forward: bool) {
        if !self.find_mode {
            self.open_find();
            return;
        }
        if let Some(doc) = self.doc.as_mut() {
            doc.search_step(forward, &self.engine);
        }
    }

    // ----- menus -----

    /// The main menu (open-menu in the sidebar header and on the start page).
    fn main_menu(&mut self, ui: &mut Ui, with_document: bool) {
        let button = adw::icon_button(ui, Icon::OpenMenu, "Main Menu", false);
        let mut open = false;
        let mut about = false;
        let mut shortcuts = false;
        let mut updates = false;
        let mut help = false;
        let mut recent = None;
        let night = self.settings.night;
        let mut toggle_night = false;
        let recents: Vec<Recent> = self.settings.recent.iter().take(8).cloned().collect();
        adw::popover_menu(&button, 260.0, |ui| {
            open = adw::menu_item(ui, "Open…", "Ctrl+O", Some(false), true);
            if !with_document && !recents.is_empty() {
                adw::menu_separator(ui);
                for r in &recents {
                    if adw::menu_item(ui, &r.display_name(), "", Some(false), r.path.exists()) {
                        recent = Some(r.clone());
                    }
                }
            }
            adw::menu_separator(ui);
            toggle_night = adw::menu_item(ui, "Night Mode", "Ctrl+I", Some(night), true);
            adw::menu_separator(ui);
            shortcuts = adw::menu_item(ui, "Keyboard Shortcuts", "Ctrl+?", Some(false), true);
            help = adw::menu_item(ui, "Help", "F1", Some(false), true);
            updates = adw::menu_item(ui, "Check for Updates…", "", Some(false), true);
            about = adw::menu_item(ui, "About Document Viewer", "", Some(false), true);
        });
        if open {
            self.open_dialog();
        }
        if let Some(r) = recent {
            self.open(r.path.clone(), Some(r));
        }
        if toggle_night {
            self.settings.night = !night;
        }
        if shortcuts {
            self.dialog = Dialog::Shortcuts;
        }
        if help {
            ui.ctx().open_url(egui::OpenUrl::new_tab(HELP_URL));
        }
        if updates {
            self.updates.check(ui.ctx(), true);
        }
        if about {
            self.dialog = Dialog::About;
        }
    }

    /// The document menu (view-more in the document header bar).
    fn document_menu(&mut self, ui: &mut Ui) {
        let button = adw::icon_button(ui, Icon::ViewMore, "Document Menu", false);
        let s = self.settings.clone();
        let fullscreen = self.fullscreen;
        #[derive(Default)]
        struct Chosen {
            print: bool,
            fullscreen: bool,
            present: bool,
            copy: bool,
            open_with: bool,
            save_as: bool,
            continuous: bool,
            dual: bool,
            odd_left: bool,
            rtl: bool,
            rotate: bool,
            properties: bool,
        }
        let mut c = Chosen::default();
        adw::popover_menu(&button, 280.0, |ui| {
            c.print = adw::menu_item(ui, "Print…", "Ctrl+P", Some(false), true);
            c.fullscreen = adw::menu_item(ui, "Fullscreen", "F11", Some(fullscreen), true);
            c.present = adw::menu_item(ui, "Present as Slideshow", "F5", Some(false), true);
            adw::menu_separator(ui);
            c.copy = adw::menu_item(ui, "Open a Copy", "Shift+Ctrl+N", Some(false), true);
            c.open_with = adw::menu_item(ui, "Open With…", "Shift+Ctrl+O", Some(false), cfg!(windows));
            c.save_as = adw::menu_item(ui, "Save As…", "Ctrl+S", Some(false), true);
            adw::menu_separator(ui);
            c.continuous = adw::menu_item(ui, "Continuous", "C", Some(s.continuous), true);
            c.dual = adw::menu_item(ui, "Dual", "D", Some(s.dual), true);
            c.odd_left = adw::menu_item(ui, "Odd Pages Left", "O", Some(s.odd_left), s.dual);
            c.rtl = adw::menu_item(ui, "Right to Left Document", "", Some(s.rtl), s.dual);
            adw::menu_separator(ui);
            c.rotate = adw::menu_item(ui, "Rotate ⤵", "Ctrl+→", Some(false), true);
            adw::menu_separator(ui);
            c.properties = adw::menu_item(ui, "Document Properties", "Alt+Enter", Some(false), true);
        });
        let ctx = ui.ctx().clone();
        if c.print {
            self.print();
        }
        if c.fullscreen {
            self.set_fullscreen(&ctx, !fullscreen);
        }
        if c.present {
            self.start_presentation(&ctx);
        }
        if c.copy
            && let Some(d) = &self.doc
        {
            open_in_new_window(Some(&d.info.path));
        }
        if c.open_with {
            self.open_with();
        }
        if c.save_as {
            self.save_as();
        }
        if c.continuous || c.dual || c.odd_left || c.rtl {
            self.settings.continuous ^= c.continuous;
            self.settings.dual ^= c.dual;
            self.settings.odd_left ^= c.odd_left;
            self.settings.rtl ^= c.rtl;
            self.apply_view_settings();
        }
        if c.rotate
            && let Some(d) = self.doc.as_mut()
        {
            d.rotate(true);
        }
        if c.properties {
            self.dialog = Dialog::Properties;
        }
    }

    /// Papers' page selector: a small entry with the page number and "of N".
    fn page_selector(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui);
        let Some(doc) = self.doc.as_mut() else { return };
        let count = doc.page_count();
        ui.label(RichText::new(format!("of {count}")).color(p.dim()));
        let id = ui.make_persistent_id("page-entry");
        let editing = ui.memory(|m| m.has_focus(id));
        if !editing {
            self.page_text = (doc.current_page + 1).to_string();
        }
        let (rect, _) = ui.allocate_exact_size(vec2(52.0, adw::BUTTON), Sense::hover());
        ui.painter().rect_filled(rect, 6.0, p.fg(0.1));
        if editing {
            ui.painter().rect_stroke(rect.expand(1.0), 7.0, egui::Stroke::new(2.0, p.accent_bg.gamma_multiply(0.6)), egui::StrokeKind::Outside);
        }
        let entry = ui.put(
            rect.shrink2(vec2(8.0, 0.0)),
            egui::TextEdit::singleline(&mut self.page_text)
                .id(id)
                .frame(egui::Frame::NONE)
                .align(Align2::RIGHT_CENTER),
        );
        let entry = entry.on_hover_text("Select Page");
        if std::mem::take(&mut self.page_focus) {
            entry.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(self.page_text.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), id);
            }
        }
        // Scrolling over the entry turns pages, as in Papers.
        if entry.hovered() {
            let dy = crate::adw::wheel_steps(ui);
            if dy > 0.0 {
                doc.prev_page();
            } else if dy < 0.0 {
                doc.next_page();
            }
        }
        if entry.lost_focus()
            && ui.input(|i| i.key_pressed(Key::Enter))
            && let Ok(n) = self.page_text.trim().parse::<u32>()
        {
            doc.go_to_page(n.saturating_sub(1));
        }
    }

    // ----- pages -----

    fn header_title(&self) -> String {
        self.doc.as_ref().map(doc_title).unwrap_or_default()
    }

    /// The document header bar: sidebar toggle · title · page selector, document menu, window buttons.
    fn document_header(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui);
        let title = self.header_title();
        let csd = self.csd();
        let sidebar = self.settings.sidebar;
        let mut toggle_sidebar = false;
        let header = adw::header_bar(
            ui,
            "document",
            p.headerbar_bg,
            |ui| {
                toggle_sidebar = adw::icon_button(ui, Icon::SidebarShow, "Toggle Sidebar", sidebar).clicked();
            },
            |ui| adw::window_title(ui, &title),
            |ui| {
                if csd {
                    adw::window_controls(ui);
                    ui.add_space(6.0);
                }
                self.document_menu(ui);
                self.page_selector(ui);
            },
        );
        if toggle_sidebar {
            self.settings.sidebar = !self.settings.sidebar;
        }
        if csd {
            adw::title_bar_behaviour(ui.ctx(), &header);
        }
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui);
        let find = self.find_mode;
        let mut toggle_find = false;
        let header = adw::header_bar(
            ui,
            "sidebar",
            p.sidebar_bg,
            |ui| {
                toggle_find = adw::icon_button(ui, Icon::EditFind, "Search", find).clicked();
            },
            |ui| adw::window_title(ui, "Document Viewer"),
            |ui| self.main_menu(ui, true),
        );
        if self.csd() {
            adw::title_bar_behaviour(ui.ctx(), &header);
        }
        if toggle_find {
            if self.find_mode {
                self.close_find();
            } else {
                self.open_find();
            }
        }
        if self.find_mode {
            self.find_sidebar(ui);
            return;
        }
        let has_outline = self.doc.as_ref().is_some_and(|d| !d.info.outline.is_empty());
        let switcher_h = adw::BUTTON + 12.0;
        let area = ui.available_rect_before_wrap();
        let content = egui::Rect::from_min_max(area.min, pos2(area.right(), area.bottom() - switcher_h));
        let mut content_ui = ui.new_child(egui::UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Min)));
        content_ui.set_clip_rect(content);
        match self.sidebar_page {
            SidebarPage::Outline if has_outline => self.outline(&mut content_ui),
            _ => self.thumbnails(&mut content_ui),
        }
        let bar = egui::Rect::from_min_max(pos2(area.left(), area.bottom() - switcher_h), area.max);
        let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink(6.0)).layout(Layout::left_to_right(Align::Center)));
        let mut items = vec![(Icon::GridLarge, "Thumbnails")];
        if has_outline {
            items.push((Icon::Outline, "Outline"));
        }
        let selected = if self.sidebar_page == SidebarPage::Outline && has_outline { 1 } else { 0 };
        if let Some(i) = adw::view_switcher(&mut bar_ui, &items, selected) {
            self.sidebar_page = if i == 1 { SidebarPage::Outline } else { SidebarPage::Thumbnails };
            self.settings.sidebar_tab = i as u8;
        }
    }

    fn thumbnails(&mut self, ui: &mut Ui) {
        let night = self.night();
        let Some(doc) = self.doc.as_mut() else { return };
        let p = Palette::of(ui);
        let n = doc.page_count() as usize;
        let thumb_w = crate::view::THUMB_W;
        let aspect = doc.thumb_aspect(0).clamp(0.3, 3.0);
        let row_h = thumb_w * aspect + 12.0 + 6.0 + 22.0;
        let current = doc.current_page;
        let mut go = None;
        let scroll_to_current = ui.memory_mut(|m| {
            let id = egui::Id::unique("thumbs-follow");
            let last: Option<u32> = m.data.get_temp(id);
            m.data.insert_temp(id, current);
            last != Some(current)
        });
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("thumbnails");
        if scroll_to_current {
            // Keep the current page in view, like Papers' thumbnail list.
            let target = current as f32 * row_h;
            let view_h = ui.available_height();
            let offset = ui.ctx().memory(|m| m.data.get_temp::<f32>(egui::Id::unique("thumbs-offset"))).unwrap_or(0.0);
            if target < offset || target + row_h > offset + view_h {
                area = area.vertical_scroll_offset((target - view_h / 2.0 + row_h / 2.0).max(0.0));
            }
        }
        let out = area.show_rows(ui, row_h, n, |ui, range| {
            for page in range {
                let page = page as u32;
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                let card = rect.shrink2(vec2(12.0, 3.0));
                let selected = page == current;
                if selected || response.hovered() {
                    ui.painter().rect_filled(card, 12.0, p.fg(if selected { 0.10 } else { 0.05 }));
                }
                let box_rect = egui::Rect::from_min_size(pos2(card.center().x - thumb_w / 2.0, card.top() + 12.0), vec2(thumb_w, thumb_w * aspect));
                let a = doc.thumb_aspect(page);
                let size = if a > aspect { vec2(box_rect.height() / a, box_rect.height()) } else { vec2(box_rect.width(), box_rect.width() * a) };
                let img = egui::Rect::from_center_size(box_rect.center(), size);
                let paper = night.map_or(Color32::WHITE, |s| s.paper());
                ui.painter().rect_filled(img, 6.0, paper);
                if let Some((tex, _)) = doc.thumbnail(page, night) {
                    let mut mesh = egui::Mesh::with_texture(tex);
                    mesh.add_rect_with_uv(img, egui::Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                    ui.painter().add(egui::Shape::mesh(mesh));
                }
                let border = if night.is_some() { Color32::WHITE.gamma_multiply(0.3) } else { p.fg(0.15) };
                ui.painter().rect_stroke(img, 6.0, egui::Stroke::new(1.0, border), egui::StrokeKind::Outside);
                ui.painter().text(
                    pos2(card.center().x, card.bottom() - 13.0),
                    Align2::CENTER_CENTER,
                    (page + 1).to_string(),
                    egui::FontId::proportional(13.0),
                    p.window_fg,
                );
                if response.clicked() {
                    go = Some(page);
                }
            }
        });
        ui.ctx().memory_mut(|m| m.data.insert_temp(egui::Id::unique("thumbs-offset"), out.state.clamped_offset().y));
        if let Some(page) = go {
            doc.go_to_page(page);
        }
    }

    fn outline(&mut self, ui: &mut Ui) {
        let Some(doc) = self.doc.as_mut() else { return };
        let outline = doc.info.outline.clone();
        let current = doc.current_page;
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("outline").show(ui, |ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            outline_rows(ui, &outline, doc, current, 0, &mut vec![]);
            ui.add_space(6.0);
        });
    }

    /// Papers' find sidebar: a search box, then the results with their surrounding text.
    fn find_sidebar(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui);
        let Some(doc) = self.doc.as_mut() else { return };
        ui.add_space(6.0);
        let mut options_changed = false;
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let id = ui.make_persistent_id("find-entry");
            let width = ui.available_width() - adw::BUTTON - 18.0;
            let entry = adw::search_entry(ui, &mut doc.search.query, "Search in document", id, width);
            if std::mem::take(&mut doc.search.focus) {
                entry.request_focus();
            }
            if entry.changed() {
                doc.search_changed();
            }
            if entry.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let forward = !ui.input(|i| i.modifiers.shift);
                doc.search_step(forward, &self.engine);
                entry.request_focus();
            }
            let options = adw::icon_button(ui, Icon::ViewMore, "Search Options", false);
            let (whole, case) = (doc.search.whole_word, doc.search.match_case);
            adw::popover_menu(&options, 220.0, |ui| {
                if adw::menu_item(ui, "Whole Words Only", "", Some(whole), true) {
                    doc.search.whole_word = !whole;
                    options_changed = true;
                }
                if adw::menu_item(ui, "Case Sensitive", "", Some(case), true) {
                    doc.search.match_case = !case;
                    options_changed = true;
                }
            });
        });
        if options_changed {
            doc.search_changed();
        }
        ui.add_space(6.0);
        let query_empty = doc.search.query.trim().is_empty();
        let count = doc.search.count();
        if query_empty {
            ui.add_space(ui.available_height() * 0.2);
            adw::status_page(ui, Some(Icon::EditFind), "Perform a Search", "The search results will appear here", true, |_| {});
            return;
        }
        if count == 0 {
            if doc.search.is_running() || doc.search.edited_pending() {
                ui.add_space(ui.available_height() * 0.3);
                ui.vertical_centered(|ui| ui.add(egui::Spinner::new().size(32.0)));
            } else {
                ui.add_space(ui.available_height() * 0.2);
                adw::status_page(ui, Some(Icon::EditFind), "No Results Found", "Try a different search", true, |_| {});
            }
            return;
        }
        let flat: Vec<(u32, usize)> = doc.search.results.iter().flat_map(|(page, ms)| (0..ms.len()).map(move |i| (*page, i))).collect();
        let current = doc.search.current;
        let mut chosen = None;
        let follow = ui.memory_mut(|m| {
            let id = egui::Id::unique("find-follow");
            let last: Option<(u32, usize)> = m.data.get_temp(id);
            m.data.insert_temp(id, current);
            last != current
        });
        let row_h = 40.0;
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("find-results");
        if follow
            && let Some(cur) = current
            && let Some(idx) = flat.iter().position(|x| *x == cur)
        {
            area = area.vertical_scroll_offset((idx as f32 * row_h - ui.available_height() / 2.0).max(0.0));
        }
        area.show_rows(ui, row_h, flat.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for &(page, i) in &flat[range] {
                let m = &doc.search.results[&page][i];
                let selected = current == Some((page, i));
                let response = adw::sidebar_row(ui, row_h, selected, |painter, r, p| {
                    let page_label = (page + 1).to_string();
                    let page_galley = painter.layout_no_wrap(page_label, egui::FontId::proportional(13.0), p.dim());
                    let text_w = (r.width() - page_galley.size().x - 12.0).max(20.0);
                    let mut job = egui::text::LayoutJob::default();
                    let normal = egui::TextFormat { font_id: egui::FontId::proportional(14.0), color: p.window_fg, ..Default::default() };
                    let strong = egui::TextFormat { font_id: adw::bold(14.0), color: p.window_fg, ..Default::default() };
                    let (a, b) = (m.range.start.min(m.context.len()), m.range.end.min(m.context.len()));
                    job.append(&m.context[..a], 0.0, normal.clone());
                    job.append(&m.context[a..b], 0.0, strong);
                    job.append(&m.context[b..], 0.0, normal);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
                    let galley = painter.layout_job(job);
                    painter.galley(pos2(r.left(), r.center().y - galley.size().y / 2.0), galley, p.window_fg);
                    painter.galley(pos2(r.right() - page_galley.size().x, r.center().y - page_galley.size().y / 2.0), page_galley, p.dim());
                });
                if response.clicked() {
                    chosen = Some((page, i));
                }
            }
        });
        if let Some((page, i)) = chosen {
            doc.select_match(page, i);
        }
        let _ = p;
    }

    /// GNOME's start page: "Open a Document".
    fn start_page(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui);
        let csd = self.csd();
        let header = adw::header_bar(
            ui,
            "start",
            p.window_bg,
            |_| {},
            |ui| adw::window_title(ui, "Document Viewer"),
            |ui| {
                if csd {
                    adw::window_controls(ui);
                    ui.add_space(6.0);
                }
                self.main_menu(ui, false);
            },
        );
        if csd {
            adw::title_bar_behaviour(ui.ctx(), &header);
        }
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, p.window_bg);
        let mut open = false;
        let mut open_recent = None;
        let mut forget = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let height = ui.available_height().max(rect.height());
            ui.vertical_centered(|ui| {
                let has_recent = !self.settings.recent.is_empty();
                ui.add_space(((height - if has_recent { 640.0 } else { 360.0 }) / 2.0).max(24.0));
                if let Some((error, path)) = &self.error {
                    let description = format!("“{}”: {error}", settings::file_name(path));
                    adw::status_page(ui, Some(Icon::DialogError), "Unable to Open Document", &description, false, |ui| {
                        open = adw::pill_button(ui, "Open Other File…", true).clicked();
                    });
                } else if let Some(engine_error) = &self.engine_error {
                    adw::status_page(ui, Some(Icon::DialogError), "The PDF Engine Could Not Be Loaded", engine_error, false, |_| {});
                } else {
                    adw::status_page(ui, Some(Icon::Document), "Open a Document", "Drag and drop documents here", false, |ui| {
                        open = adw::pill_button(ui, "Open…", true).on_hover_text("Open a Document").clicked();
                    });
                }
                if has_recent {
                    ui.add_space(30.0);
                    let width = 420.0f32.min(ui.available_width() - 24.0);
                    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(width);
                        adw::group_title(ui, "Recent Documents");
                        adw::boxed_list(ui, |list| {
                            for r in self.settings.recent.iter().take(6) {
                                let resp = list.action(&r.display_name(), &format!("Page {}", r.page + 1));
                                let resp = resp.on_hover_text(r.path.display().to_string());
                                if resp.clicked() {
                                    if r.path.exists() {
                                        open_recent = Some(r.clone());
                                    } else {
                                        forget = Some(r.path.clone());
                                    }
                                }
                                resp.context_menu(|ui| {
                                    if adw::menu_item(ui, "Remove From List", "", None, true) {
                                        forget = Some(r.path.clone());
                                    }
                                });
                            }
                        });
                    });
                }
                ui.add_space(24.0);
            });
        });
        if open {
            self.open_dialog();
        }
        if let Some(r) = open_recent {
            self.open(r.path.clone(), Some(r));
        }
        if let Some(p) = forget {
            self.settings.forget(&p);
        }
    }

    /// "Loading Document", shown when opening takes a moment.
    fn loading_page(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui);
        let path = self.opening.as_ref().map(|o| o.path.display().to_string()).unwrap_or_default();
        adw::header_bar(ui, "loading", p.window_bg, |_| {}, |ui| adw::window_title(ui, "Document Viewer"), |ui| {
            if self.csd {
                adw::window_controls(ui);
            }
        });
        let mut cancel = false;
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 140.0).max(20.0));
            adw::status_page(ui, None, "Loading Document", &path, false, |ui| {
                ui.add(egui::Spinner::new().size(64.0));
                ui.add_space(24.0);
                cancel = adw::pill_button(ui, "Cancel", false).clicked();
            });
        });
        if cancel {
            self.opening = None;
        }
    }

    /// "Password Required", with the unlock dialog.
    fn password_page(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui);
        adw::header_bar(ui, "password", p.window_bg, |_| {}, |ui| adw::window_title(ui, "Password Required"), |ui| {
            if self.csd {
                adw::window_controls(ui);
            }
        });
        let mut unlock = false;
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 200.0).max(20.0));
            adw::status_page(
                ui,
                Some(Icon::Password),
                "Password Required",
                "This document is locked and can only be read by entering the correct password",
                false,
                |ui| unlock = adw::pill_button(ui, "Unlock Document", false).clicked(),
            );
        });
        if unlock && let Some(prompt) = self.password.as_mut() {
            prompt.dialog = true;
        }
    }

    // ----- dialogs -----

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.dialog != Dialog::None || self.password.as_ref().is_some_and(|p| p.dialog) || self.updates.window {
            egui::Popup::close_all(ctx);
        }
        let dark = ctx.global_style().visuals.dark_mode;
        self.password_dialog(ctx, dark);
        match self.dialog {
            Dialog::Properties => self.properties_dialog(ctx, dark),
            Dialog::Shortcuts => self.shortcuts_dialog(ctx, dark),
            Dialog::About => self.about_dialog(ctx, dark),
            Dialog::None => {}
        }
        let mut auto = self.settings.check_updates;
        updates::window(ctx, &mut self.updates, &mut auto);
        self.settings.check_updates = auto;
        if let Some((text, at)) = &self.toast {
            if at.elapsed() > Duration::from_secs(6) {
                self.toast = None;
            } else {
                let text = text.clone();
                if adw::toast(ctx, &text) {
                    self.toast = None;
                }
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
    }

    fn password_dialog(&mut self, ctx: &egui::Context, dark: bool) {
        let Some(prompt) = self.password.as_mut().filter(|p| p.dialog) else { return };
        let mut submit = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("password-dialog")).frame(adw::dialog_frame(dark)).show(ctx, |ui| {
            let p = Palette::of(ui);
            ui.set_width(340.0);
            egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top: 24, bottom: 12 }).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Password Required").font(adw::bold(20.0)));
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(format!(
                        "The document “{}” is locked and requires a password before it can be opened.",
                        settings::file_name(&prompt.path)
                    )).wrap());
                    ui.add_space(14.0);
                    let id = ui.make_persistent_id("password-entry");
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), adw::BUTTON), Sense::hover());
                    ui.painter().rect_filled(rect, 6.0, p.fg(0.1));
                    let edit = ui.put(
                        rect.shrink2(vec2(10.0, 0.0)),
                        egui::TextEdit::singleline(&mut prompt.text).id(id).password(true).hint_text("Password").frame(egui::Frame::NONE).align(Align2::LEFT_CENTER),
                    );
                    edit.request_focus();
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        submit = true;
                    }
                    if prompt.wrong {
                        ui.add_space(6.0);
                        ui.colored_label(p.destructive, "Wrong password, try again");
                    }
                });
            });
            adw_dialog_responses(ui, &mut cancel, &mut submit, "Cancel", "Unlock");
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
        });
        if submit {
            let prompt = self.password.as_mut().expect("present");
            let (path, restore, password) = (prompt.path.clone(), prompt.restore.clone(), std::mem::take(&mut prompt.text));
            prompt.dialog = false;
            self.open_with_password(path, restore, Some(password));
        } else if cancel {
            let prompt = self.password.as_mut().expect("present");
            prompt.dialog = false;
            if self.doc.is_some() {
                self.password = None;
            }
        }
    }

    fn properties_dialog(&mut self, ctx: &egui::Context, dark: bool) {
        let Some(doc) = &self.doc else {
            self.dialog = Dialog::None;
            return;
        };
        let groups = doc.info.properties.clone();
        let mut close = false;
        let max_h = (ctx.content_rect().height() - 80.0).clamp(200.0, 640.0);
        let response = egui::Modal::new(egui::Id::unique("properties-dialog")).frame(adw::dialog_frame(dark)).show(ctx, |ui| {
            ui.set_width(440.0);
            close = adw::dialog_header(ui, "Properties");
            egui::ScrollArea::vertical().max_height(max_h - adw::HEADER_HEIGHT).show(ui, |ui| {
                egui::Frame::new().inner_margin(Margin { left: 18, right: 18, top: 0, bottom: 18 }).show(ui, |ui| {
                    for g in &groups {
                        adw::group_title(ui, &g.title);
                        adw::boxed_list(ui, |list| {
                            for (k, v) in &g.rows {
                                list.property(k, v);
                            }
                        });
                    }
                });
            });
        });
        if close || response.should_close() {
            self.dialog = Dialog::None;
        }
    }

    fn shortcuts_dialog(&mut self, ctx: &egui::Context, dark: bool) {
        let mut close = false;
        let max_h = (ctx.content_rect().height() - 80.0).clamp(200.0, 680.0);
        let response = egui::Modal::new(egui::Id::unique("shortcuts-dialog")).frame(adw::dialog_frame(dark)).show(ctx, |ui| {
            ui.set_width(460.0);
            close = adw::dialog_header(ui, "Keyboard Shortcuts");
            egui::ScrollArea::vertical().max_height(max_h - adw::HEADER_HEIGHT).show(ui, |ui| {
                egui::Frame::new().inner_margin(Margin { left: 18, right: 18, top: 0, bottom: 18 }).show(ui, |ui| {
                    for (group, rows) in SHORTCUTS {
                        adw::group_title(ui, group);
                        adw::boxed_list(ui, |list| {
                            for (title, accel) in *rows {
                                list.row(title, |ui| {
                                    let mut alts: Vec<&str> = accel.split(" / ").collect();
                                    // The row lays out right to left; keep the reading order.
                                    alts.reverse();
                                    for (i, alt) in alts.into_iter().enumerate() {
                                        if i > 0 {
                                            ui.label(RichText::new("or").color(Palette::of(ui).dim()));
                                        }
                                        adw::keycaps(ui, alt);
                                    }
                                });
                            }
                        });
                    }
                });
            });
        });
        if close || response.should_close() {
            self.dialog = Dialog::None;
        }
    }

    fn about_dialog(&mut self, ctx: &egui::Context, dark: bool) {
        let icon = self
            .icon
            .get_or_insert_with(|| ctx.load_texture("app-icon", load_png(include_bytes!("../assets/icon-256.png")), egui::TextureOptions::LINEAR))
            .id();
        let mut close = false;
        let response = egui::Modal::new(egui::Id::unique("about-dialog")).frame(adw::dialog_frame(dark)).show(ctx, |ui| {
            let p = Palette::of(ui);
            ui.set_width(360.0);
            close = adw::dialog_header(ui, "");
            egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 24 }).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add(egui::Image::new((icon, vec2(128.0, 128.0))));
                    ui.add_space(12.0);
                    ui.label(RichText::new("Document Viewer").font(adw::bold(26.0)));
                    ui.label(RichText::new("Papers for Windows").color(p.dim()));
                    ui.add_space(10.0);
                    egui::Frame::new().fill(p.accent_bg.gamma_multiply(0.15)).corner_radius(20).inner_margin(Margin::symmetric(12, 4)).show(ui, |ui| {
                        ui.label(RichText::new(updates::VERSION).color(p.accent).font(adw::bold(14.0)));
                    });
                    ui.add_space(18.0);
                });
                adw::boxed_list(ui, |list| {
                    if list.action("Website", "").clicked() {
                        list.ui.ctx().open_url(egui::OpenUrl::new_tab(format!("https://github.com/{}", papers_update::REPOSITORY)));
                    }
                    if list.action("Report an Issue", "").clicked() {
                        list.ui.ctx().open_url(egui::OpenUrl::new_tab(format!("https://github.com/{}/issues", papers_update::REPOSITORY)));
                    }
                });
                ui.add_space(14.0);
                ui.vertical_centered(|ui| {
                    ui.add(egui::Label::new(RichText::new(
                        "A Rust rebuild of GNOME Papers' interface. PDF rendering by PDFium (BSD-3-Clause), \
                         text in Adwaita Sans (SIL Open Font License).\nThis program comes with absolutely no warranty. \
                         See the GNU General Public License, version 2 or later, for details.",
                    ).size(12.5).color(p.dim())).wrap());
                });
            });
        });
        if close || response.should_close() {
            self.dialog = Dialog::None;
        }
    }

    fn dropped_files(&mut self, ctx: &egui::Context) {
        let files: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        let mut files = files.into_iter();
        if let Some(first) = files.next() {
            self.open(first, None);
        }
        for f in files {
            open_in_new_window(Some(&f));
        }
    }

    /// Papers' floating zoom buttons in the bottom-right corner of the document.
    fn zoom_overlay(&mut self, ctx: &egui::Context, area: egui::Rect) {
        let Some(doc) = self.doc.as_mut() else { return };
        let show_fit = doc.fit == Fit::None;
        egui::Area::new(egui::Id::unique("zoom-overlay"))
            .fixed_pos(pos2(area.right() - 18.0 - 40.0, area.bottom() - 18.0 - (if show_fit { 3.0 } else { 2.0 }) * 40.0 - 24.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 9.0;
                ui.vertical(|ui| {
                    if show_fit && adw::circular_card_button(ui, Icon::ZoomFitBest, "Fit Automatically", true).clicked() {
                        doc.set_fit(Fit::Width);
                    }
                    ui.add_space(6.0);
                    if adw::circular_card_button(ui, Icon::ZoomIn, "Zoom In", doc.zoom < 63.9).clicked() {
                        doc.zoom_in();
                    }
                    if adw::circular_card_button(ui, Icon::ZoomOut, "Zoom Out", doc.zoom > 0.051).clicked() {
                        doc.zoom_out();
                    }
                });
            });
    }
}

fn adw_dialog_responses(ui: &mut Ui, cancel: &mut bool, ok: &mut bool, cancel_label: &str, ok_label: &str) {
    let p = Palette::of(ui);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 50.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, p.fg(0.1)));
    ui.painter().vline(rect.center().x, rect.y_range(), egui::Stroke::new(1.0, p.fg(0.1)));
    let left = egui::Rect::from_min_max(rect.min, pos2(rect.center().x, rect.bottom()));
    let right = egui::Rect::from_min_max(pos2(rect.center().x, rect.top()), rect.max);
    for (r, label, suggested, flag) in [(left, cancel_label, false, &mut *cancel), (right, ok_label, true, &mut *ok)] {
        let resp = ui.interact(r, ui.make_persistent_id(("response", label)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r.shrink(1.0), 0.0, p.fg(0.05));
        }
        let color = if suggested { p.accent } else { p.window_fg };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, label, adw::bold(14.5), color);
        if resp.clicked() {
            *flag = true;
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
        if self.csd() && !self.fullscreen {
            adw::resize_edges(&ctx);
        }
        let night = self.night();
        let p = Palette::of(ui);

        if let Some(doc) = self.doc.as_mut().filter(|d| d.presenting) {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ui, |ui| {
                for action in doc.ui(ui, &self.engine, night) {
                    run_action(&ctx, action);
                }
            });
            if !doc.presenting {
                self.set_fullscreen(&ctx, false);
            }
            self.dialogs(&ctx);
            return;
        }

        if self.doc.is_none() {
            let loading = self.opening.as_ref().is_some_and(|o| o.started.elapsed() > Duration::from_millis(200));
            if self.opening.is_some() && !loading {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            egui::CentralPanel::default().frame(egui::Frame::new().fill(p.window_bg)).show(ui, |ui| {
                if self.password.is_some() {
                    self.password_page(ui);
                } else if loading {
                    self.loading_page(ui);
                } else {
                    self.start_page(ui);
                }
            });
            self.dialogs(&ctx);
            return;
        }

        let chrome = !self.fullscreen || ui.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y < 48.0));
        if self.settings.sidebar && !self.fullscreen {
            egui::Panel::left("sidebar")
                .exact_size(SIDEBAR_WIDTH)
                .resizable(false)
                .frame(egui::Frame::new().fill(p.sidebar_bg).stroke(egui::Stroke::new(1.0, p.sidebar_border)))
                .show(ui, |ui| self.sidebar(ui));
        }
        let mut central = egui::Rect::NOTHING;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.doc_bg)).show(ui, |ui| {
            if chrome {
                self.document_header(ui);
            }
            let show_banner = self.updates.outcome.is_some()
                || (!self.updates.banner_dismissed
                    && matches!(self.updates.state, updates::State::Available(_) | updates::State::Downloading { .. }));
            if show_banner && let Some(skip) = updates::banner(ui, &mut self.updates) {
                self.settings.skipped_version = skip;
            }
            central = ui.available_rect_before_wrap();
            if let Some(doc) = self.doc.as_mut() {
                for action in doc.ui(ui, &self.engine, night) {
                    run_action(&ctx, action);
                }
            }
        });
        if central.is_positive() {
            self.zoom_overlay(&ctx, central);
        }
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

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        Palette::for_dark(visuals.dark_mode).window_bg.to_normalized_gamma_f32()
    }
}

fn doc_title(d: &DocView) -> String {
    d.info.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| settings::file_name(&d.info.path))
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
        Action::Copy(text) => ctx.copy_text(text),
    }
}

/// Outline rows: expander, title, and the page number on the right, as in Papers.
fn outline_rows(ui: &mut Ui, items: &[OutlineItem], doc: &mut DocView, current: u32, depth: usize, path: &mut Vec<usize>) {
    for (i, item) in items.iter().enumerate() {
        path.push(i);
        let id = ui.make_persistent_id(("outline", path.clone()));
        let has_children = !item.children.is_empty();
        let mut open: bool = ui.data(|d| d.get_temp(id)).unwrap_or(depth == 0 && items.len() < 12);
        let title = if item.title.is_empty() { "(untitled)".to_string() } else { item.title.clone() };
        let here = item.dest.is_some_and(|d| d.page == current);
        let indent = depth as f32 * 16.0;
        let mut toggle = false;
        let response = adw::sidebar_row(ui, 36.0, false, |painter, r, p| {
            let x = r.left() + indent;
            if has_children {
                let icon = if open { Icon::PanDown } else { Icon::PanEnd };
                adw::paint_icon(painter, egui::Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), icon, p.dim());
            }
            let page = item.dest.map(|d| (d.page + 1).to_string()).unwrap_or_default();
            let page_g = painter.layout_no_wrap(page, egui::FontId::proportional(13.0), p.dim());
            let text_x = x + 22.0;
            let font = if here { adw::bold(14.0) } else { egui::FontId::proportional(14.0) };
            let mut job = egui::text::LayoutJob::simple_singleline(title.clone(), font, p.window_fg);
            job.wrap = egui::text::TextWrapping::truncate_at_width((r.right() - text_x - page_g.size().x - 10.0).max(20.0));
            let g = painter.layout_job(job);
            painter.galley(pos2(text_x, r.center().y - g.size().y / 2.0), g, p.window_fg);
            painter.galley(pos2(r.right() - page_g.size().x, r.center().y - page_g.size().y / 2.0), page_g, p.dim());
        });
        if response.clicked() {
            let pointer = response.interact_pointer_pos().map(|p| p.x).unwrap_or(f32::MAX);
            if has_children && pointer < response.rect.left() + 14.0 + indent + 22.0 {
                toggle = true;
            } else if let Some(d) = item.dest {
                doc.go_to(d);
            } else if has_children {
                toggle = true;
            }
        }
        if response.double_clicked() && has_children {
            toggle = true;
        }
        if toggle {
            open = !open;
            ui.data_mut(|d| d.insert_temp(id, open));
        }
        if has_children && open {
            outline_rows(ui, &item.children, doc, current, depth + 1, path);
        }
        path.pop();
    }
}

const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Documents",
        &[
            ("Open a document", "Ctrl+O"),
            ("Open a copy of the document", "Shift+Ctrl+N"),
            ("Save a copy", "Ctrl+S"),
            ("Print", "Ctrl+P"),
            ("Close the window", "Ctrl+W"),
            ("Reload", "Ctrl+R"),
            ("Document properties", "Alt+Enter"),
        ],
    ),
    (
        "Moving Around",
        &[
            ("Next page", "N / Ctrl+Page Down"),
            ("Previous page", "P / Ctrl+Page Up"),
            ("Go to page", "Ctrl+L"),
            ("Scroll a screen", "Space / Page Down"),
            ("Beginning and end", "Home / End"),
        ],
    ),
    (
        "Find and Select",
        &[
            ("Find", "Ctrl+F / /"),
            ("Next match", "Ctrl+G / F3"),
            ("Previous match", "Shift+Ctrl+G / Shift+F3"),
            ("Copy", "Ctrl+C"),
            ("Select all", "Ctrl+A"),
        ],
    ),
    (
        "View",
        &[
            ("Zoom in", "Ctrl++ / +"),
            ("Zoom out", "Ctrl+− / −"),
            ("Actual size", "Ctrl+0"),
            ("Fit width", "W"),
            ("Fit page", "F"),
            ("Rotate", "Ctrl+← / Ctrl+→"),
            ("Continuous", "C"),
            ("Dual pages", "D"),
            ("Odd pages left", "O"),
            ("Sidebar", "F9"),
            ("Night mode", "Ctrl+I"),
            ("Fullscreen", "F11"),
            ("Present as slideshow", "F5"),
        ],
    ),
    ("General", &[("Keyboard shortcuts", "Ctrl+?"), ("Help", "F1"), ("New window", "Ctrl+N"), ("Quit", "Ctrl+Q")]),
];

fn load_png(bytes: &[u8]) -> egui::ColorImage {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let Ok(mut reader) = decoder.read_info() else { return egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]) };
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let Ok(info) = reader.next_frame(&mut buf) else { return egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]) };
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]),
    };
    egui::ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], &rgba)
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
