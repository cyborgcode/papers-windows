//! End-to-end rendering checks against a real PDFium library.
//!
//! Skipped unless PDFIUM_DIR points at a directory with the PDFium shared library and
//! PAPERS_SAMPLE_PDF at a multi-page PDF (CI sets both; see ci/make-samples.py).
use papers_render::{ColorImage, Engine, Event, NightStyle, Request, Rotation, TILE_H, TILE_W, tiles_for};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn setup() -> Option<(Engine, Box<papers_render::DocInfo>)> {
    let dir = PathBuf::from(std::env::var_os("PDFIUM_DIR")?);
    let pdf = PathBuf::from(std::env::var_os("PAPERS_SAMPLE_PDF")?);
    let engine = Engine::start(vec![dir], || {});
    engine.send(Request::Open { id: 7, path: pdf, password: None });
    let info = loop {
        match next(&engine) {
            Event::Opened(info) => break info,
            Event::OpenFailed { error, .. } | Event::EngineFailed { error } => panic!("{error}"),
            _ => {}
        }
    };
    Some((engine, info))
}

fn next(engine: &Engine) -> Event {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(e) = engine.poll() {
            return e;
        }
        assert!(Instant::now() < deadline, "timed out waiting for the render thread");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Render every tile of a page and stitch them into one image.
fn render_page(engine: &Engine, page: u32, width: u32, height: u32, rotation: Rotation, night: Option<NightStyle>) -> ColorImage {
    let tiles = tiles_for(page, width, height, rotation, [0.0, 0.0, 1.0, 1.0]);
    engine.send(Request::Want { doc: 7, tiles: tiles.clone(), night });
    let mut out = ColorImage::new([width as usize, height as usize], vec![egui::Color32::TRANSPARENT; (width * height) as usize]);
    let mut left = tiles.len();
    while left > 0 {
        if let Event::Tile { key, image, night: n, .. } = next(engine) {
            assert_eq!(n, night);
            assert_eq!(image.size, [key.size()[0] as usize, key.size()[1] as usize]);
            let [x0, y0, _, _] = key.pixel_rect();
            blit(&mut out, &image, x0 as usize, y0 as usize);
            left -= 1;
        }
    }
    out
}

fn blit(dst: &mut ColorImage, src: &Arc<ColorImage>, x0: usize, y0: usize) {
    for y in 0..src.size[1] {
        for x in 0..src.size[0] {
            dst.pixels[(y0 + y) * dst.size[0] + x0 + x] = src.pixels[y * src.size[0] + x];
        }
    }
}

fn at(img: &ColorImage, x: usize, y: usize) -> egui::Color32 {
    img.pixels[y * img.size[0] + x]
}

#[test]
fn tiles_stitch_rotate_and_go_dark() {
    let Some((engine, info)) = setup() else {
        eprintln!("PDFIUM_DIR / PAPERS_SAMPLE_PDF not set; skipping");
        return;
    };
    assert!(info.page_sizes.len() >= 2);
    let [pw, ph] = info.page_sizes[0];
    // Big enough to need several tiles in both directions.
    let width = TILE_W + 500;
    let height = (width as f32 * ph / pw).round() as u32;
    assert!(height > TILE_H * 2);
    let day = render_page(&engine, 0, width, height, Rotation::R0, None);
    assert!(day.pixels.iter().all(|p| p.a() == 255), "every pixel is covered by a tile");
    // Some ink, mostly paper.
    let dark_px = day.pixels.iter().filter(|p| p.r() < 128).count();
    assert!(dark_px > 100 && dark_px < day.pixels.len() / 4, "{dark_px}");

    // Rotating by 180° maps each pixel to the opposite corner.
    let flipped = render_page(&engine, 0, width, height, Rotation::R180, None);
    let mut differences = 0;
    for &(x, y) in &[(100, 100), (width as usize / 2, height as usize / 3), (width as usize - 7, 5), (37, height as usize - 3)] {
        let a = at(&day, x, y);
        let b = at(&flipped, width as usize - 1 - x, height as usize - 1 - y);
        if (a.r() as i32 - b.r() as i32).abs() > 40 {
            differences += 1;
        }
    }
    assert_eq!(differences, 0);
    let ink_day = day.pixels.iter().filter(|p| p.r() < 128).count() as f64;
    let ink_flip = flipped.pixels.iter().filter(|p| p.r() < 128).count() as f64;
    assert!((ink_day - ink_flip).abs() / ink_day < 0.02, "{ink_day} vs {ink_flip}");

    // A sideways rotation swaps the page dimensions.
    let side = render_page(&engine, 0, height / 2, width / 2, Rotation::R90, None);
    assert_eq!(side.size, [(height / 2) as usize, (width / 2) as usize]);

    // Night mode turns the paper dark and the ink light, from the cache.
    let started = Instant::now();
    let night = render_page(&engine, 0, width, height, Rotation::R0, Some(NightStyle::Smart));
    let toggle = started.elapsed();
    let paper = at(&day, 3, 3);
    assert_eq!(paper, egui::Color32::WHITE);
    assert!(at(&night, 3, 3).r() < 60);
    eprintln!("night toggle of a {width}x{height} page took {toggle:?}");

    // Text and links are available, and search finds words that are on the page.
    engine.send(Request::Extras { doc: 7, page: 0 });
    let extras = loop {
        if let Event::Extras { extras, .. } = next(&engine) {
            break extras;
        }
    };
    let text: String = extras.chars.iter().map(|c| c.text.as_str()).collect();
    assert!(!text.trim().is_empty());
    assert!(extras.chars.iter().any(|c| c.is_visible()));
    let word = text.split_whitespace().find(|w| w.len() > 3).expect("a word").to_string();
    engine.send(Request::Search {
        doc: 7,
        generation: 1,
        query: word.clone(),
        options: papers_render::SearchOptions { match_case: true, whole_word: false },
        start_page: 0,
    });
    let mut hits = 0;
    loop {
        match next(&engine) {
            Event::SearchHits { page, matches, .. } => {
                if page == 0 {
                    hits += matches.len();
                }
            }
            Event::SearchDone { .. } => break,
            _ => {}
        }
    }
    assert!(hits > 0, "searching for {word:?} found nothing");
}
