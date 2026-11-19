//! The Papers rendering engine.
//!
//! All PDFium calls happen on one dedicated worker thread (PDFium is not
//! thread-safe). The UI sends it the list of tiles it wants, most important
//! first, and gets finished, already night-mode-filtered images back. Every
//! new request list replaces the previous one, so scrolling quickly never
//! builds up a backlog of pages that are no longer on screen.
//!
//! Pages are cut into tiles of at most [`TILE_W`]×[`TILE_H`] pixels so that
//! zooming in deeply only renders what is visible. Raw (day-mode) tiles are
//! kept in a memory-bounded cache, which makes switching night mode on and off
//! a matter of re-running the colour filter, not re-rendering.

mod night;
mod print;
mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

pub use egui::ColorImage;
pub use night::{Filter, NightStyle};

/// Maximum tile size in pixels. Wide and short tiles suit continuous vertical scrolling.
pub const TILE_W: u32 = 2048;
pub const TILE_H: u32 = 1024;

/// Identifies an open document within one [`Engine`].
pub type DocId = u64;

/// A rectangle in normalised page coordinates: `[x0, y0, x1, y1]`, each in `0..=1`,
/// origin at the top-left of the page as it is shown without user rotation.
pub type NormRect = [f32; 4];

/// Clockwise rotation applied by the user on top of the page's own rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Rotation {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    pub fn degrees(self) -> u32 {
        match self {
            Rotation::R0 => 0,
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }

    pub fn from_degrees(deg: i64) -> Self {
        match deg.rem_euclid(360) {
            90 => Rotation::R90,
            180 => Rotation::R180,
            270 => Rotation::R270,
            _ => Rotation::R0,
        }
    }

    pub fn clockwise(self) -> Self {
        Self::from_degrees(self.degrees() as i64 + 90)
    }

    pub fn counter_clockwise(self) -> Self {
        Self::from_degrees(self.degrees() as i64 - 90)
    }

    /// Whether width and height swap.
    pub fn is_sideways(self) -> bool {
        matches!(self, Rotation::R90 | Rotation::R270)
    }

    /// Map a point from unrotated normalised page coordinates to rotated ones.
    pub fn map_point(self, [u, v]: [f32; 2]) -> [f32; 2] {
        match self {
            Rotation::R0 => [u, v],
            Rotation::R90 => [1.0 - v, u],
            Rotation::R180 => [1.0 - u, 1.0 - v],
            Rotation::R270 => [v, 1.0 - u],
        }
    }

    /// Inverse of [`Rotation::map_point`].
    pub fn unmap_point(self, [u, v]: [f32; 2]) -> [f32; 2] {
        match self {
            Rotation::R0 => [u, v],
            Rotation::R90 => [v, 1.0 - u],
            Rotation::R180 => [1.0 - u, 1.0 - v],
            Rotation::R270 => [1.0 - v, u],
        }
    }

    pub fn map_rect(self, r: NormRect) -> NormRect {
        let a = self.map_point([r[0], r[1]]);
        let b = self.map_point([r[2], r[3]]);
        [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
    }
}

/// One tile of one page rendered at one size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub page: u32,
    /// Full page size in pixels at this zoom level (after rotation).
    pub width: u32,
    pub height: u32,
    pub rotation: Rotation,
    pub col: u16,
    pub row: u16,
}

impl TileKey {
    /// The pixel rectangle `[x0, y0, x1, y1)` this tile covers within the full page bitmap.
    pub fn pixel_rect(&self) -> [u32; 4] {
        let x0 = self.col as u32 * TILE_W;
        let y0 = self.row as u32 * TILE_H;
        [x0, y0, (x0 + TILE_W).min(self.width), (y0 + TILE_H).min(self.height)]
    }

    pub fn size(&self) -> [u32; 2] {
        let [x0, y0, x1, y1] = self.pixel_rect();
        [x1 - x0, y1 - y0]
    }

    /// The tile's area as a fraction of the (rotated) page.
    pub fn norm_rect(&self) -> NormRect {
        let [x0, y0, x1, y1] = self.pixel_rect();
        let (w, h) = (self.width as f32, self.height as f32);
        [x0 as f32 / w, y0 as f32 / h, x1 as f32 / w, y1 as f32 / h]
    }

    pub fn bytes(&self) -> usize {
        let [w, h] = self.size();
        w as usize * h as usize * 4
    }
}

/// The tiles of a page at a given pixel size that intersect `visible` (normalised, rotated
/// coordinates), in reading order. Pass `[0, 0, 1, 1]` for the whole page.
pub fn tiles_for(page: u32, width: u32, height: u32, rotation: Rotation, visible: NormRect) -> Vec<TileKey> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let cols = width.div_ceil(TILE_W);
    let rows = height.div_ceil(TILE_H);
    let clampi = |v: f32, n: u32, px: u32, tile: u32| -> u32 {
        (((v.clamp(0.0, 1.0) * px as f32) as u32) / tile).min(n - 1)
    };
    let c0 = clampi(visible[0], cols, width, TILE_W);
    let c1 = clampi(visible[2], cols, width, TILE_W);
    let r0 = clampi(visible[1], rows, height, TILE_H);
    let r1 = clampi(visible[3], rows, height, TILE_H);
    let mut out = Vec::with_capacity(((c1 - c0 + 1) * (r1 - r0 + 1)) as usize);
    for row in r0..=r1 {
        for col in c0..=c1 {
            out.push(TileKey { page, width, height, rotation, col: col as u16, row: row as u16 });
        }
    }
    out
}

/// A table-of-contents entry.
#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub title: String,
    pub dest: Option<Dest>,
    pub children: Vec<OutlineItem>,
}

/// A place in the document: a page and optionally a vertical position on it (0 = top).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dest {
    pub page: u32,
    pub y: Option<f32>,
}

#[derive(Clone, Debug)]
pub enum LinkTarget {
    Dest(Dest),
    Uri(String),
}

#[derive(Clone, Debug)]
pub struct Link {
    pub rect: NormRect,
    pub target: LinkTarget,
}

/// One character of a page's text layer.
#[derive(Clone, Debug)]
pub struct TextChar {
    pub text: String,
    /// Empty (all zeros) for characters PDFium generated, such as line breaks.
    pub rect: NormRect,
}

impl TextChar {
    pub fn is_visible(&self) -> bool {
        self.rect[2] > self.rect[0] && self.rect[3] > self.rect[1]
    }
}

/// What the UI needs to know about a page beyond its pixels.
#[derive(Clone, Debug, Default)]
pub struct PageExtras {
    pub chars: Vec<TextChar>,
    pub links: Vec<Link>,
}

#[derive(Clone, Debug)]
pub struct DocInfo {
    pub id: DocId,
    pub path: PathBuf,
    /// Page sizes in PDF points, as displayed (the page's own /Rotate applied).
    pub page_sizes: Vec<[f32; 2]>,
    pub outline: Vec<OutlineItem>,
    /// Document properties as (label, value) pairs, ready to display.
    pub properties: Vec<(String, String)>,
    pub title: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
}

/// Messages from the UI to the render thread.
pub enum Request {
    Open { id: DocId, path: PathBuf, password: Option<String> },
    Close { id: DocId },
    /// Replace the set of wanted tiles. The list is in priority order.
    Want { doc: DocId, tiles: Vec<TileKey>, night: Option<NightStyle> },
    Extras { doc: DocId, page: u32 },
    Search { doc: DocId, generation: u64, query: String, options: SearchOptions, start_page: u32 },
    CancelSearch,
    /// Show the system print dialog (owned by window `owner`, a Win32 HWND) and print.
    Print { doc: DocId, owner: isize, title: String },
}

/// Messages from the render thread to the UI.
pub enum Event {
    Opened(Box<DocInfo>),
    OpenFailed { id: DocId, path: PathBuf, error: String, needs_password: bool },
    Tile { doc: DocId, key: TileKey, night: Option<NightStyle>, image: Arc<ColorImage> },
    TileFailed { doc: DocId, key: TileKey, error: String },
    Extras { doc: DocId, page: u32, extras: PageExtras },
    /// Every match on the page; a match that spans lines has several rectangles.
    SearchHits { doc: DocId, generation: u64, page: u32, matches: Vec<Vec<NormRect>> },
    SearchProgress { doc: DocId, generation: u64, searched: u32, total: u32 },
    SearchDone { doc: DocId, generation: u64 },
    /// `Ok(false)` when the print dialog was cancelled.
    PrintDone { result: Result<bool, String> },
    /// The PDFium library could not be loaded; nothing can be rendered.
    EngineFailed { error: String },
}

/// Handle to the render thread.
pub struct Engine {
    tx: Sender<Request>,
    rx: Receiver<Event>,
}

impl Engine {
    /// Start the render thread. `wake` is called after every event so the UI can repaint.
    /// `library_dirs` are searched for the PDFium shared library before the system paths.
    pub fn start(library_dirs: Vec<PathBuf>, wake: impl Fn() + Send + 'static) -> Self {
        let (tx, worker_rx) = channel();
        let (worker_tx, rx) = channel();
        std::thread::Builder::new()
            .name("pdf-render".into())
            .spawn(move || worker::run(library_dirs, worker_rx, worker_tx, Box::new(wake)))
            .expect("could not start the render thread");
        Engine { tx, rx }
    }

    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }

    /// Next pending event, if any. Never blocks.
    pub fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_page() {
        let tiles = tiles_for(0, 5000, 2500, Rotation::R0, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(tiles.len(), 3 * 3);
        let area: usize = tiles.iter().map(|t| t.bytes() / 4).sum();
        assert_eq!(area, 5000 * 2500);
        let last = tiles.last().unwrap();
        assert_eq!(last.pixel_rect(), [4096, 2048, 5000, 2500]);
        assert_eq!(last.norm_rect()[2], 1.0);
    }

    #[test]
    fn tiles_visible_subset() {
        let tiles = tiles_for(3, 5000, 4000, Rotation::R0, [0.5, 0.3, 0.6, 0.35]);
        assert_eq!(tiles.len(), 1);
        assert_eq!((tiles[0].col, tiles[0].row), (1, 1));
        // Exactly at the far edge still maps to the last tile.
        let tiles = tiles_for(3, 5000, 4000, Rotation::R0, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!((tiles[0].col, tiles[0].row), (2, 3));
    }

    #[test]
    fn small_page_is_one_tile() {
        let tiles = tiles_for(0, 300, 400, Rotation::R0, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].size(), [300, 400]);
    }

    #[test]
    fn rotation_round_trips() {
        for r in [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270] {
            let p = [0.2, 0.7];
            let q = r.unmap_point(r.map_point(p));
            assert!((p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6, "{r:?}");
        }
        // Rotating clockwise by 90°: the top-left corner goes to the top-right.
        assert_eq!(Rotation::R90.map_point([0.0, 0.0]), [1.0, 0.0]);
        assert_eq!(Rotation::R270.clockwise(), Rotation::R0);
        assert_eq!(Rotation::R0.counter_clockwise(), Rotation::R270);
    }
}
