# Papers for Windows

A fast PDF reader for Windows, written in Rust, with the look of GNOME Papers.
It is a rebuild of the earlier GTK port of GNOME Papers, designed so the window
never waits on rendering. It has a night mode that switches instantly and keeps
the page's colours readable, and it updates itself.

## Looks like GNOME Papers

The interface follows Papers 50: the sidebar with its own header bar
(search, "Document Viewer", main menu) and thumbnails / outline switched at the
bottom, a find sidebar listing every match in its line of text, the document
header bar with the page selector and the ⋮ document menu, round zoom buttons
floating in the corner, and GNOME's start, loading, password and error pages.
It uses libadwaita's colours in light and dark mode (following Windows), the
Adwaita Sans font, GNOME-style window buttons in the header bar, and Papers'
keyboard shortcuts. Start it with `--native-titlebar` (or set
`PAPERS_NATIVE_TITLEBAR=1`) to use the normal Windows title bar instead.

## Why it is fast

- **Rendering never blocks the window.** PDFium runs on its own thread. The
  window only draws finished images, so scrolling and zooming stay smooth even
  on heavy pages.
- **Only what you see is rendered.** Pages are cut into tiles. The visible
  tiles are rendered first, then a few pages ahead. When you scroll past
  something, it is dropped from the queue instead of rendered for nothing.
- **Zooming is instant.** While you zoom with Ctrl+wheel or a pinch, the tiles
  you already have are stretched. Sharp tiles at the new size are rendered once
  the zoom stops changing, and they appear on top without any blank frames.
- **Night mode doesn't lag.** Pages are rendered once in day colours and kept
  in memory. Night mode is a colour filter that runs in parallel on all cores
  in a few milliseconds per page; it doesn't re-render anything. Switching back
  to day mode reuses the cached tiles.
- **New tiles reach the GPU gradually.** At most 48 MB are uploaded per frame,
  so a burst of tiles never causes a stutter.

Night mode (Ctrl+I) uses the same filter as GNOME Papers, `invert(1)
hue-rotate(180deg)`: black and white swap, but red stays red and blue links stay
blue.

## Features

- Continuous or page-by-page scrolling, dual pages (odd pages left, right to
  left), rotation, fit width and fit page, zoom from 5 % to 6400 %
- Search with case-sensitive and whole-word options; results are listed in the
  sidebar and every match is highlighted
- Text selection and copying (drag over text; double-click selects a word),
  clickable links and a right-click menu
- Sidebar with the outline (table of contents) and page thumbnails
- Night mode (Ctrl+I), fullscreen (F11) and presentation mode (F5)
- Password-protected PDFs, forms and annotations, document properties
- Printing through the Windows print dialog (Ctrl+P), Save As, Open With
- Remembers each document's page, zoom and rotation, and keeps a recent-files
  list
- Reloads automatically when the file changes on disk, which is handy when
  writing LaTeX
- Opens files from the command line (`papers.exe file.pdf --page 12`), by drag
  and drop, and through "Open with"

Press **Keyboard Shortcuts** in the main menu (Ctrl+?) to see every shortcut.

## Install

From this repository's **Releases** page:

- `Papers-<version>-windows-x64-setup.exe` installs Papers, adds a Start menu
  entry and registers it under "Open with" for PDF files. It upgrades an
  existing installation of the GTK build in place.
- `Papers-<version>-windows-x64-portable.zip` runs without installing: unzip it
  and start `Papers\papers.exe`.

## Updates

Papers checks for a new release once a day; this can be turned off in main
menu → Check for Updates. When a new release is out, a bar offers **Update and
restart**. The update:

1. downloads the setup program or the portable zip, whichever matches your copy,
2. checks its size and SHA-256 against the release's `latest.json`,
3. hands over to `papers-updater.exe`, which waits for Papers to close and then
   installs it: the setup program runs silently, or the zip is unpacked over the
   portable folder (if any file can't be replaced, the old files are put back),
4. starts Papers again on the same document.

You can also run the updater yourself:

```bash
papers-updater check
```

```bash
papers-updater update
```

## Building

You need Rust (stable) and the PDFium library from
[pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases). Put
`pdfium.dll` (Windows) or `libpdfium.so` (Linux) next to the executable, or
point `PDFIUM_DIR` at its folder.

```bash
cargo build --release -p papers -p papers-updater
```

```bash
cargo test -p papers-render -p papers-update -p papers
```

The rendering tests need `PDFIUM_DIR` and `PAPERS_SAMPLE_PDF`; without them,
they are skipped.

| Path | What it is |
| --- | --- |
| `crates/papers-render` | Render thread: PDFium, tiles, cache, night filter, search, text and links, printing |
| `crates/papers-update` | Release manifest, version checks, verified downloads, installing updates |
| `apps/papers` | The reader (egui/wgpu); `src/adw.rs` is the GNOME/Adwaita look |
| `apps/papers-updater` | `papers-updater.exe` |
| `packaging/papers-rs.iss` | Inno Setup script |
| `.github/workflows/build.yml` | CI: tests, Windows build, smoke test, installer, portable zip, `latest.json`, release |

The [workflow](.github/workflows/build.yml) builds on a Windows runner, starts
the app on a sample PDF and takes screenshots through a keyboard-driven tour.
It then packages the installer and the portable zip and writes `latest.json`.
On `main`, it publishes a release tagged `v<version>`. To ship an update, bump
`version` in the top-level `Cargo.toml`.

### The earlier GTK build

The scripts that built GNOME Papers 50.2 with GTK 4 from MSYS2 (`ci/build.sh`,
`ci/port-to-windows.py`, `exempi-shim/`, `packaging/papers.iss`, and so on) are
still here. Their workflow, [`build-windows.yml`](.github/workflows/build-windows.yml),
now runs only when started by hand. Unlike the GTK build, the Rust build reads
PDF only; DjVu, TIFF and comic books aren't supported yet.

## License

GPL-2.0-or-later. PDFium is BSD-3-Clause and the Adwaita Sans font is under the
SIL Open Font License 1.1; their licenses are shipped in `licenses\`.
