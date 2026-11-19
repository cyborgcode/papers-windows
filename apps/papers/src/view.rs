//! The document view: layout, scrolling, zoom, tiles, text selection, links and search.
//!
//! Nothing here renders pixels. Each frame the view works out which tiles the screen
//! needs, asks the render thread for the ones it lacks, uploads whatever has arrived
//! (a bounded amount per frame, so a burst of tiles never stalls a frame) and draws
//! the best tiles it has: sharp ones at the current zoom on top of lower-resolution
//! ones, so zooming and scrolling never show blank pages.

use crate::settings::{Fit, Recent};
use egui::{
    Align2, Color32, CursorIcon, Key, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};
use papers_render::{
    ColorImage, Dest, DocInfo, Engine, LinkTarget, NightStyle, NormRect, PageExtras, Request, Rotation, SearchOptions,
    TileKey, tiles_for,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

const GAP: f32 = 12.0;
const MARGIN: f32 = 16.0;
/// Width of the low-resolution page images used as placeholders and thumbnails, in points.
pub const THUMB_W: f32 = 150.0;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 64.0;
/// Points per PDF point at 100 %: a PDF point is 1/72 inch, an egui point 1/96 inch.
const PT: f32 = 96.0 / 72.0;
/// GPU memory for page textures before old ones are dropped.
const TEXTURE_BUDGET: usize = 768 * 1024 * 1024;
/// Upload at most this many bytes of new tiles per frame; the rest waits for the next frame.
const UPLOAD_PER_FRAME: usize = 48 * 1024 * 1024;
/// Wait this long after the last zoom step before rendering at the new size.
const ZOOM_SETTLE: Duration = Duration::from_millis(140);
const EXTRAS_KEEP: usize = 48;

type TexKey = (TileKey, Option<NightStyle>);

struct Tex {
    handle: TextureHandle,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Textures {
    map: HashMap<TexKey, Tex>,
    by_page: HashMap<u32, Vec<TexKey>>,
    incoming: VecDeque<(TexKey, Arc<ColorImage>)>,
    bytes: usize,
    frame: u64,
}

impl Textures {
    fn has(&self, key: &TexKey) -> bool {
        self.map.contains_key(key) || self.incoming.iter().any(|(k, _)| k == key)
    }

    fn page_has_any(&self, page: u32, night: Option<NightStyle>, rotation: Rotation) -> bool {
        self.by_page.get(&page).is_some_and(|keys| keys.iter().any(|(k, n)| *n == night && k.rotation == rotation))
            || self.incoming.iter().any(|((k, n), _)| k.page == page && *n == night && k.rotation == rotation)
    }

    /// Turn received images into GPU textures, within the per-frame budget.
    fn upload(&mut self, ctx: &egui::Context) -> bool {
        let mut budget = UPLOAD_PER_FRAME;
        while let Some((key, image)) = self.incoming.pop_front() {
            let bytes = image.pixels.len() * 4;
            let handle = ctx.load_texture(
                format!("p{}-{}x{}-{}-{}", key.0.page, key.0.width, key.0.height, key.0.col, key.0.row),
                egui::ImageData::Color(image),
                TextureOptions::LINEAR,
            );
            if let Some(old) = self.map.insert(key, Tex { handle, bytes, used: self.frame }) {
                self.bytes -= old.bytes;
            } else {
                self.by_page.entry(key.0.page).or_default().push(key);
            }
            self.bytes += bytes;
            budget = budget.saturating_sub(bytes);
            if budget == 0 {
                break;
            }
        }
        self.evict();
        !self.incoming.is_empty()
    }

    fn evict(&mut self) {
        if self.bytes <= TEXTURE_BUDGET {
            return;
        }
        let mut old: Vec<(u64, TexKey)> =
            self.map.iter().filter(|(_, t)| t.used + 1 < self.frame).map(|(k, t)| (t.used, *k)).collect();
        old.sort_unstable_by_key(|(used, _)| *used);
        for (_, key) in old {
            if self.bytes <= TEXTURE_BUDGET * 3 / 4 {
                break;
            }
            self.remove(&key);
        }
    }

    fn remove(&mut self, key: &TexKey) {
        if let Some(t) = self.map.remove(key) {
            self.bytes -= t.bytes;
            if let Some(list) = self.by_page.get_mut(&key.0.page) {
                list.retain(|k| k != key);
            }
        }
    }

}

/// Page rectangles in content coordinates (points), for one zoom level.
#[derive(Default)]
struct Layout {
    rects: Vec<Rect>,
    size: Vec2,
    /// Points per PDF point.
    k: f32,
    key: (u32, Rotation, bool, u32),
}

#[derive(Default)]
pub struct Search {
    pub open: bool,
    pub focus: bool,
    pub query: String,
    pub match_case: bool,
    pub whole_word: bool,
    generation: u64,
    sent: Option<(String, bool, bool)>,
    edited: Option<Instant>,
    /// Matches by page; each match is one or more rectangles (unrotated, normalised).
    pub results: BTreeMap<u32, Vec<Vec<NormRect>>>,
    pub current: Option<(u32, usize)>,
    pub searched: u32,
    pub total_pages: u32,
    pub done: bool,
    jump_on_first: bool,
}

impl Search {
    pub fn count(&self) -> usize {
        self.results.values().map(Vec::len).sum()
    }

    /// 1-based position of the current match among all matches.
    pub fn position(&self) -> Option<usize> {
        let (page, i) = self.current?;
        Some(self.results.range(..page).map(|(_, v)| v.len()).sum::<usize>() + i + 1)
    }

    pub fn is_running(&self) -> bool {
        self.sent.is_some() && !self.done
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Selection {
    anchor: (u32, usize),
    head: (u32, usize),
}

impl Selection {
    fn ordered(&self) -> ((u32, usize), (u32, usize)) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Pan,
    Select,
}

/// Things the view asks the application to do.
pub enum Action {
    OpenUrl(String),
}

pub struct DocView {
    pub info: Box<DocInfo>,
    pub rotation: Rotation,
    pub fit: Fit,
    pub zoom: f32,
    pub dual: bool,
    pub current_page: u32,
    pub search: Search,
    pub presenting: bool,
    layout: Layout,
    scroll: Vec2,
    pending_scroll: Option<Vec2>,
    pending_dest: Option<Dest>,
    pending_anchor: Option<(u32, f32, f32, Pos2)>,
    viewport: Rect,
    render_k: f32,
    zoom_moved: Option<Instant>,
    textures: Textures,
    extras: HashMap<u32, Arc<PageExtras>>,
    extras_requested: HashSet<u32>,
    selection: Option<Selection>,
    drag: Drag,
    last_sent: (HashSet<TileKey>, Option<NightStyle>, bool),
    last_night: Option<NightStyle>,
    ppp: f32,
    hover_link: Option<String>,
    sidebar_thumbs: Vec<u32>,
}

impl DocView {
    pub fn new(info: Box<DocInfo>, restore: Option<&Recent>) -> Self {
        let mut view = DocView {
            info,
            rotation: Rotation::R0,
            fit: Fit::Width,
            zoom: 1.0,
            dual: false,
            current_page: 0,
            search: Search::default(),
            presenting: false,
            layout: Layout::default(),
            scroll: Vec2::ZERO,
            pending_scroll: None,
            pending_dest: None,
            pending_anchor: None,
            viewport: Rect::NOTHING,
            render_k: 0.0,
            zoom_moved: None,
            textures: Textures::default(),
            extras: HashMap::new(),
            extras_requested: HashSet::new(),
            selection: None,
            drag: Drag::None,
            last_sent: (HashSet::new(), None, false),
            last_night: None,
            ppp: 1.0,
            hover_link: None,
            sidebar_thumbs: Vec::new(),
        };
        if let Some(r) = restore {
            view.rotation = r.rotation();
            view.fit = r.fit;
            view.zoom = r.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
            let page = r.page.min(view.page_count().saturating_sub(1));
            view.pending_dest = Some(Dest { page, y: Some(r.offset.clamp(0.0, 1.0)) });
            view.current_page = page;
        }
        view
    }

    pub fn page_count(&self) -> u32 {
        self.info.page_sizes.len() as u32
    }

    pub fn id(&self) -> papers_render::DocId {
        self.info.id
    }

    /// Where the reader is, for "continue where you left off".
    pub fn recent(&self) -> Recent {
        let page = self.current_page;
        let offset = self
            .layout
            .rects
            .get(page as usize)
            .map(|r| ((self.scroll.y + MARGIN - r.top()) / r.height()).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        Recent {
            path: self.info.path.clone(),
            page,
            offset,
            zoom: self.zoom,
            fit: self.fit,
            rotation: self.rotation.degrees(),
            title: self.info.title.clone(),
        }
    }

    /// Displayed page size in PDF points (after user rotation).
    fn page_pts(&self, page: usize) -> Vec2 {
        let [w, h] = self.info.page_sizes[page];
        if self.rotation.is_sideways() { vec2(h, w) } else { vec2(w, h) }
    }

    // ----- events from the render thread -----

    pub fn tile_arrived(&mut self, key: TileKey, night: Option<NightStyle>, image: Arc<ColorImage>) {
        let k = (key, night);
        self.textures.incoming.retain(|(old, _)| *old != k);
        self.textures.incoming.push_back((k, image));
    }

    pub fn extras_arrived(&mut self, page: u32, extras: PageExtras) {
        self.extras.insert(page, Arc::new(extras));
        if self.extras.len() > EXTRAS_KEEP {
            let current = self.current_page as i64;
            let keep: HashSet<u32> = {
                let mut pages: Vec<u32> = self.extras.keys().copied().collect();
                pages.sort_by_key(|p| (*p as i64 - current).abs());
                pages.into_iter().take(EXTRAS_KEEP / 2).collect()
            };
            let selected = self.selection.map(|s| s.ordered());
            self.extras.retain(|p, _| {
                keep.contains(p) || selected.is_some_and(|((a, _), (b, _))| (a..=b).contains(p))
            });
            self.extras_requested.retain(|p| self.extras.contains_key(p));
        }
    }

    pub fn search_hits(&mut self, generation: u64, page: u32, matches: Vec<Vec<NormRect>>) {
        if generation != self.search.generation {
            return;
        }
        self.search.results.insert(page, matches);
        if self.search.jump_on_first && self.search.current.is_none() {
            self.search.jump_on_first = false;
            self.search.current = Some((page, 0));
            self.scroll_to_match();
        }
    }

    pub fn search_progress(&mut self, generation: u64, searched: u32, total: u32) {
        if generation == self.search.generation {
            self.search.searched = searched;
            self.search.total_pages = total;
        }
    }

    pub fn search_done(&mut self, generation: u64) {
        if generation == self.search.generation {
            self.search.done = true;
            self.search.searched = self.search.total_pages;
        }
    }

    // ----- navigation -----

    pub fn go_to(&mut self, dest: Dest) {
        self.pending_dest = Some(dest);
    }

    pub fn go_to_page(&mut self, page: u32) {
        let page = page.min(self.page_count().saturating_sub(1));
        self.current_page = page;
        self.go_to(Dest { page, y: None });
    }

    pub fn next_page(&mut self) {
        let step = if self.dual && !self.presenting { 2 } else { 1 };
        self.go_to_page((self.current_page + step).min(self.page_count().saturating_sub(1)));
    }

    pub fn prev_page(&mut self) {
        let step = if self.dual && !self.presenting { 2 } else { 1 };
        self.go_to_page(self.current_page.saturating_sub(step));
    }

    pub fn scroll_by(&mut self, delta: Vec2) {
        let base = self.pending_scroll.unwrap_or(self.scroll);
        self.pending_scroll = Some(self.clamp_scroll(base + delta));
    }

    pub fn page_down(&mut self) {
        self.scroll_by(vec2(0.0, (self.viewport.height() - 40.0).max(40.0)));
    }

    pub fn page_up(&mut self) {
        self.scroll_by(vec2(0.0, -(self.viewport.height() - 40.0).max(40.0)));
    }

    pub fn home(&mut self) {
        self.pending_scroll = Some(vec2(self.scroll.x, 0.0));
    }

    pub fn end(&mut self) {
        self.pending_scroll = Some(self.clamp_scroll(vec2(self.scroll.x, f32::MAX)));
    }

    fn clamp_scroll(&self, v: Vec2) -> Vec2 {
        let max = (self.layout.size - self.viewport.size()).max(Vec2::ZERO);
        vec2(v.x.clamp(0.0, max.x), v.y.clamp(0.0, max.y))
    }

    // ----- zoom and rotation -----

    pub fn set_zoom(&mut self, zoom: f32, anchor: Option<Pos2>) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if (zoom - self.zoom).abs() < 1e-4 && self.fit == Fit::None {
            return;
        }
        self.remember_anchor(anchor);
        self.fit = Fit::None;
        self.zoom = zoom;
        self.zoom_moved = Some(Instant::now());
    }

    pub fn zoom_in(&mut self) {
        let z = ZOOM_STEPS.iter().copied().find(|z| *z > self.zoom * 1.01).unwrap_or(MAX_ZOOM);
        self.set_zoom(z, None);
        self.zoom_moved = None; // discrete steps render right away
    }

    pub fn zoom_out(&mut self) {
        let z = ZOOM_STEPS.iter().rev().copied().find(|z| *z < self.zoom * 0.99).unwrap_or(MIN_ZOOM);
        self.set_zoom(z, None);
        self.zoom_moved = None;
    }

    pub fn set_fit(&mut self, fit: Fit) {
        self.remember_anchor(None);
        self.fit = fit;
        self.zoom_moved = None;
    }

    pub fn rotate(&mut self, clockwise: bool) {
        self.rotation = if clockwise { self.rotation.clockwise() } else { self.rotation.counter_clockwise() };
        self.pending_dest = Some(Dest { page: self.current_page, y: None });
        self.selection = None;
    }

    pub fn set_dual(&mut self, dual: bool) {
        if self.dual != dual {
            self.dual = dual;
            self.pending_dest = Some(Dest { page: self.current_page, y: None });
        }
    }

    pub fn zoom_label(&self) -> String {
        format!("{:.0}%", self.zoom * 100.0)
    }

    /// Keep the point under `anchor` (screen position; default: the top centre of the view)
    /// in place across the next layout change.
    fn remember_anchor(&mut self, anchor: Option<Pos2>) {
        if self.layout.rects.is_empty() || !self.viewport.is_positive() {
            return;
        }
        let screen = anchor.unwrap_or(pos2(self.viewport.center().x, self.viewport.top() + 1.0));
        let content = screen - self.viewport.min + self.scroll;
        let page = self.page_at_y(content.y);
        let r = self.layout.rects[page as usize];
        let u = (content.x - r.left()) / r.width();
        let v = (content.y - r.top()) / r.height();
        self.pending_anchor = Some((page, u, v, screen));
    }

    fn page_at_y(&self, y: f32) -> u32 {
        let rects = &self.layout.rects;
        let i = rects.partition_point(|r| r.bottom() + GAP / 2.0 < y);
        i.min(rects.len().saturating_sub(1)) as u32
    }

    fn fit_k(&self, viewport: Vec2) -> f32 {
        let avail_w = (viewport.x - 2.0 * MARGIN - 14.0).max(50.0);
        let avail_h = (viewport.y - 2.0 * MARGIN).max(50.0);
        let n = self.page_count() as usize;
        let widest = if self.dual && !self.presenting {
            (0..n).step_by(2).map(|i| self.page_pts(i).x + if i + 1 < n { self.page_pts(i + 1).x } else { 0.0 }).fold(1.0, f32::max)
        } else {
            (0..n).map(|i| self.page_pts(i).x).fold(1.0, f32::max)
        };
        let gap = if self.dual { GAP } else { 0.0 };
        let kw = (avail_w - gap) / widest;
        match self.fit {
            Fit::Width => kw,
            Fit::Page => {
                let p = self.page_pts(self.current_page as usize);
                kw.min(avail_h / p.y)
            }
            Fit::None => self.zoom * PT,
        }
    }

    fn update_layout(&mut self, viewport: Vec2) {
        let k = self.fit_k(viewport).clamp(MIN_ZOOM * PT, MAX_ZOOM * PT);
        if self.fit != Fit::None {
            self.zoom = k / PT;
        }
        let key = (k.to_bits(), self.rotation, self.dual, viewport.x.to_bits());
        if key == self.layout.key && !self.layout.rects.is_empty() {
            return;
        }
        let n = self.page_count() as usize;
        let sizes: Vec<Vec2> = (0..n).map(|i| self.page_pts(i) * k).collect();
        let mut rects = Vec::with_capacity(n);
        let rows: Vec<Vec<usize>> =
            if self.dual { (0..n).step_by(2).map(|i| (i..(i + 2).min(n)).collect()).collect() } else { (0..n).map(|i| vec![i]).collect() };
        let row_w = |row: &Vec<usize>| row.iter().map(|&i| sizes[i].x).sum::<f32>() + GAP * (row.len() as f32 - 1.0);
        let content_w = rows.iter().map(row_w).fold(0.0, f32::max) + 2.0 * MARGIN;
        let width = content_w.max(viewport.x);
        let mut y = MARGIN;
        rects.resize(n, Rect::NOTHING);
        for row in &rows {
            let w = row_w(row);
            let h = row.iter().map(|&i| sizes[i].y).fold(0.0, f32::max);
            let mut x = ((width - w) / 2.0).max(MARGIN);
            for &i in row {
                rects[i] = Rect::from_min_size(pos2(x, y + (h - sizes[i].y) / 2.0), sizes[i]);
                x += sizes[i].x + GAP;
            }
            y += h + GAP;
        }
        self.layout = Layout { rects, size: vec2(width, y - GAP + MARGIN), k, key };
    }

    /// Resolve pending navigation now that the layout is known.
    fn apply_pending(&mut self) {
        if let Some((page, u, v, screen)) = self.pending_anchor.take()
            && let Some(r) = self.layout.rects.get(page as usize)
        {
            let content = pos2(r.left() + u * r.width(), r.top() + v * r.height());
            self.pending_scroll = Some(self.clamp_scroll(content.to_vec2() - (screen - self.viewport.min)));
        }
        if let Some(dest) = self.pending_dest.take()
            && let Some(r) = self.layout.rects.get(dest.page as usize)
        {
            let y = r.top() + dest.y.map_or(0.0, |v| v * r.height()) - MARGIN;
            let x = if self.layout.size.x > self.viewport.width() {
                r.center().x - self.viewport.width() / 2.0
            } else {
                0.0
            };
            self.pending_scroll = Some(self.clamp_scroll(vec2(x, y)));
            self.current_page = dest.page;
        }
    }

    // ----- search -----

    pub fn search_changed(&mut self) {
        self.search.edited = Some(Instant::now());
    }

    fn run_search(&mut self, engine: &Engine) {
        let s = &mut self.search;
        let wanted = (s.query.trim().to_string(), s.match_case, s.whole_word);
        if s.sent.as_ref() == Some(&wanted) {
            return;
        }
        s.generation += 1;
        s.results.clear();
        s.current = None;
        s.done = false;
        s.searched = 0;
        s.jump_on_first = true;
        if wanted.0.is_empty() {
            s.sent = None;
            engine.send(Request::CancelSearch);
            return;
        }
        engine.send(Request::Search {
            doc: self.info.id,
            generation: s.generation,
            query: wanted.0.clone(),
            options: SearchOptions { match_case: wanted.1, whole_word: wanted.2 },
            start_page: self.current_page,
        });
        s.sent = Some(wanted);
    }

    pub fn search_step(&mut self, forward: bool, engine: &Engine) {
        self.search.edited = None;
        self.run_search(engine);
        let results = &self.search.results;
        if results.is_empty() {
            return;
        }
        let next = match self.search.current {
            None => {
                let from = self.current_page;
                if forward {
                    results.range(from..).next().or_else(|| results.iter().next()).map(|(p, _)| (*p, 0))
                } else {
                    results.range(..=from).next_back().or_else(|| results.iter().next_back()).map(|(p, v)| (*p, v.len() - 1))
                }
            }
            Some((page, i)) => {
                let len = results.get(&page).map_or(0, Vec::len);
                if forward {
                    if i + 1 < len {
                        Some((page, i + 1))
                    } else {
                        results.range(page + 1..).next().or_else(|| results.iter().next()).map(|(p, _)| (*p, 0))
                    }
                } else if i > 0 && i <= len {
                    Some((page, i - 1))
                } else {
                    results.range(..page).next_back().or_else(|| results.iter().next_back()).map(|(p, v)| (*p, v.len() - 1))
                }
            }
        };
        self.search.current = next;
        self.scroll_to_match();
    }

    pub fn close_search(&mut self, engine: &Engine) {
        self.search.open = false;
        self.search.query.clear();
        self.search.sent = None;
        self.search.results.clear();
        self.search.current = None;
        engine.send(Request::CancelSearch);
    }

    fn scroll_to_match(&mut self) {
        let Some((page, i)) = self.search.current else { return };
        let Some(rects) = self.search.results.get(&page).and_then(|m| m.get(i)) else { return };
        let r = self.rotation.map_rect(rects[0]);
        if self.layout.rects.is_empty() {
            self.pending_dest = Some(Dest { page, y: Some(r[1]) });
            return;
        }
        let pr = self.layout.rects[page as usize];
        let target = Rect::from_min_max(
            pos2(pr.left() + r[0] * pr.width(), pr.top() + r[1] * pr.height()),
            pos2(pr.left() + r[2] * pr.width(), pr.top() + r[3] * pr.height()),
        );
        let view = Rect::from_min_size(self.scroll.to_pos2(), self.viewport.size());
        if !view.shrink(24.0).contains_rect(target) {
            let mut s = self.scroll;
            s.y = target.top() - self.viewport.height() / 3.0;
            if target.left() < view.left() || target.right() > view.right() {
                s.x = target.center().x - self.viewport.width() / 2.0;
            }
            self.pending_scroll = Some(self.clamp_scroll(s));
        }
        self.current_page = page;
    }

    // ----- text selection -----

    pub fn has_selection(&self) -> bool {
        self.selection.is_some_and(|s| s.anchor != s.head)
    }

    pub fn selected_text(&self) -> Option<String> {
        let ((p0, i0), (p1, i1)) = self.selection?.ordered();
        let mut out = String::new();
        for page in p0..=p1 {
            let Some(extras) = self.extras.get(&page) else { continue };
            let from = if page == p0 { i0 } else { 0 };
            let to = if page == p1 { i1 + 1 } else { extras.chars.len() };
            for c in extras.chars.get(from..to.min(extras.chars.len())).unwrap_or(&[]) {
                out.push_str(&c.text);
            }
            if page != p1 && !out.ends_with('\n') {
                out.push('\n');
            }
        }
        let out = out.replace("\r\n", "\n");
        (!out.trim().is_empty()).then_some(out)
    }

    pub fn select_all_on_page(&mut self) {
        let page = self.current_page;
        if let Some(e) = self.extras.get(&page)
            && !e.chars.is_empty()
        {
            self.selection = Some(Selection { anchor: (page, 0), head: (page, e.chars.len() - 1) });
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// The character at (or, with `nearest`, closest to) a point on a page.
    fn char_at(&self, page: u32, point: [f32; 2], nearest: bool) -> Option<usize> {
        let extras = self.extras.get(&page)?;
        let [u, v] = self.rotation.unmap_point(point);
        let mut best = None;
        let mut best_d = f32::MAX;
        for (i, c) in extras.chars.iter().enumerate() {
            if !c.is_visible() {
                continue;
            }
            let r = c.rect;
            let dx = (r[0] - u).max(0.0).max(u - r[2]);
            let dy = (r[1] - v).max(0.0).max(v - r[3]);
            // Distance across lines counts more than along a line.
            let d = dx + dy * 4.0;
            if d < best_d {
                best_d = d;
                best = Some(i);
                if d == 0.0 {
                    break;
                }
            }
        }
        if nearest || best_d == 0.0 { best } else { None }
    }

    fn select_word(&mut self, page: u32, index: usize) {
        let Some(extras) = self.extras.get(&page) else { return };
        let is_word = |i: usize| extras.chars.get(i).is_some_and(|c| c.text.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '\''));
        if !is_word(index) {
            self.selection = Some(Selection { anchor: (page, index), head: (page, index) });
            return;
        }
        let mut a = index;
        while a > 0 && is_word(a - 1) {
            a -= 1;
        }
        let mut b = index;
        while is_word(b + 1) {
            b += 1;
        }
        self.selection = Some(Selection { anchor: (page, a), head: (page, b) });
    }

    // ----- per-frame work -----

    /// Show the document. Returns actions for the application.
    pub fn ui(&mut self, ui: &mut Ui, engine: &Engine, night: Option<NightStyle>) -> Vec<Action> {
        let ctx = ui.ctx().clone();
        self.ppp = ctx.pixels_per_point();
        self.textures.frame += 1;
        if self.textures.upload(&ctx) {
            ctx.request_repaint();
        }
        if night != self.last_night {
            self.last_night = night;
            // Keep the other mode's textures for an instant switch back; the budget evicts them later.
        }
        if let Some(t) = self.search.edited
            && t.elapsed() > Duration::from_millis(250)
        {
            self.search.edited = None;
            self.run_search(engine);
        } else if self.search.edited.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        if self.presenting {
            return self.present(ui, engine, night);
        }

        let mut actions = Vec::new();
        let avail = ui.available_rect_before_wrap();
        self.viewport = avail;
        self.update_layout(avail.size());
        self.apply_pending();

        // Zoom with Ctrl+wheel or pinch, anchored at the pointer.
        let (zoom_delta, pointer) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
        if (zoom_delta - 1.0).abs() > 1e-4 && pointer.is_some_and(|p| avail.contains(p)) {
            self.set_zoom(self.zoom * zoom_delta, pointer);
            self.update_layout(avail.size());
            self.apply_pending();
        }
        // Render at the new size only once zooming has settled; until then the existing
        // tiles are stretched, which is instant.
        let k = self.layout.k;
        match self.zoom_moved {
            Some(t) if t.elapsed() < ZOOM_SETTLE && self.render_k > 0.0 => {
                ctx.request_repaint_after(ZOOM_SETTLE.saturating_sub(t.elapsed()));
            }
            _ => {
                self.zoom_moved = None;
                self.render_k = k;
            }
        }

        let mut area = egui::ScrollArea::both()
            .id_salt(("document", self.info.id))
            .auto_shrink([false, false])
            .scroll_source(egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Never, ..Default::default() });
        if let Some(offset) = self.pending_scroll.take() {
            area = area.scroll_offset(offset);
        }
        let paper = night.map_or(Color32::WHITE, |s| s.paper());
        let output = area.show_viewport(ui, |ui, viewport| {
            let origin = ui.min_rect().min;
            let (_, response) = ui.allocate_exact_size(self.layout.size, Sense::click_and_drag());
            self.interact(ui, &response, origin, viewport, engine, &mut actions);
            let painter = ui.painter().clone();
            let first = self.page_at_y(viewport.top()) as usize;
            let mut visible = Vec::new();
            for i in first.saturating_sub(1)..self.layout.rects.len() {
                let r = self.layout.rects[i];
                if r.top() > viewport.bottom() {
                    break;
                }
                if r.intersects(viewport) {
                    visible.push(i as u32);
                }
                let screen = snap(r.translate(origin.to_vec2()), self.ppp);
                if !screen.intersects(painter.clip_rect().expand(8.0)) {
                    continue;
                }
                self.draw_page(&painter, i as u32, screen, paper, night);
            }
            visible
        });
        self.scroll = output.state.clamped_offset();
        let visible = output.inner;
        let viewport = Rect::from_min_size(self.scroll.to_pos2(), self.viewport.size());
        if let Some(&first) = visible.first() {
            let probe = viewport.top() + viewport.height() * 0.3;
            let page = visible.iter().copied().find(|&p| self.layout.rects[p as usize].bottom() + GAP >= probe).unwrap_or(first);
            if self.pending_dest.is_none() {
                self.current_page = page;
            }
        }
        self.request(engine, &visible, viewport, night);
        if let Some(url) = &self.hover_link {
            egui::Tooltip::always_open(ctx.clone(), ui.layer_id(), egui::Id::unique("link-tip"), egui::PopupAnchor::Pointer)
                .show(|ui| ui.label(url));
        }
        actions
    }


    fn page_at_screen(&self, pos: Pos2, origin: Pos2) -> Option<(u32, [f32; 2])> {
        let content = pos - origin.to_vec2();
        let page = self.page_at_y(content.y);
        let r = *self.layout.rects.get(page as usize)?;
        let u = (content.x - r.left()) / r.width();
        let v = (content.y - r.top()) / r.height();
        Some((page, [u, v]))
    }

    fn interact(&mut self, ui: &Ui, response: &egui::Response, origin: Pos2, _viewport: Rect, engine: &Engine, actions: &mut Vec<Action>) {
        self.hover_link = None;
        let pointer = response.hover_pos().or_else(|| response.interact_pointer_pos());
        let on_page = pointer.and_then(|p| self.page_at_screen(p, origin));
        let inside = on_page.filter(|(_, [u, v])| (0.0..=1.0).contains(u) && (0.0..=1.0).contains(v));

        // Hover: links and text get their own cursors.
        let mut link_here = None;
        if let Some((page, point)) = inside {
            if let Some(extras) = self.extras.get(&page) {
                let raw = self.rotation.unmap_point(point);
                link_here = extras
                    .links
                    .iter()
                    .find(|l| raw[0] >= l.rect[0] && raw[0] <= l.rect[2] && raw[1] >= l.rect[1] && raw[1] <= l.rect[3])
                    .cloned();
            } else if !self.extras_requested.contains(&page) {
                self.extras_requested.insert(page);
                engine.send(Request::Extras { doc: self.info.id, page });
            }
        }
        if self.drag == Drag::None && response.hovered() {
            if let Some(link) = &link_here {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                self.hover_link = Some(match &link.target {
                    LinkTarget::Uri(u) => u.clone(),
                    LinkTarget::Dest(d) => format!("Go to page {}", d.page + 1),
                });
            } else if inside.is_some_and(|(p, pt)| self.char_at(p, pt, false).is_some()) {
                ui.ctx().set_cursor_icon(CursorIcon::Text);
            }
        }

        let (primary, middle) = ui.input(|i| (i.pointer.primary_down(), i.pointer.middle_down()));
        if response.drag_started() {
            let on_text = inside.and_then(|(p, pt)| self.char_at(p, pt, false).map(|i| (p, i)));
            self.drag = if middle || !primary {
                Drag::Pan
            } else if let Some(at) = on_text {
                self.selection = Some(Selection { anchor: at, head: at });
                Drag::Select
            } else {
                self.selection = None;
                Drag::Pan
            };
        }
        if response.dragged() {
            match self.drag {
                Drag::Pan => {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    let delta = response.drag_delta();
                    self.scroll_by(-delta);
                }
                Drag::Select => {
                    ui.ctx().set_cursor_icon(CursorIcon::Text);
                    if let Some((page, pt)) = on_page
                        && let Some(i) = self.char_at(page, [pt[0].clamp(0.0, 1.0), pt[1].clamp(0.0, 1.0)], true)
                        && let Some(sel) = self.selection.as_mut()
                    {
                        sel.head = (page, i);
                    }
                    // Scroll when selecting past the edge of the view.
                    if let Some(p) = pointer {
                        let view = response.rect.intersect(ui.clip_rect());
                        let dy = if p.y < view.top() + 10.0 {
                            -12.0
                        } else if p.y > view.bottom() - 10.0 {
                            12.0
                        } else {
                            0.0
                        };
                        if dy != 0.0 {
                            self.scroll_by(vec2(0.0, dy));
                            ui.ctx().request_repaint();
                        }
                    }
                }
                Drag::None => {}
            }
        }
        if response.drag_stopped() {
            self.drag = Drag::None;
            if let Some(text) = self.selected_text() {
                // Like X11's primary selection, but explicit: keep it for Ctrl+C.
                log::debug!("selected {} characters", text.chars().count());
            }
        }
        if response.double_clicked()
            && let Some((page, pt)) = inside
            && let Some(i) = self.char_at(page, pt, false)
        {
            self.select_word(page, i);
        } else if response.clicked() {
            if let Some(link) = link_here {
                match link.target {
                    LinkTarget::Dest(d) => self.go_to(d),
                    LinkTarget::Uri(u) => actions.push(Action::OpenUrl(u)),
                }
            }
            self.selection = None;
        }
        let _ = Key::Escape;
    }

    fn draw_page(&mut self, painter: &egui::Painter, page: u32, screen: Rect, paper: Color32, night: Option<NightStyle>) {
        let shadow = Color32::from_black_alpha(if night.is_some() { 120 } else { 50 });
        painter.rect_filled(screen.translate(vec2(0.0, 1.5)).expand(1.0), 2.0, shadow);
        painter.rect_filled(screen, 0.0, paper);
        let frame = self.textures.frame;
        let render_w = self.pixel_size(page, self.render_k)[0];
        if let Some(keys) = self.textures.by_page.get(&page) {
            let mut keys: Vec<TexKey> =
                keys.iter().filter(|(k, n)| *n == night && k.rotation == self.rotation).copied().collect();
            // Low resolution first, the current resolution last (on top).
            keys.sort_by_key(|(k, _)| (k.width == render_w, k.width));
            let clip = painter.clip_rect();
            for key in keys {
                let n = key.0.norm_rect();
                let r = Rect::from_min_max(
                    pos2(screen.left() + n[0] * screen.width(), screen.top() + n[1] * screen.height()),
                    pos2(screen.left() + n[2] * screen.width(), screen.top() + n[3] * screen.height()),
                );
                if !r.intersects(clip) {
                    continue;
                }
                if let Some(tex) = self.textures.map.get_mut(&key) {
                    tex.used = frame;
                    painter.image(tex.handle.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
        }
        self.draw_overlays(painter, page, screen, night);
    }

    fn draw_overlays(&self, painter: &egui::Painter, page: u32, screen: Rect, night: Option<NightStyle>) {
        let to_screen = |r: NormRect| -> Rect {
            let r = self.rotation.map_rect(r);
            Rect::from_min_max(
                pos2(screen.left() + r[0] * screen.width(), screen.top() + r[1] * screen.height()),
                pos2(screen.left() + r[2] * screen.width(), screen.top() + r[3] * screen.height()),
            )
        };
        // Search matches.
        if let Some(matches) = self.search.results.get(&page) {
            let (all, current) = if night.is_some() {
                (Color32::from_rgba_unmultiplied(255, 200, 0, 70), Color32::from_rgba_unmultiplied(255, 140, 0, 130))
            } else {
                (Color32::from_rgba_unmultiplied(255, 220, 0, 90), Color32::from_rgba_unmultiplied(255, 120, 0, 140))
            };
            for (i, m) in matches.iter().enumerate() {
                let is_current = self.search.current == Some((page, i));
                for r in m {
                    let r = to_screen(*r).expand(1.0);
                    painter.rect_filled(r, 2.0, if is_current { current } else { all });
                    if is_current {
                        painter.rect_stroke(r, 2.0, Stroke::new(1.5, Color32::from_rgb(230, 100, 0)), egui::StrokeKind::Outside);
                    }
                }
            }
        }
        // Selection.
        if let Some(sel) = self.selection
            && sel.anchor != sel.head
        {
            let ((p0, i0), (p1, i1)) = sel.ordered();
            if (p0..=p1).contains(&page)
                && let Some(extras) = self.extras.get(&page)
            {
                let from = if page == p0 { i0 } else { 0 };
                let to = if page == p1 { i1 + 1 } else { extras.chars.len() };
                let color = if night.is_some() {
                    Color32::from_rgba_unmultiplied(110, 160, 255, 90)
                } else {
                    Color32::from_rgba_unmultiplied(40, 110, 230, 80)
                };
                let mut line: Option<NormRect> = None;
                let flush = |r: Option<NormRect>| {
                    if let Some(r) = r {
                        painter.rect_filled(to_screen(r), 0.0, color);
                    }
                };
                for c in extras.chars.get(from..to.min(extras.chars.len())).unwrap_or(&[]) {
                    if !c.is_visible() {
                        continue;
                    }
                    let r = c.rect;
                    line = match line {
                        Some(l) if same_line(&l, &r) => Some([l[0].min(r[0]), l[1].min(r[1]), l[2].max(r[2]), l[3].max(r[3])]),
                        other => {
                            flush(other);
                            Some(r)
                        }
                    };
                }
                flush(line);
            }
        }
    }

    /// Full page size in pixels at `k` points per PDF point.
    fn pixel_size(&self, page: u32, k: f32) -> [u32; 2] {
        let p = self.page_pts(page as usize) * k * self.ppp;
        [(p.x.round() as u32).clamp(1, 60_000), (p.y.round() as u32).clamp(1, 60_000)]
    }

    fn thumb_size(&self, page: u32) -> [u32; 2] {
        let p = self.page_pts(page as usize);
        let w = (THUMB_W * self.ppp).round();
        [w as u32, ((w * p.y / p.x).round() as u32).max(1)]
    }

    /// Work out which tiles are needed and ask the render thread for the missing ones.
    fn request(&mut self, engine: &Engine, visible: &[u32], viewport: Rect, night: Option<NightStyle>) {
        let mut want: Vec<TileKey> = Vec::new();
        let rot = self.rotation;
        let center = viewport.center().y;
        let mut pages: Vec<u32> = visible.to_vec();
        pages.sort_by(|a, b| {
            let da = (self.layout.rects[*a as usize].center().y - center).abs();
            let db = (self.layout.rects[*b as usize].center().y - center).abs();
            da.total_cmp(&db)
        });
        let have = |t: &Textures, k: &TileKey| t.has(&(*k, night));
        // 1. A quick low-resolution image for visible pages that show nothing yet.
        for &p in &pages {
            if !self.textures.page_has_any(p, night, rot) {
                let [w, h] = self.thumb_size(p);
                want.extend(tiles_for(p, w, h, rot, [0.0, 0.0, 1.0, 1.0]).into_iter().filter(|k| !have(&self.textures, k)));
            }
        }
        // 2. Sharp tiles for what is on screen.
        for &p in &pages {
            let r = self.layout.rects[p as usize];
            let vis = r.intersect(viewport);
            let norm = [
                (vis.left() - r.left()) / r.width(),
                (vis.top() - r.top()) / r.height(),
                (vis.right() - r.left()) / r.width(),
                (vis.bottom() - r.top()) / r.height(),
            ];
            let [w, h] = self.pixel_size(p, self.render_k);
            want.extend(tiles_for(p, w, h, rot, norm).into_iter().filter(|k| !have(&self.textures, k)));
        }
        // 3. Placeholders and then sharp tiles for the pages just around the view.
        let n = self.page_count();
        if let (Some(&first), Some(&last)) = (visible.iter().min(), visible.iter().max()) {
            let around: Vec<u32> =
                (last + 1..(last + 4).min(n)).chain(first.saturating_sub(3)..first).collect();
            for &p in &around {
                let [w, h] = self.thumb_size(p);
                want.extend(tiles_for(p, w, h, rot, [0.0, 0.0, 1.0, 1.0]).into_iter().filter(|k| !have(&self.textures, k)));
            }
            for p in [last + 1, first.wrapping_sub(1)] {
                if p < n {
                    let [w, h] = self.pixel_size(p, self.render_k);
                    // Only the part that will scroll into view first.
                    let part = if p > last { [0.0, 0.0, 1.0, 0.4] } else { [0.0, 0.6, 1.0, 1.0] };
                    want.extend(tiles_for(p, w, h, rot, part).into_iter().filter(|k| !have(&self.textures, k)));
                }
            }
            // Text and links for visible pages, for hovering and selecting.
            for &p in visible {
                if self.extras_requested.insert(p) {
                    engine.send(Request::Extras { doc: self.info.id, page: p });
                }
            }
        }
        // 4. Sidebar thumbnails.
        for p in std::mem::take(&mut self.sidebar_thumbs) {
            let [w, h] = self.thumb_size(p);
            want.extend(tiles_for(p, w, h, rot, [0.0, 0.0, 1.0, 1.0]).into_iter().filter(|k| !have(&self.textures, k)));
        }
        let mut seen = HashSet::new();
        want.retain(|k| seen.insert(*k));
        self.send_want(engine, want, night);
    }

    fn send_want(&mut self, engine: &Engine, want: Vec<TileKey>, night: Option<NightStyle>) {
        // The render thread works through the previous list in order; only send a new list
        // when it asks for something the previous one did not.
        let (sent, sent_night, active) = &self.last_sent;
        let active = *active;
        if active && *sent_night == night && want.iter().all(|k| sent.contains(k)) {
            return;
        }
        let empty = want.is_empty();
        self.last_sent = (want.iter().copied().collect(), night, !empty);
        if empty && !active {
            return;
        }
        engine.send(Request::Want { doc: self.info.id, tiles: want, night });
    }

    /// Ask for a page thumbnail (for the sidebar), returning the texture if it is ready.
    pub fn thumbnail(&mut self, page: u32, night: Option<NightStyle>) -> Option<(egui::TextureId, Vec2)> {
        let [w, h] = self.thumb_size(page);
        let tiles = tiles_for(page, w, h, self.rotation, [0.0, 0.0, 1.0, 1.0]);
        if tiles.len() == 1
            && let Some(tex) = self.textures.map.get_mut(&(tiles[0], night))
        {
            tex.used = self.textures.frame;
            return Some((tex.handle.id(), vec2(w as f32, h as f32) / self.ppp));
        }
        self.sidebar_thumbs.push(page);
        None
    }

    pub fn thumb_aspect(&self, page: u32) -> f32 {
        let p = self.page_pts(page as usize);
        p.y / p.x
    }


    // ----- presentation mode -----

    fn present(&mut self, ui: &mut Ui, engine: &Engine, night: Option<NightStyle>) -> Vec<Action> {
        let mut actions = Vec::new();
        let rect = ui.max_rect();
        let painter = ui.painter().clone();
        painter.rect_filled(rect, 0.0, Color32::BLACK);
        let page = self.current_page.min(self.page_count().saturating_sub(1));
        let p = self.page_pts(page as usize);
        let k = (rect.width() / p.x).min(rect.height() / p.y);
        let size = p * k;
        let screen = snap(Rect::from_center_size(rect.center(), size), self.ppp);
        self.render_k = k;
        let paper = night.map_or(Color32::WHITE, |s| s.paper());
        // Draw with a temporary single-page layout so overlays line up.
        self.draw_page(&painter, page, screen, paper, night);
        let response = ui.interact(rect, ui.scope_id().with("present"), Sense::click());
        if response.clicked() {
            self.next_page();
        } else if response.secondary_clicked() {
            self.prev_page();
        }
        let mut want = Vec::new();
        for (i, pg) in [page, page + 1, page.wrapping_sub(1)].into_iter().enumerate() {
            if pg >= self.page_count() {
                continue;
            }
            let pp = self.page_pts(pg as usize);
            let kk = (rect.width() / pp.x).min(rect.height() / pp.y);
            let [w, h] = self.pixel_size(pg, kk);
            if i == 0 && !self.textures.page_has_any(pg, night, self.rotation) {
                let [tw, th] = self.thumb_size(pg);
                want.extend(tiles_for(pg, tw, th, self.rotation, [0.0, 0.0, 1.0, 1.0]));
            }
            want.extend(tiles_for(pg, w, h, self.rotation, [0.0, 0.0, 1.0, 1.0]));
        }
        want.retain(|k| !self.textures.has(&(*k, night)));
        self.send_want(engine, want, night);
        if let Some(e) = self.extras.get(&page)
            && let Some(pos) = response.hover_pos()
        {
            let u = (pos.x - screen.left()) / screen.width();
            let v = (pos.y - screen.top()) / screen.height();
            let raw = self.rotation.unmap_point([u, v]);
            if let Some(l) = e.links.iter().find(|l| raw[0] >= l.rect[0] && raw[0] <= l.rect[2] && raw[1] >= l.rect[1] && raw[1] <= l.rect[3]) {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                if response.clicked() {
                    match &l.target {
                        LinkTarget::Dest(d) => self.current_page = d.page,
                        LinkTarget::Uri(u) => actions.push(Action::OpenUrl(u.clone())),
                    }
                }
            }
        } else if self.extras_requested.insert(page) {
            engine.send(Request::Extras { doc: self.info.id, page });
        }
        painter.text(
            rect.right_bottom() - vec2(16.0, 12.0),
            Align2::RIGHT_BOTTOM,
            format!("{} / {}", page + 1, self.page_count()),
            egui::FontId::proportional(13.0),
            Color32::from_gray(90),
        );
        actions
    }
}

const ZOOM_STEPS: [f32; 17] =
    [0.1, 0.25, 0.333, 0.5, 0.667, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0, 8.0, 16.0];

/// Align a rectangle's corner to the physical pixel grid so tiles land on whole pixels.
fn snap(r: Rect, ppp: f32) -> Rect {
    let s = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_max(pos2(s(r.left()), s(r.top())), pos2(s(r.right()), s(r.bottom())))
}

fn same_line(a: &NormRect, b: &NormRect) -> bool {
    let overlap = a[3].min(b[3]) - a[1].max(b[1]);
    let h = (a[3] - a[1]).min(b[3] - b[1]).max(1e-6);
    overlap / h > 0.5 && b[0] >= a[0] - (a[3] - a[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_merging() {
        assert!(same_line(&[0.1, 0.1, 0.12, 0.12], &[0.12, 0.1, 0.14, 0.12]));
        assert!(!same_line(&[0.1, 0.1, 0.12, 0.12], &[0.1, 0.13, 0.12, 0.15]));
    }

    #[test]
    fn snapping() {
        let r = snap(Rect::from_min_max(pos2(10.3, 4.6), pos2(20.2, 9.9)), 1.5);
        assert_eq!(r.left() * 1.5, (10.3f32 * 1.5).round());
    }
}
