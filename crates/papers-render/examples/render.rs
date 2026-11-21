//! Render a PDF page through the engine and report timings.
//!
//! cargo run --release -p papers-render --example render -- FILE.pdf [PAGE] [WIDTH] [OUT_PREFIX]
//!
//! Writes OUT_PREFIX-day.png and OUT_PREFIX-night.png (the first tile) when OUT_PREFIX is given,
//! and prints how long opening, rendering and the night-mode toggle took. Set PDFIUM_DIR to the
//! directory that holds the PDFium library.
use papers_render::{Engine, Event, NightStyle, Request, Rotation, tiles_for};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn wait(engine: &Engine, mut done: impl FnMut(Event) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        match engine.poll() {
            Some(e) => {
                if done(e) {
                    return;
                }
            }
            None => std::thread::sleep(Duration::from_micros(200)),
        }
    }
    panic!("timed out");
}

fn save(path: &str, image: &egui::ColorImage) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), image.size[0] as u32, image.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    let mut w = enc.write_header().unwrap();
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    w.write_image_data(&bytes).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let file = PathBuf::from(&args[1]);
    let page: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let width: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1600);
    let out = args.get(4).cloned();
    let dirs = std::env::var_os("PDFIUM_DIR").map(PathBuf::from).into_iter().collect();
    let engine = Engine::start(dirs, || {});

    let t = Instant::now();
    engine.send(Request::Open { id: 1, path: file, password: None });
    let mut info = None;
    wait(&engine, |e| match e {
        Event::Opened(i) => {
            info = Some(i);
            true
        }
        Event::OpenFailed { error, .. } | Event::EngineFailed { error } => panic!("{error}"),
        _ => false,
    });
    let info = info.unwrap();
    println!("open: {:?}, {} pages, outline items: {}", t.elapsed(), info.page_sizes.len(), info.outline.len());
    for group in &info.properties {
        for (k, v) in &group.rows {
            println!("  {}: {k}: {v}", group.title);
        }
    }
    let [pw, ph] = info.page_sizes[page as usize];
    let height = (width as f32 * ph / pw).round() as u32;
    let tiles = tiles_for(page, width, height, Rotation::R0, [0.0, 0.0, 1.0, 1.0]);
    let n = tiles.len();

    for (label, night) in [("day (render)", None), ("night (filter only, cached)", Some(NightStyle::Smart)), ("day (cached)", None)] {
        let t = Instant::now();
        engine.send(Request::Want { doc: 1, tiles: tiles.clone(), night });
        let mut got = 0;
        wait(&engine, |e| match e {
            Event::Tile { image, key, night, .. } => {
                if got == 0 && let Some(prefix) = &out {
                    let suffix = if night.is_some() { "night" } else { "day" };
                    if key.col == 0 && key.row == 0 {
                        save(&format!("{prefix}-{suffix}.png"), &image);
                    }
                }
                got += 1;
                got == n
            }
            Event::TileFailed { error, .. } => panic!("{error}"),
            _ => false,
        });
        println!("{label}: {n} tiles of a {width}x{height} page in {:?}", t.elapsed());
    }

    let t = Instant::now();
    engine.send(Request::Extras { doc: 1, page });
    wait(&engine, |e| match e {
        Event::Extras { extras, .. } => {
            let text: String = extras.chars.iter().take(80).map(|c| c.text.as_str()).collect();
            println!("extras: {} chars, {} links in {:?}: {text:?}", extras.chars.len(), extras.links.len(), t.elapsed());
            for l in extras.links.iter().take(5) {
                println!("  link {:?} -> {:?}", l.rect, l.target);
            }
            true
        }
        _ => false,
    });

    if let Some(word) = args.get(5) {
        let t = Instant::now();
        engine.send(Request::Search {
            doc: 1,
            generation: 1,
            query: word.clone(),
            options: papers_render::SearchOptions { match_case: false, whole_word: false },
            start_page: 0,
        });
        let mut hits = 0;
        wait(&engine, |e| match e {
            Event::SearchHits { matches, .. } => {
                hits += matches.len();
                false
            }
            Event::SearchDone { .. } => true,
            _ => false,
        });
        println!("search {word:?}: {hits} matches in {:?}", t.elapsed());
    }
}
