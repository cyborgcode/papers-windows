//! A small Adwaita (GNOME) look for egui: the libadwaita palette and font,
//! symbolic icons drawn as vectors, and the widgets Papers is built from —
//! header bars, flat and circular buttons, pill buttons, popover menus,
//! status pages, boxed lists, an inline view switcher, dialogs and toasts.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin, Pos2,
    Rect, Response, RichText, Sense, Shadow, Stroke, StrokeKind, TextStyle, Ui, Vec2, pos2, vec2,
};
use std::sync::Arc;

pub const HEADER_HEIGHT: f32 = 47.0;
pub const BUTTON: f32 = 34.0;

/// libadwaita's named colours (1.7), for one colour scheme.
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub window_bg: Color32,
    pub window_fg: Color32,
    pub headerbar_bg: Color32,
    pub headerbar_shade: Color32,
    pub sidebar_bg: Color32,
    pub sidebar_border: Color32,
    pub card_bg: Color32,
    pub popover_bg: Color32,
    pub dialog_bg: Color32,
    pub accent_bg: Color32,
    pub accent: Color32,
    pub destructive: Color32,
    pub warning: Color32,
    /// The document area behind the pages.
    pub doc_bg: Color32,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        dark: false,
        window_bg: Color32::from_rgb(0xfa, 0xfa, 0xfb),
        window_fg: Color32::from_rgba_premultiplied(0, 0, 5, 204),
        headerbar_bg: Color32::WHITE,
        headerbar_shade: Color32::from_rgba_premultiplied(0, 0, 1, 31),
        sidebar_bg: Color32::from_rgb(0xeb, 0xeb, 0xed),
        sidebar_border: Color32::from_rgba_premultiplied(0, 0, 1, 18),
        card_bg: Color32::WHITE,
        popover_bg: Color32::WHITE,
        dialog_bg: Color32::from_rgb(0xfa, 0xfa, 0xfb),
        accent_bg: Color32::from_rgb(0x35, 0x84, 0xe4),
        accent: Color32::from_rgb(0x04, 0x61, 0xbe),
        destructive: Color32::from_rgb(0xc0, 0x1c, 0x28),
        warning: Color32::from_rgb(0x9c, 0x6e, 0x03),
        doc_bg: Color32::from_rgb(0xf0, 0xf0, 0xf2),
    };

    pub const DARK: Palette = Palette {
        dark: true,
        window_bg: Color32::from_rgb(0x22, 0x22, 0x26),
        window_fg: Color32::WHITE,
        headerbar_bg: Color32::from_rgb(0x2e, 0x2e, 0x32),
        headerbar_shade: Color32::from_rgba_premultiplied(0, 0, 3, 92),
        sidebar_bg: Color32::from_rgb(0x2e, 0x2e, 0x32),
        sidebar_border: Color32::from_rgba_premultiplied(0, 0, 3, 92),
        card_bg: Color32::from_rgb(0x36, 0x36, 0x3a),
        popover_bg: Color32::from_rgb(0x36, 0x36, 0x3a),
        dialog_bg: Color32::from_rgb(0x36, 0x36, 0x3a),
        accent_bg: Color32::from_rgb(0x35, 0x84, 0xe4),
        accent: Color32::from_rgb(0x81, 0xd0, 0xff),
        destructive: Color32::from_rgb(0xff, 0x93, 0x8c),
        warning: Color32::from_rgb(0xf8, 0xe4, 0x5c),
        doc_bg: Color32::from_rgb(0x1d, 0x1d, 0x20),
    };

    pub fn of(ui: &Ui) -> Palette {
        Self::for_dark(ui.visuals().dark_mode)
    }

    pub fn for_dark(dark: bool) -> Palette {
        if dark { Self::DARK } else { Self::LIGHT }
    }

    /// The foreground colour at a given opacity (Adwaita draws most chrome this way).
    pub fn fg(&self, alpha: f32) -> Color32 {
        let base = if self.dark { Color32::WHITE } else { Color32::from_rgb(0, 0, 6) };
        base.gamma_multiply(alpha)
    }

    pub fn dim(&self) -> Color32 {
        self.fg(0.55)
    }
}

/// Install Adwaita Sans and the Adwaita-like style for both light and dark mode.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "adwaita-sans".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/fonts/AdwaitaSans-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "adwaita-sans-bold".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/fonts/AdwaitaSans-Bold.ttf"))),
    );
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "adwaita-sans".into());
    let mut bold = vec!["adwaita-sans-bold".to_string()];
    bold.extend(fonts.families[&FontFamily::Proportional].iter().skip(1).cloned());
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    ctx.set_fonts(fonts);

    for dark in [false, true] {
        let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
        ctx.style_mut_of(theme, |style| apply_style(style, Palette::for_dark(dark)));
    }
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

fn apply_style(style: &mut egui::Style, p: Palette) {
    use egui::style::{Selection, WidgetVisuals};
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Body, FontId::proportional(14.5)),
        (TextStyle::Button, FontId::proportional(14.5)),
        (TextStyle::Heading, bold(14.5)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    style.spacing.item_spacing = vec2(6.0, 6.0);
    style.spacing.button_padding = vec2(10.0, 5.0);
    style.spacing.interact_size = vec2(32.0, 32.0);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.window_margin = Margin::same(12);
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    style.interaction.selectable_labels = false;

    let v = &mut style.visuals;
    *v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.window_bg;
    v.window_fill = p.popover_bg;
    v.faint_bg_color = p.fg(0.04);
    v.extreme_bg_color = p.fg(0.08);
    v.code_bg_color = p.fg(0.08);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.destructive;
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(12);
    v.window_stroke = Stroke::new(1.0, p.fg(if p.dark { 0.0 } else { 0.06 }));
    v.window_shadow = Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(if p.dark { 110 } else { 45 }) };
    v.popup_shadow = Shadow { offset: [0, 2], blur: 12, spread: 0, color: Color32::from_black_alpha(if p.dark { 100 } else { 40 }) };
    v.selection = Selection { bg_fill: p.accent_bg.gamma_multiply(0.45), stroke: Stroke::new(1.0, p.accent) };
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.striped = false;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    let fg = p.window_fg;
    let w = |bg: Color32| WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: bg,
        bg_stroke: Stroke::NONE,
        corner_radius: CornerRadius::same(6),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    };
    v.widgets.noninteractive = WidgetVisuals { bg_stroke: Stroke::new(1.0, p.fg(0.12)), ..w(p.window_bg) };
    v.widgets.inactive = w(p.fg(0.10));
    v.widgets.hovered = w(p.fg(0.15));
    v.widgets.active = w(p.fg(0.30));
    v.widgets.open = w(p.fg(0.15));
    v.override_text_color = Some(fg);
}

// ---------------------------------------------------------------- icons

/// GNOME-style symbolic icons, drawn on a 16×16 grid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    OpenMenu,
    ViewMore,
    SidebarShow,
    EditFind,
    ZoomIn,
    ZoomOut,
    ZoomFitBest,
    GridLarge,
    Outline,
    Document,
    DialogError,
    Password,
    WindowClose,
    WindowMinimize,
    WindowMaximize,
    WindowRestore,
    EditClear,
    Check,
    PanEnd,
    PanDown,
}

pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 16.0;
    // Snap the 16×16 grid to physical pixels so straight strokes stay sharp.
    let ppp = painter.pixels_per_point();
    let o = rect.center() - vec2(8.0, 8.0) * s;
    let o = pos2((o.x * ppp).round() / ppp, (o.y * ppp).round() / ppp);
    let p = |x: f32, y: f32| o + vec2(x, y) * s;
    let w = (1.6 * s).max(1.2);
    let st = Stroke::new(w, color);
    let line = |a: Pos2, b: Pos2| painter.line_segment([a, b], st);
    let round_rect = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        painter.rect_stroke(Rect::from_min_max(p(x0, y0), p(x1, y1)), r * s, st, StrokeKind::Middle);
    };
    let filled = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        painter.rect_filled(Rect::from_min_max(p(x0, y0), p(x1, y1)), r * s, color);
    };
    let magnifier = || {
        painter.circle_stroke(p(7.0, 7.0), 5.0 * s, Stroke::new(1.5 * s, color));
        painter.line_segment([p(10.7, 10.7), p(14.5, 14.5)], Stroke::new(2.0 * s, color));
    };
    match icon {
        Icon::OpenMenu => {
            for y in [4.0, 8.0, 12.0] {
                filled(2.0, y - 0.9, 14.0, y + 0.9, 0.9);
            }
        }
        Icon::ViewMore => {
            for y in [3.0, 8.0, 13.0] {
                painter.circle_filled(p(8.0, y), 1.5 * s, color);
            }
        }
        Icon::SidebarShow => {
            round_rect(1.5, 2.5, 14.5, 13.5, 2.0);
            filled(2.0, 3.0, 6.2, 13.0, 1.0);
        }
        Icon::EditFind => magnifier(),
        Icon::ZoomIn | Icon::ZoomOut => {
            magnifier();
            // A 6×2 bar centred in the lens, on whole pixels; the vertical bar of the
            // plus is drawn in two halves so the translucent colour never overlaps.
            filled(4.0, 6.0, 10.0, 8.0, 0.0);
            if icon == Icon::ZoomIn {
                filled(6.0, 4.0, 8.0, 6.0, 0.0);
                filled(6.0, 8.0, 8.0, 10.0, 0.0);
            }
        }
        Icon::ZoomFitBest => {
            for (cx, cy, dx, dy) in [(2.0, 2.0, 1.0, 1.0), (14.0, 2.0, -1.0, 1.0), (2.0, 14.0, 1.0, -1.0), (14.0, 14.0, -1.0, -1.0)] {
                line(p(cx, cy), p(cx + 4.0 * dx, cy));
                line(p(cx, cy), p(cx, cy + 4.0 * dy));
            }
            round_rect(5.5, 5.5, 10.5, 10.5, 1.0);
        }
        Icon::GridLarge => {
            for (x, y) in [(2.0, 2.0), (9.0, 2.0), (2.0, 9.0), (9.0, 9.0)] {
                round_rect(x, y, x + 5.0, y + 5.0, 1.2);
            }
        }
        Icon::Outline => {
            for (i, y) in [3.5, 8.0, 12.5].into_iter().enumerate() {
                painter.circle_filled(p(2.8 + i as f32 * 1.6, y), 1.1 * s, color);
                filled(5.5 + i as f32 * 1.6, y - 0.8, 14.0, y + 0.8, 0.8);
            }
        }
        Icon::Document => {
            let pts = vec![p(3.0, 1.5), p(9.5, 1.5), p(13.0, 5.0), p(13.0, 14.5), p(3.0, 14.5), p(3.0, 1.5)];
            painter.add(egui::Shape::line(pts, st));
            painter.add(egui::Shape::line(vec![p(9.5, 1.5), p(9.5, 5.0), p(13.0, 5.0)], st));
            for y in [8.0, 10.5] {
                line(p(5.5, y), p(10.5, y));
            }
        }
        Icon::DialogError => {
            painter.circle_stroke(p(8.0, 8.0), 6.5 * s, st);
            line(p(8.0, 4.5), p(8.0, 9.0));
            painter.circle_filled(p(8.0, 11.4), 0.95 * s, color);
        }
        Icon::Password => {
            round_rect(3.0, 7.0, 13.0, 14.5, 1.5);
            painter.add(egui::Shape::line(
                (0..=16).map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
                    p(8.0 + 3.2 * a.cos(), 7.0 + 3.6 * a.sin())
                }).collect(),
                st,
            ));
            painter.circle_filled(p(8.0, 10.8), 1.1 * s, color);
        }
        Icon::WindowClose | Icon::EditClear => {
            let (a, b) = if icon == Icon::EditClear { (5.2, 10.8) } else { (4.3, 11.7) };
            if icon == Icon::EditClear {
                painter.circle_filled(p(8.0, 8.0), 6.5 * s, color.gamma_multiply(0.35));
            }
            line(p(a, a), p(b, b));
            line(p(b, a), p(a, b));
        }
        Icon::WindowMinimize => {
            line(p(4.0, 11.0), p(12.0, 11.0));
        }
        Icon::WindowMaximize => round_rect(4.0, 4.0, 12.0, 12.0, 1.0),
        Icon::WindowRestore => {
            round_rect(3.5, 5.5, 10.5, 12.5, 1.0);
            painter.add(egui::Shape::line(vec![p(5.5, 3.5), p(12.5, 3.5), p(12.5, 10.5)], st));
        }
        Icon::Check => {
            painter.add(egui::Shape::line(vec![p(3.0, 8.5), p(6.5, 12.0), p(13.0, 4.5)], Stroke::new(w * 1.2, color)));
        }
        Icon::PanEnd => {
            painter.add(egui::Shape::convex_polygon(vec![p(6.0, 4.0), p(11.0, 8.0), p(6.0, 12.0)], color, Stroke::NONE));
        }
        Icon::PanDown => {
            painter.add(egui::Shape::convex_polygon(vec![p(4.0, 6.0), p(12.0, 6.0), p(8.0, 11.0)], color, Stroke::NONE));
        }
    }
}

// ---------------------------------------------------------------- buttons

/// A flat header-bar style icon button (Adwaita `.flat`), optionally toggled.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str, checked: bool) -> Response {
    icon_button_sized(ui, icon, tooltip, checked, vec2(BUTTON, BUTTON), true)
}

pub fn icon_button_sized(ui: &mut Ui, icon: Icon, tooltip: &str, checked: bool, size: Vec2, enabled: bool) -> Response {
    let p = Palette::of(ui);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let response = if tooltip.is_empty() { response } else { response.on_hover_text(tooltip) };
    if ui.is_rect_visible(rect) {
        let bg = if !enabled {
            Color32::TRANSPARENT
        } else if response.is_pointer_button_down_on() {
            p.fg(0.25)
        } else if checked && response.hovered() {
            p.fg(0.18)
        } else if checked {
            p.fg(0.12)
        } else if response.hovered() {
            p.fg(0.07)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 6.0, bg);
        let color = if enabled { p.window_fg } else { p.fg(0.3) };
        paint_icon(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, color);
        if enabled && response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Default);
        }
    }
    response
}

/// A round button that floats over the document (Adwaita `.circular.card`).
pub fn circular_card_button(ui: &mut Ui, icon: Icon, tooltip: &str, enabled: bool) -> Response {
    let p = Palette::of(ui);
    let size = vec2(40.0, 40.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let response = response.on_hover_text(tooltip);
    let painter = ui.painter();
    let base = if p.dark { Color32::from_rgb(0x2f, 0x2f, 0x33) } else { Color32::WHITE };
    let bg = if !enabled {
        base
    } else if response.is_pointer_button_down_on() {
        blend(base, p.fg(1.0), 0.18)
    } else if response.hovered() {
        blend(base, p.fg(1.0), 0.07)
    } else {
        base
    };
    painter.add(Shadow { offset: [0, 1], blur: 6, spread: 0, color: Color32::from_black_alpha(if p.dark { 90 } else { 40 }) }.as_shape(rect, CornerRadius::same(20)));
    painter.circle_filled(rect.center(), 20.0, bg);
    painter.circle_stroke(rect.center(), 20.0, Stroke::new(1.0, p.fg(if p.dark { 0.06 } else { 0.08 })));
    let color = if enabled { p.window_fg } else { p.fg(0.3) };
    paint_icon(painter, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, color);
    response
}

/// A rounded pill button; `suggested` uses the accent colour (Adwaita `.pill.suggested-action`).
pub fn pill_button(ui: &mut Ui, text: &str, suggested: bool) -> Response {
    let p = Palette::of(ui);
    let galley = ui.painter().layout_no_wrap(text.to_string(), bold(14.5), Color32::WHITE);
    let size = vec2(galley.size().x + 64.0, 40.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let (bg, fg) = if suggested {
        let base = p.accent_bg;
        let bg = if response.is_pointer_button_down_on() {
            blend(base, Color32::BLACK, 0.2)
        } else if response.hovered() {
            blend(base, Color32::WHITE, 0.1)
        } else {
            base
        };
        (bg, Color32::WHITE)
    } else {
        let bg = if response.is_pointer_button_down_on() {
            p.fg(0.3)
        } else if response.hovered() {
            p.fg(0.15)
        } else {
            p.fg(0.1)
        };
        (bg, p.window_fg)
    };
    ui.painter().rect_filled(rect, 20.0, bg);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, bold(14.5), fg);
    response
}

/// A normal rounded text button (Adwaita default button).
pub fn text_button(ui: &mut Ui, text: &str, suggested: bool, enabled: bool) -> Response {
    let p = Palette::of(ui);
    let galley = ui.painter().layout_no_wrap(text.to_string(), bold(14.5), Color32::WHITE);
    let size = vec2(galley.size().x + 32.0, BUTTON);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let down = enabled && response.is_pointer_button_down_on();
    let hover = enabled && response.hovered();
    let (bg, fg) = if suggested {
        let base = if enabled { p.accent_bg } else { p.accent_bg.gamma_multiply(0.5) };
        (if down { blend(base, Color32::BLACK, 0.2) } else if hover { blend(base, Color32::WHITE, 0.1) } else { base }, Color32::WHITE)
    } else {
        (if down { p.fg(0.3) } else if hover { p.fg(0.15) } else { p.fg(0.1) }, if enabled { p.window_fg } else { p.fg(0.4) })
    };
    ui.painter().rect_filled(rect, 6.0, bg);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, bold(14.5), fg);
    response
}

/// Mouse-wheel movement this frame from raw wheel events (positive = up), without smoothing.
pub fn wheel_steps(ui: &Ui) -> f32 {
    ui.input(|i| {
        i.events
            .iter()
            .map(|e| match e {
                egui::Event::MouseWheel { delta, .. } => delta.y,
                _ => 0.0,
            })
            .sum()
    })
}

pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

// ---------------------------------------------------------------- header bars

/// What happened on the empty part of a header bar (it doubles as the window title bar).
pub struct HeaderBar {
    pub background: Response,
}

/// Draw an Adwaita header bar across `rect` and lay out start / title / end parts.
/// The empty background can be dragged to move the window and double-clicked to maximise.
pub fn header_bar(
    ui: &mut Ui,
    id: &str,
    bg: Color32,
    start: impl FnOnce(&mut Ui),
    title: impl FnOnce(&mut Ui),
    end: impl FnOnce(&mut Ui),
) -> HeaderBar {
    let p = Palette::of(ui);
    let full = ui.available_rect_before_wrap();
    let rect = Rect::from_min_size(full.min, vec2(full.width(), HEADER_HEIGHT));
    let background = ui.interact(rect, ui.make_persistent_id(("headerbar", id)), Sense::click_and_drag());
    ui.painter().rect_filled(rect, 0.0, bg);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.headerbar_shade));
    let inner = rect.shrink2(vec2(6.0, 6.0));
    let mut start_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    start_ui.spacing_mut().item_spacing.x = 6.0;
    start(&mut start_ui);
    let start_right = start_ui.min_rect().right();
    let mut end_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)));
    end_ui.spacing_mut().item_spacing.x = 6.0;
    end(&mut end_ui);
    let end_left = end_ui.min_rect().left();
    // The title is centred on the whole bar when it fits, else between the two sides.
    let side = (inner.center().x - start_right).min(end_left - inner.center().x).max(0.0);
    let title_rect = Rect::from_center_size(rect.center(), vec2((side * 2.0 - 12.0).max(0.0), HEADER_HEIGHT - 12.0));
    let mut title_ui = ui.new_child(egui::UiBuilder::new().max_rect(title_rect).layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)));
    title(&mut title_ui);
    ui.advance_cursor_after_rect(rect);
    HeaderBar { background }
}

/// Adwaita's `WindowTitle`: a bold, centred title that elides.
pub fn window_title(ui: &mut Ui, title: &str) {
    let label = egui::Label::new(RichText::new(title).font(bold(14.5))).truncate().selectable(false);
    ui.add(label);
}

/// Minimise / maximise / close in the Adwaita style (small round buttons).
pub fn window_controls(ui: &mut Ui) {
    let p = Palette::of(ui);
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    let controls = [
        (Icon::WindowClose, "Close"),
        (if maximized { Icon::WindowRestore } else { Icon::WindowMaximize }, if maximized { "Restore" } else { "Maximize" }),
        (Icon::WindowMinimize, "Minimize"),
    ];
    for (icon, tip) in controls {
        let (rect, response) = ui.allocate_exact_size(vec2(26.0, BUTTON), Sense::click());
        let response = response.on_hover_text(tip);
        let circle = Rect::from_center_size(rect.center(), vec2(24.0, 24.0));
        let bg = if response.is_pointer_button_down_on() {
            p.fg(0.3)
        } else if response.hovered() {
            p.fg(0.15)
        } else {
            p.fg(0.1)
        };
        ui.painter().circle_filled(circle.center(), 12.0, bg);
        paint_icon(ui.painter(), Rect::from_center_size(circle.center(), vec2(16.0, 16.0)), icon, p.window_fg);
        if response.clicked() {
            let cmd = match icon {
                Icon::WindowClose => egui::ViewportCommand::Close,
                Icon::WindowMinimize => egui::ViewportCommand::Minimized(true),
                _ => egui::ViewportCommand::Maximized(!maximized),
            };
            ui.ctx().send_viewport_cmd(cmd);
        }
    }
}

/// Make a header bar act as the title bar: drag to move, double-click to maximise.
pub fn title_bar_behaviour(ctx: &egui::Context, header: &HeaderBar) {
    if header.background.double_clicked() {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if header.background.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

/// Resize handles along the edges of an undecorated window.
pub fn resize_edges(ctx: &egui::Context) {
    use egui::viewport::ResizeDirection as D;
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let r = ctx.content_rect();
    let m = 5.0;
    let (l, rr, t, b) = (pos.x - r.left() < m, r.right() - pos.x < m, pos.y - r.top() < m, r.bottom() - pos.y < m);
    let dir = match (l, rr, t, b) {
        (true, _, true, _) => Some((D::NorthWest, CursorIcon::ResizeNorthWest)),
        (_, true, true, _) => Some((D::NorthEast, CursorIcon::ResizeNorthEast)),
        (true, _, _, true) => Some((D::SouthWest, CursorIcon::ResizeSouthWest)),
        (_, true, _, true) => Some((D::SouthEast, CursorIcon::ResizeSouthEast)),
        (true, ..) => Some((D::West, CursorIcon::ResizeWest)),
        (_, true, ..) => Some((D::East, CursorIcon::ResizeEast)),
        (_, _, true, _) => Some((D::North, CursorIcon::ResizeNorth)),
        (_, _, _, true) => Some((D::South, CursorIcon::ResizeSouth)),
        _ => None,
    };
    if let Some((dir, cursor)) = dir {
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }
}

// ---------------------------------------------------------------- menus

/// The Adwaita popover frame.
pub fn popover_frame(ctx: &egui::Context) -> Frame {
    let style = ctx.global_style();
    let p = Palette::for_dark(style.visuals.dark_mode);
    Frame::new()
        .fill(p.popover_bg)
        .corner_radius(12)
        .inner_margin(Margin::same(6))
        .stroke(Stroke::new(1.0, p.fg(if p.dark { 0.08 } else { 0.07 })))
        .shadow(style.visuals.popup_shadow)
}

/// Open a popover menu under `button` (toggled by clicking it).
pub fn popover_menu<R>(button: &Response, width: f32, content: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    egui::Popup::menu(button)
        .frame(popover_frame(&button.ctx))
        .width(width)
        .gap(4.0)
        .align(egui::RectAlign::BOTTOM_END)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_min_width(width);
            content(ui)
        })
        .map(|r| r.inner)
}

/// One menu row. `check` shows a check mark when `Some(true)`, and reserves space when `Some(false)`.
pub fn menu_item(ui: &mut Ui, label: &str, accel: &str, check: Option<bool>, enabled: bool) -> bool {
    let p = Palette::of(ui);
    let width = ui.available_width().max(160.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && (response.hovered() || response.has_focus()) {
        ui.painter().rect_filled(rect, 6.0, p.fg(if response.is_pointer_button_down_on() { 0.16 } else { 0.07 }));
    }
    let fg = if enabled { p.window_fg } else { p.fg(0.35) };
    let mut x = rect.left() + 12.0;
    if let Some(on) = check {
        if on {
            paint_icon(ui.painter(), Rect::from_center_size(pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0)), Icon::Check, fg);
        }
        x += 24.0;
    }
    ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.5), fg);
    if !accel.is_empty() {
        ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, accel, FontId::proportional(13.0), p.dim());
    }
    let clicked = enabled && response.clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// The line between menu sections.
pub fn menu_separator(ui: &mut Ui) {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 13.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.fg(0.1)));
}

// ---------------------------------------------------------------- pages and lists

/// Adwaita `StatusPage`: a large dim icon, a title, a description and optional content.
pub fn status_page(ui: &mut Ui, icon: Option<Icon>, title: &str, description: &str, compact: bool, child: impl FnOnce(&mut Ui)) {
    let p = Palette::of(ui);
    let icon_size = if compact { 64.0 } else { 128.0 };
    let title_size = if compact { 20.0 } else { 26.0 };
    ui.vertical_centered(|ui| {
        if let Some(icon) = icon {
            let (rect, _) = ui.allocate_exact_size(vec2(icon_size, icon_size), Sense::hover());
            // Opaque colour, so overlapping strokes do not show darker joints.
            let base = if p.dark { Color32::WHITE } else { Color32::from_rgb(0, 0, 6) };
            paint_icon(ui.painter(), rect, icon, blend(p.window_bg, base, 0.35));
            ui.add_space(if compact { 12.0 } else { 24.0 });
        }
        ui.label(RichText::new(title).font(bold(title_size)));
        if !description.is_empty() {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(description).size(if compact { 14.5 } else { 15.5 })).wrap());
        }
        ui.add_space(if compact { 12.0 } else { 30.0 });
        child(ui);
    });
}

/// Adwaita `.boxed-list` card holding rows separated by lines.
pub fn boxed_list(ui: &mut Ui, rows: impl FnOnce(&mut BoxedList<'_>)) {
    let p = Palette::of(ui);
    let fill = if p.dark { p.fg(0.08) } else { p.card_bg };
    Frame::new()
        .fill(fill)
        .corner_radius(12)
        .stroke(Stroke::new(1.0, p.fg(if p.dark { 0.0 } else { 0.07 })))
        .shadow(Shadow { offset: [0, 1], blur: 3, spread: 0, color: Color32::from_black_alpha(if p.dark { 0 } else { 14 }) })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(ui.available_width());
            let mut list = BoxedList { ui, first: true, palette: p };
            rows(&mut list);
        });
}

pub struct BoxedList<'a> {
    pub ui: &'a mut Ui,
    first: bool,
    palette: Palette,
}

impl BoxedList<'_> {
    fn separator(&mut self) {
        if !self.first {
            let r = self.ui.available_rect_before_wrap();
            self.ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, self.palette.fg(0.1)));
        }
        self.first = false;
    }

    /// A property row: small dim title over a selectable value (Adwaita `.property`).
    pub fn property(&mut self, title: &str, value: &str) {
        self.separator();
        Frame::new().inner_margin(Margin::symmetric(12, 8)).show(self.ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(title).size(12.5).color(self.palette.dim()));
            ui.add(egui::Label::new(value).wrap().selectable(true));
        });
    }

    /// A clickable row with a title and an optional trailing text. Returns the response.
    pub fn action(&mut self, title: &str, trailing: &str) -> Response {
        self.separator();
        let ui = &mut *self.ui;
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, 50.0), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, self.palette.fg(0.04));
        }
        ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, title, FontId::proportional(14.5), self.palette.window_fg);
        if !trailing.is_empty() {
            let max = (width * 0.45).max(60.0);
            let font = FontId::proportional(13.0);
            let mut job = egui::text::LayoutJob::simple_singleline(trailing.to_string(), font, self.palette.dim());
            job.wrap = egui::text::TextWrapping::truncate_at_width(max);
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(pos2(rect.right() - 12.0 - galley.size().x, rect.center().y - galley.size().y / 2.0), galley, self.palette.dim());
        }
        response
    }

    /// A row with a title on the left and arbitrary content on the right.
    pub fn row(&mut self, title: &str, right: impl FnOnce(&mut Ui)) {
        self.separator();
        Frame::new().inner_margin(Margin::symmetric(12, 6)).show(self.ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(34.0);
                ui.label(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
            });
        });
    }
}

/// A group heading above a boxed list (Adwaita `PreferencesGroup` title).
pub fn group_title(ui: &mut Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(title).font(bold(14.5)));
    ui.add_space(6.0);
}

/// Keyboard shortcut keycaps (`.keycap`).
pub fn keycaps(ui: &mut Ui, accel: &str) {
    let p = Palette::of(ui);
    let mut keys: Vec<&str> = accel.split('+').map(str::trim).collect();
    // "Ctrl++" is Ctrl and the plus key.
    if accel.ends_with("++") {
        keys.retain(|k| !k.is_empty());
        keys.push("+");
    }
    if ui.layout().prefer_right_to_left() {
        keys.reverse();
    }
    for (i, key) in keys.into_iter().enumerate() {
        if i > 0 {
            ui.label(RichText::new("+").color(p.dim()));
        }
        Frame::new()
            .fill(p.fg(0.06))
            .stroke(Stroke::new(1.0, p.fg(0.15)))
            .corner_radius(6)
            .inner_margin(Margin::symmetric(7, 2))
            .show(ui, |ui| {
                ui.label(RichText::new(key).size(13.0));
            });
    }
}

/// Adwaita's `InlineViewSwitcher` (icons only, flat). Returns the chosen index.
pub fn view_switcher(ui: &mut Ui, items: &[(Icon, &str)], selected: usize) -> Option<usize> {
    let p = Palette::of(ui);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, BUTTON), Sense::hover());
    let n = items.len().max(1) as f32;
    let w = rect.width() / n;
    let mut chosen = None;
    for (i, (icon, tip)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + w * i as f32, rect.top()), vec2(w, rect.height())).shrink2(vec2(2.0, 0.0));
        let response = ui.interact(r, ui.make_persistent_id(("switcher", i)), Sense::click()).on_hover_text(*tip);
        let bg = if i == selected {
            p.fg(if response.hovered() { 0.18 } else { 0.12 })
        } else if response.hovered() {
            p.fg(0.07)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(r, 6.0, bg);
        paint_icon(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), *icon, p.window_fg);
        if response.clicked() {
            chosen = Some(i);
        }
    }
    chosen
}

/// A navigation-sidebar row (rounded highlight when selected or hovered).
pub fn sidebar_row(ui: &mut Ui, height: f32, selected: bool, paint: impl FnOnce(&egui::Painter, Rect, Palette)) -> Response {
    let p = Palette::of(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let row = rect.shrink2(vec2(6.0, 1.0));
    let bg = if selected {
        p.fg(if response.hovered() { 0.13 } else { 0.10 })
    } else if response.hovered() {
        p.fg(0.05)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(row, 6.0, bg);
    paint(ui.painter(), row.shrink2(vec2(8.0, 0.0)), p);
    response
}

/// A rounded search entry with a magnifier and a clear button.
pub fn search_entry(ui: &mut Ui, text: &mut String, hint: &str, id: egui::Id, width: f32) -> Response {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, BUTTON), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(id));
    ui.painter().rect_filled(rect, 6.0, p.fg(0.1));
    if focused {
        ui.painter().rect_stroke(rect.expand(1.0), 7.0, Stroke::new(2.0, p.accent_bg.gamma_multiply(0.6)), StrokeKind::Outside);
    }
    paint_icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(16.0, 16.0)), Icon::EditFind, p.dim());
    let clear_w = if text.is_empty() { 0.0 } else { 30.0 };
    let edit_rect = Rect::from_min_max(pos2(rect.left() + 32.0, rect.top()), pos2(rect.right() - 6.0 - clear_w, rect.bottom()));
    let response = ui.put(
        edit_rect,
        egui::TextEdit::singleline(text).id(id).hint_text(hint).frame(egui::Frame::NONE).align(Align2::LEFT_CENTER),
    );
    if !text.is_empty() {
        let r = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(22.0, 22.0));
        let clear = ui.interact(r, id.with("clear"), Sense::click()).on_hover_text("Clear");
        paint_icon(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Icon::EditClear, if clear.hovered() { p.window_fg } else { p.dim() });
        if clear.clicked() {
            text.clear();
            ui.memory_mut(|m| m.request_focus(id));
            let mut r = response.clone();
            r.mark_changed();
            return r;
        }
    }
    response
}

/// The Adwaita dialog frame (rounded, with a dimmed backdrop handled by `egui::Modal`).
pub fn dialog_frame(dark: bool) -> Frame {
    let p = Palette::for_dark(dark);
    Frame::new()
        .fill(p.dialog_bg)
        .corner_radius(15)
        .inner_margin(Margin::ZERO)
        .stroke(Stroke::new(1.0, p.fg(if dark { 0.08 } else { 0.0 })))
        .shadow(Shadow { offset: [0, 6], blur: 28, spread: 0, color: Color32::from_black_alpha(if dark { 130 } else { 60 }) })
}

/// A dialog's header: centred title and a close button. Returns true when closed.
pub fn dialog_header(ui: &mut Ui, title: &str) -> bool {
    let p = Palette::of(ui);
    let mut close = false;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, title, bold(14.5), p.window_fg);
    let r = Rect::from_center_size(pos2(rect.right() - 24.0, rect.center().y), vec2(24.0, 24.0));
    let resp = ui.interact(r, ui.make_persistent_id(("dialog-close", title)), Sense::click()).on_hover_text("Close");
    ui.painter().circle_filled(r.center(), 12.0, p.fg(if resp.hovered() { 0.15 } else { 0.1 }));
    paint_icon(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Icon::WindowClose, p.window_fg);
    if resp.clicked() {
        close = true;
    }
    close
}

/// Adwaita toast: a dark rounded pill at the bottom of the window.
pub fn toast(ctx: &egui::Context, text: &str) -> bool {
    let mut dismissed = false;
    egui::Area::new(egui::Id::unique("adw-toast"))
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
        .interactable(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let dark = ui.visuals().dark_mode;
            let bg = if dark { Color32::from_rgb(0x4a, 0x4a, 0x4e) } else { Color32::from_rgb(0x32, 0x32, 0x36) };
            Frame::new()
                .fill(bg)
                .corner_radius(20)
                .inner_margin(Margin { left: 20, right: 6, top: 6, bottom: 6 })
                .shadow(Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(80) })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(text).color(Color32::WHITE));
                        let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                        ui.painter().circle_filled(r.center(), 14.0, Color32::WHITE.gamma_multiply(if resp.hovered() { 0.2 } else { 0.1 }));
                        paint_icon(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Icon::WindowClose, Color32::WHITE);
                        dismissed = resp.clicked();
                    });
                });
        });
    dismissed
}

/// Adwaita `Banner`: a tinted strip under the header bar with a message and a button.
pub fn banner(ui: &mut Ui, text: &str, button: Option<&str>) -> bool {
    let p = Palette::of(ui);
    let fill = blend(p.headerbar_bg, p.accent_bg, if p.dark { 0.25 } else { 0.15 });
    let mut clicked = false;
    Frame::new().fill(fill).inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_centered(|ui| {
            ui.set_min_height(34.0);
            ui.label(text);
            if let Some(label) = button {
                clicked = text_button(ui, label, true, true).clicked();
            }
        });
    });
    clicked
}

/// The "card" page look (white page with Adwaita's soft card shadow).
pub fn page_shadow(painter: &egui::Painter, rect: Rect, dark: bool) {
    let shadow = Shadow { offset: [0, 1], blur: 6, spread: 1, color: Color32::from_black_alpha(if dark { 110 } else { 30 }) };
    painter.add(shadow.as_shape(rect, CornerRadius::ZERO));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_black_alpha(if dark { 60 } else { 10 })), StrokeKind::Outside);
}
