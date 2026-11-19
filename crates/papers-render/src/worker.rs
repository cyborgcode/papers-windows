//! The render thread. Owns PDFium and every open document.

use crate::{
    Dest, DocId, DocInfo, Event, Link, LinkTarget, NightStyle, NormRect, OutlineItem, PageExtras, Request, Rotation,
    SearchOptions, TextChar, TileKey, night::Filter,
};
use egui::{Color32, ColorImage};
use pdfium_render::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Instant;

/// Upper bound for cached raw tiles (day-mode pixels), in bytes.
const RAW_CACHE_BYTES: usize = 384 * 1024 * 1024;
/// Loaded (parsed) pages kept per document. Parsing a page's content is often the
/// most expensive part of rendering it, and neighbouring tiles share it.
const PAGE_CACHE: usize = 8;
/// Pixel size of the virtual page used to convert PDF points to normalised coordinates.
const REF: i32 = 100_000;

pub(crate) fn run(library_dirs: Vec<PathBuf>, rx: Receiver<Request>, tx: Sender<Event>, wake: Box<dyn Fn() + Send>) {
    let pdfium = match bind(&library_dirs) {
        Ok(p) => &*Box::leak(Box::new(p)),
        Err(error) => {
            log::error!("PDFium could not be loaded: {error}");
            let _ = tx.send(Event::EngineFailed { error: error.clone() });
            wake();
            // Keep answering so the UI can show a clear message instead of hanging.
            while let Ok(request) = rx.recv() {
                if let Request::Open { id, path, .. } = request {
                    let _ = tx.send(Event::OpenFailed { id, path, error: error.clone(), needs_password: false });
                    wake();
                }
            }
            return;
        }
    };
    let mut worker = Worker {
        pdfium,
        tx,
        wake,
        docs: HashMap::new(),
        queue: VecDeque::new(),
        night: None,
        filters: HashMap::new(),
        raw: RawCache::default(),
        extras: VecDeque::new(),
        search: None,
    };
    worker.run(rx);
}

fn bind(dirs: &[PathBuf]) -> Result<Pdfium, String> {
    let mut errors = Vec::new();
    for dir in dirs {
        let path = Pdfium::pdfium_platform_library_name_at_path(dir);
        if !path.exists() {
            continue;
        }
        match Pdfium::bind_to_library(&path) {
            Ok(bindings) => return Ok(Pdfium::new(bindings)),
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    match Pdfium::bind_to_system_library() {
        Ok(bindings) => Ok(Pdfium::new(bindings)),
        Err(e) => {
            errors.push(format!("system library: {e}"));
            Err(errors.join("; "))
        }
    }
}

struct SearchJob {
    doc: DocId,
    generation: u64,
    query: String,
    options: SearchOptions,
    order: Vec<u32>,
    next: usize,
}

struct Worker {
    pdfium: &'static Pdfium,
    tx: Sender<Event>,
    wake: Box<dyn Fn() + Send>,
    docs: HashMap<DocId, OpenDoc>,
    queue: VecDeque<(DocId, TileKey)>,
    night: Option<NightStyle>,
    filters: HashMap<NightStyle, Filter>,
    raw: RawCache,
    extras: VecDeque<(DocId, u32)>,
    search: Option<SearchJob>,
}

impl Worker {
    fn send(&self, event: Event) {
        let _ = self.tx.send(event);
        (self.wake)();
    }

    fn has_work(&self) -> bool {
        !self.queue.is_empty() || !self.extras.is_empty() || self.search.is_some()
    }

    fn run(&mut self, rx: Receiver<Request>) {
        loop {
            // Apply every pending request before doing any work, so that the newest
            // wanted-tile list always wins and stale work is never started.
            loop {
                let request = if self.has_work() {
                    match rx.try_recv() {
                        Ok(r) => r,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                } else {
                    match rx.recv() {
                        Ok(r) => r,
                        Err(_) => return,
                    }
                };
                self.handle(request);
            }

            if let Some((doc, page)) = self.extras.pop_front() {
                self.page_extras(doc, page);
            } else if let Some((doc, key)) = self.queue.pop_front() {
                self.render(doc, key);
            } else if self.search.is_some() {
                self.search_step();
            }
        }
    }

    fn handle(&mut self, request: Request) {
        match request {
            Request::Open { id, path, password } => self.open(id, path, password),
            Request::Close { id } => {
                self.docs.remove(&id);
                self.raw.remove_doc(id);
                self.queue.retain(|(d, _)| *d != id);
                self.extras.retain(|(d, _)| *d != id);
                if self.search.as_ref().is_some_and(|s| s.doc == id) {
                    self.search = None;
                }
            }
            Request::Want { doc, tiles, night } => {
                self.night = night;
                self.queue = tiles.into_iter().map(|t| (doc, t)).collect();
            }
            Request::Extras { doc, page } => {
                if !self.extras.contains(&(doc, page)) {
                    self.extras.push_back((doc, page));
                }
            }
            Request::Search { doc, generation, query, options, start_page } => {
                let pages = self.docs.get(&doc).map_or(0, |d| d.page_count);
                let start = start_page.min(pages.saturating_sub(1));
                let order = (start..pages).chain(0..start).collect();
                self.search = Some(SearchJob { doc, generation, query, options, order, next: 0 });
            }
            Request::CancelSearch => self.search = None,
            Request::Print { doc, owner, title } => {
                let result = match self.docs.get(&doc) {
                    Some(d) => crate::print::print(&d.document, &title, owner),
                    None => Err("The document is not open.".into()),
                };
                self.send(Event::PrintDone { result });
            }
        }
    }

    fn open(&mut self, id: DocId, path: PathBuf, password: Option<String>) {
        let started = Instant::now();
        let result = self.pdfium.load_pdf_from_file(&path, password.as_deref());
        let document = match result {
            Ok(d) => d,
            Err(e) => {
                let needs_password =
                    matches!(e, PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError));
                let error = if needs_password { "This document is password protected.".to_string() } else { describe(&e) };
                self.send(Event::OpenFailed { id, path, error, needs_password });
                return;
            }
        };
        let page_sizes: Vec<[f32; 2]> = match document.pages().page_sizes() {
            Ok(sizes) => sizes.iter().map(|r| [r.width().value.abs().max(1.0), r.height().value.abs().max(1.0)]).collect(),
            Err(e) => {
                self.send(Event::OpenFailed { id, path, error: describe(&e), needs_password: false });
                return;
            }
        };
        let outline = read_outline(&document, &page_sizes);
        let (properties, title) = read_properties(&document, &path, &page_sizes);
        log::info!("opened {} ({} pages) in {:?}", path.display(), page_sizes.len(), started.elapsed());
        let page_count = page_sizes.len() as u32;
        self.docs.insert(id, OpenDoc { pages: VecDeque::new(), document, page_count, page_sizes: page_sizes.clone().into() });
        self.send(Event::Opened(Box::new(DocInfo { id, path, page_sizes, outline, properties, title })));
    }

    fn filter(&mut self, style: NightStyle) -> &Filter {
        self.filters.entry(style).or_insert_with(|| Filter::new(style))
    }

    fn render(&mut self, doc_id: DocId, key: TileKey) {
        let raw = match self.raw.get(doc_id, &key) {
            Some(raw) => raw,
            None => {
                let Some(doc) = self.docs.get_mut(&doc_id) else { return };
                match doc.render(&key) {
                    Ok(pixels) => {
                        let [w, h] = key.size();
                        let pixels = Arc::new(ColorImage::new([w as usize, h as usize], pixels));
                        self.raw.insert(doc_id, key, pixels.clone());
                        pixels
                    }
                    Err(error) => {
                        log::warn!("page {} tile failed: {error}", key.page + 1);
                        self.send(Event::TileFailed { doc: doc_id, key, error });
                        return;
                    }
                }
            }
        };
        let night = self.night;
        // Day mode shares the cached buffer; night mode filters it into a new one.
        let image = match night {
            Some(style) => Arc::new(ColorImage::new(raw.size, self.filter(style).apply_copy(&raw.pixels))),
            None => raw,
        };
        self.send(Event::Tile { doc: doc_id, key, night, image });
    }

    fn page_extras(&mut self, doc_id: DocId, index: u32) {
        let Some(doc) = self.docs.get_mut(&doc_id) else { return };
        let page_sizes = doc.page_sizes.clone();
        let extras = match doc.page(index) {
            Ok(page) => read_extras(page, &page_sizes),
            Err(_) => PageExtras::default(),
        };
        self.send(Event::Extras { doc: doc_id, page: index, extras });
    }

    fn search_step(&mut self) {
        let Some(job) = self.search.as_mut() else { return };
        let total = job.order.len() as u32;
        let Some(&index) = job.order.get(job.next) else {
            let (doc, generation) = (job.doc, job.generation);
            self.search = None;
            self.send(Event::SearchDone { doc, generation });
            return;
        };
        job.next += 1;
        let (doc_id, generation, searched) = (job.doc, job.generation, job.next as u32);
        let Some(doc) = self.docs.get(&doc_id) else {
            self.search = None;
            return;
        };
        let matches = match doc.document.pages().get(index as u16 as PdfPageIndex) {
            Ok(page) => search_page(&page, &job.query, &job.options),
            Err(_) => Vec::new(),
        };
        if !matches.is_empty() {
            self.send(Event::SearchHits { doc: doc_id, generation, page: index, matches });
        }
        if searched % 16 == 0 || searched == total {
            self.send(Event::SearchProgress { doc: doc_id, generation, searched, total });
        }
    }
}

struct OpenDoc {
    // Field order matters: loaded pages must be dropped before their document.
    pages: VecDeque<(u32, PdfPage<'static>)>,
    document: PdfDocument<'static>,
    page_count: u32,
    page_sizes: Arc<[[f32; 2]]>,
}

impl OpenDoc {
    fn page(&mut self, index: u32) -> Result<&PdfPage<'static>, String> {
        if let Some(pos) = self.pages.iter().position(|(i, _)| *i == index) {
            let entry = self.pages.remove(pos).expect("position is valid");
            self.pages.push_back(entry);
        } else {
            if index >= self.page_count {
                return Err("page out of range".into());
            }
            let page = self.document.pages().get(index as PdfPageIndex).map_err(|e| describe(&e))?;
            if self.pages.len() >= PAGE_CACHE {
                self.pages.pop_front();
            }
            self.pages.push_back((index, page));
        }
        Ok(&self.pages.back().expect("just pushed").1)
    }

    fn render(&mut self, key: &TileKey) -> Result<Vec<Color32>, String> {
        let page = self.page(key.page)?;
        let [x0, y0, _, _] = key.pixel_rect();
        let [tw, th] = key.size();
        if tw == 0 || th == 0 {
            return Err("empty tile".into());
        }
        let mut bitmap = PdfBitmap::empty(tw as Pixels, th as Pixels, PdfBitmapFormat::BGRA).map_err(|e| describe(&e))?;
        let config = PdfRenderConfig::new()
            .set_fixed_size(key.width as Pixels, key.height as Pixels)
            .rotate(render_rotation(key.rotation), false)
            .set_origin(-(x0 as Pixels), -(y0 as Pixels))
            .set_reverse_byte_order(true)
            .render_annotations(true)
            .render_form_data(true)
            .set_clear_color(PdfColor::WHITE);
        page.render_into_bitmap_with_config(&mut bitmap, &config).map_err(|e| describe(&e))?;
        let bytes = bitmap.as_raw_bytes();
        let (tw, th) = (tw as usize, th as usize);
        let stride = bytes.len() / th;
        if stride < tw * 4 {
            return Err("unexpected bitmap layout".into());
        }
        let mut out = Vec::with_capacity(tw * th);
        for row in bytes.chunks_exact(stride).take(th) {
            out.extend(row[..tw * 4].chunks_exact(4).map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], 255)));
        }
        Ok(out)
    }
}

fn render_rotation(r: Rotation) -> PdfPageRenderRotation {
    match r {
        Rotation::R0 => PdfPageRenderRotation::None,
        Rotation::R90 => PdfPageRenderRotation::Degrees90,
        Rotation::R180 => PdfPageRenderRotation::Degrees180,
        Rotation::R270 => PdfPageRenderRotation::Degrees270,
    }
}

fn describe(e: &PdfiumError) -> String {
    match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => {
            "The file is not a PDF document or it is damaged.".into()
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FileError) => {
            "The file could not be opened.".into()
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::SecurityError) => {
            "This document uses an unsupported security handler.".into()
        }
        PdfiumError::IoError(io) => io.to_string(),
        other => other.to_string(),
    }
}

/// Converts PDF page-space points to normalised display coordinates of one loaded page.
struct Normaliser<'p, 'a> {
    page: &'p PdfPage<'a>,
    config: PdfRenderConfig,
}

impl<'p, 'a> Normaliser<'p, 'a> {
    fn new(page: &'p PdfPage<'a>) -> Self {
        Normaliser { page, config: PdfRenderConfig::new().set_fixed_size(REF, REF) }
    }

    fn point(&self, x: PdfPoints, y: PdfPoints) -> Option<[f32; 2]> {
        let (px, py) = self.page.points_to_pixels(x, y, &self.config).ok()?;
        Some([px as f32 / REF as f32, py as f32 / REF as f32])
    }

    fn rect(&self, r: PdfRect) -> NormRect {
        let (Some(a), Some(b)) = (self.point(r.left(), r.top()), self.point(r.right(), r.bottom())) else {
            return [0.0; 4];
        };
        let c = |v: f32| v.clamp(0.0, 1.0);
        [c(a[0].min(b[0])), c(a[1].min(b[1])), c(a[0].max(b[0])), c(a[1].max(b[1]))]
    }
}

fn read_extras(page: &PdfPage, page_sizes: &[[f32; 2]]) -> PageExtras {
    let norm = Normaliser::new(page);
    let mut extras = PageExtras::default();
    if let Ok(text) = page.text() {
        let chars = text.chars();
        extras.chars.reserve(chars.len());
        for ch in chars.iter() {
            let text = ch.unicode_string().unwrap_or_default();
            let rect = match ch.loose_bounds() {
                Ok(b) if b.width().value > 0.0 && b.height().value > 0.0 => norm.rect(b),
                _ => [0.0; 4],
            };
            extras.chars.push(TextChar { text, rect });
        }
    }
    for link in page.links().iter() {
        let Ok(rect) = link.rect() else { continue };
        let target = match link.action() {
            Some(PdfAction::Uri(action)) => action.uri().ok().map(LinkTarget::Uri),
            Some(PdfAction::LocalDestination(action)) => {
                action.destination().ok().and_then(|d| dest_of(&d, page_sizes)).map(LinkTarget::Dest)
            }
            _ => None,
        }
        .or_else(|| link.destination().and_then(|d| dest_of(&d, page_sizes)).map(LinkTarget::Dest));
        if let Some(target) = target {
            extras.links.push(Link { rect: norm.rect(rect), target });
        }
    }
    extras
}

fn search_page(page: &PdfPage, query: &str, options: &SearchOptions) -> Vec<Vec<NormRect>> {
    let norm = Normaliser::new(page);
    let mut matches = Vec::new();
    let Ok(text) = page.text() else { return matches };
    let search_options = PdfSearchOptions::new().match_case(options.match_case).match_whole_word(options.whole_word);
    let Ok(search) = text.search(query, &search_options) else { return matches };
    for segments in search.iter(PdfSearchDirection::SearchForward) {
        // A match spanning several lines comes back as several segments.
        let rects: Vec<NormRect> = segments.iter().map(|s| norm.rect(s.bounds())).collect();
        if !rects.is_empty() {
            matches.push(rects);
        }
        if matches.len() >= 10_000 {
            break;
        }
    }
    matches
}

fn dest_of(dest: &PdfDestination, page_sizes: &[[f32; 2]]) -> Option<Dest> {
    let page = dest.page_index().ok()? as u32;
    let height = page_sizes.get(page as usize)?[1];
    // Vertical target from the destination's view settings, converted from PDF user space
    // (measured from the bottom) to a fraction from the top. Exact for the common case of
    // an unrotated page whose media box starts at zero.
    let y = match dest.view_settings().ok()? {
        PdfDestinationViewSettings::SpecificCoordinatesAndZoom(_, Some(y), _)
        | PdfDestinationViewSettings::FitPageHorizontallyToWindow(Some(y))
        | PdfDestinationViewSettings::FitBoundsHorizontallyToWindow(Some(y)) => Some(y.value),
        PdfDestinationViewSettings::FitPageToRectangle(r) => Some(r.top().value),
        _ => None,
    };
    Some(Dest { page, y: y.map(|y| (1.0 - y / height).clamp(0.0, 1.0)) })
}

fn read_outline(document: &PdfDocument<'static>, page_sizes: &[[f32; 2]]) -> Vec<OutlineItem> {
    fn walk(
        first: Option<PdfBookmark>,
        page_sizes: &[[f32; 2]],
        budget: &mut usize,
        depth: usize,
    ) -> Vec<OutlineItem> {
        let mut out = Vec::new();
        let mut current = first;
        while let Some(bookmark) = current {
            if *budget == 0 || depth > 32 {
                break;
            }
            *budget -= 1;
            let title = bookmark.title().unwrap_or_default().trim().to_string();
            let dest = bookmark
                .destination()
                .and_then(|d| dest_of(&d, page_sizes))
                .or_else(|| match bookmark.action() {
                    Some(PdfAction::LocalDestination(l)) => l.destination().ok().and_then(|d| dest_of(&d, page_sizes)),
                    _ => None,
                })
                ;
            let children = walk(bookmark.first_child(), page_sizes, budget, depth + 1);
            out.push(OutlineItem { title, dest, children });
            current = bookmark.next_sibling();
        }
        out
    }
    let mut budget = 20_000;
    walk(document.bookmarks().root(), page_sizes, &mut budget, 0)
}

fn read_properties(document: &PdfDocument, path: &Path, page_sizes: &[[f32; 2]]) -> (Vec<(String, String)>, Option<String>) {
    use PdfDocumentMetadataTagType as T;
    let meta = document.metadata();
    let get = |t: T| meta.get(t).map(|v| v.value().trim().to_string()).filter(|v| !v.is_empty());
    let title = get(T::Title);
    let mut props = Vec::new();
    let mut push = |label: &str, value: Option<String>| {
        if let Some(v) = value {
            props.push((label.to_string(), v));
        }
    };
    push("Title", title.clone());
    push("Author", get(T::Author));
    push("Subject", get(T::Subject));
    push("Keywords", get(T::Keywords));
    push("Creator", get(T::Creator));
    push("Producer", get(T::Producer));
    push("Created", get(T::CreationDate).map(|d| pdf_date(&d)));
    push("Modified", get(T::ModificationDate).map(|d| pdf_date(&d)));
    push("Pages", Some(page_sizes.len().to_string()));
    if let Some([w, h]) = page_sizes.first() {
        let mm = |pt: f32| pt / 72.0 * 25.4;
        push("Page size", Some(format!("{:.0} × {:.0} mm ({:.1} × {:.1} in)", mm(*w), mm(*h), w / 72.0, h / 72.0)));
    }
    push("PDF version", pdf_version(document.version()));
    if let Ok(md) = std::fs::metadata(path) {
        push("File size", Some(human_size(md.len())));
    }
    push("Location", Some(path.display().to_string()));
    (props, title)
}

fn pdf_version(v: PdfDocumentVersion) -> Option<String> {
    let s = format!("{v:?}");
    let rest = s.strip_prefix("Pdf")?;
    Some(rest.replace('_', "."))
}

/// `D:20240131093000+01'00'` → `2024-01-31 09:30`.
pub(crate) fn pdf_date(raw: &str) -> String {
    let s = raw.strip_prefix("D:").unwrap_or(raw);
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let part = |a: usize, b: usize| digits.get(a..b);
    match (part(0, 4), part(4, 6), part(6, 8)) {
        (Some(y), Some(m), Some(d)) => match (part(8, 10), part(10, 12)) {
            (Some(hh), Some(mm)) => format!("{y}-{m}-{d} {hh}:{mm}"),
            _ => format!("{y}-{m}-{d}"),
        },
        (Some(y), _, _) => y.to_string(),
        _ => raw.to_string(),
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} bytes") } else { format!("{v:.1} {}", UNITS[unit]) }
}

#[derive(Default)]
struct RawCache {
    map: HashMap<(DocId, TileKey), (Arc<ColorImage>, u64)>,
    bytes: usize,
    tick: u64,
}

impl RawCache {
    fn get(&mut self, doc: DocId, key: &TileKey) -> Option<Arc<ColorImage>> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(&(doc, *key)).map(|(px, t)| {
            *t = tick;
            px.clone()
        })
    }

    fn insert(&mut self, doc: DocId, key: TileKey, pixels: Arc<ColorImage>) {
        self.tick += 1;
        self.bytes += key.bytes();
        if let Some((old, _)) = self.map.insert((doc, key), (pixels, self.tick)) {
            self.bytes -= old.pixels.len() * 4;
        }
        while self.bytes > RAW_CACHE_BYTES && self.map.len() > 1 {
            let oldest = *self.map.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k).expect("non-empty");
            if let Some((px, _)) = self.map.remove(&oldest) {
                self.bytes -= px.pixels.len() * 4;
            }
        }
    }

    fn remove_doc(&mut self, doc: DocId) {
        let mut freed = 0;
        self.map.retain(|(d, _), (px, _)| {
            let keep = *d != doc;
            if !keep {
                freed += px.pixels.len() * 4;
            }
            keep
        });
        self.bytes -= freed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(pdf_date("D:20240131093000+01'00'"), "2024-01-31 09:30");
        assert_eq!(pdf_date("D:20240131"), "2024-01-31");
        assert_eq!(pdf_date("garbage"), "garbage");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(10), "10 bytes");
        assert_eq!(human_size(1536), "1.5 KB");
    }
}
