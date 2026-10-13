# Papers for Windows

A Windows build of [GNOME Papers](https://gitlab.gnome.org/GNOME/papers)
(the GNOME "Document Viewer") **50.2**. It is the real application, compiled
for Windows with GTK 4 and libadwaita from MSYS2, so it has the same
features and interface as the Linux version:

- PDF, DjVu, TIFF and comic books (CBZ, CBR, CB7, CBT)
- thumbnails, outline, annotations, attachments and layers sidebars
- search with whole-word and case-sensitive options
- continuous, dual-page, odd-pages-left and right-to-left layouts, rotation, zoom
- night mode, fullscreen and presentation mode with transitions
- highlight, underline, strike-through, squiggly, notes, free text, ink and
  highlighter annotations, plus an eraser
- forms, password-protected documents, document properties and digital
  signature verification
- printing through the native Windows print dialog

## Install

Download the setup program or the portable zip from this repository's
**Releases** page:

- `Papers-50.2-windows-x64-setup.exe` installs Papers, adds a Start menu
  entry and lists it under "Open with" for supported files.
- `Papers-50.2-windows-x64-portable.zip` runs without installing: unzip it
  and start `Papers\bin\papers.exe`.

## How it is built

This repository does not contain a copy of the Papers source. The
[workflow](.github/workflows/build-windows.yml) runs on a GitHub Windows
runner and:

1. fetches the upstream `50.2` tag and applies the Windows changes with
   [`ci/port-to-windows.py`](ci/port-to-windows.py) (the resulting diff is
   saved as `windows-port.patch` in each run's logs),
2. builds [`exempi-shim`](exempi-shim), a small libxml2-based
   implementation of the read-only exempi API that Papers uses for XMP
   metadata (exempi itself is not available for Windows),
3. compiles Papers with Meson, Cargo and the MSYS2 UCRT64 toolchain,
4. bundles the DLLs, GDK-Pixbuf loaders, GSettings schemas, icon themes,
   translations and Poppler data next to `papers.exe` ([`ci/bundle.sh`](ci/bundle.sh)),
5. opens sample PDF, DjVu, TIFF and comic documents to check that the app
   starts and stays up ([`ci/smoke-test.ps1`](ci/smoke-test.ps1)),
6. packages an Inno Setup installer and a portable zip.

Build logs, the applied diff and smoke-test screenshots of each run are
uploaded as the `logs` artifact.

### What the Windows patch changes

- Locates backends and translations relative to `papers.exe` instead of
  compile-time paths, so the install is relocatable.
- Detects document types from file signatures and extensions, because Windows
  has no shared-mime-info database, and adds extension-based filters to the
  file chooser.
- Replaces the few POSIX-only calls (`fcntl`, `O_CLOEXEC`) with Windows
  equivalents.
- Prints DjVu and TIFF by rasterizing pages through `GtkPrintOperation`; the
  upstream route uses a Unix-only print dialog.
- Starts new windows with properly quoted arguments, builds `papers.exe` as a
  GUI program (no console window), embeds the icon and version information,
  and opens the online manual for Help.

### Differences from the Linux version

- Help opens <https://help.gnome.org/papers/> in the browser; the offline
  Yelp manual is not available on Windows.
- Passwords for encrypted documents cannot be remembered in a keyring
  (the "Remember forever" option has no effect).
- Compressed `.pdf.gz` / `.pdf.bz2` / `.pdf.xz` files need the matching
  command-line decompressor on `PATH`.
- Digital signing uses Poppler's NSS certificate database; certificates have
  to be imported into it with NSS tools before "Sign Digitally" can use them.

## License

Papers is GPL-2.0-or-later, and so are the patches and scripts here.
