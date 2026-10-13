#!/usr/bin/env bash
# Turn the Papers install tree into a self-contained, relocatable Windows
# application folder (_work/dist/Papers) with every runtime file it needs
# from MSYS2: DLLs, GDK-Pixbuf loaders, GIO modules, GSettings schemas,
# icon themes, translations, Poppler data and fontconfig configuration.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$ROOT/_work
INSTALL=$WORK/install
DIST=$WORK/dist/Papers
MSYS_PREFIX=${MINGW_PREFIX:-/ucrt64}
exec > >(tee -a "$WORK/logs/bundle.log") 2>&1

rm -rf "$DIST"
mkdir -p "$DIST"
cp -r "$INSTALL"/. "$DIST"/
# Development and Linux-desktop-only files
rm -rf "$DIST/include" "$DIST/lib/pkgconfig" "$DIST/share/gir-1.0" "$DIST/share/applications" \
	"$DIST/share/dbus-1" "$DIST/share/thumbnailers" "$DIST/share/man" "$DIST/share/help"
find "$DIST/lib" -name '*.dll.a' -delete

# GLib spawn helpers (used by g_spawn_* on Windows)
cp "$MSYS_PREFIX"/bin/gspawn-win64-helper*.exe "$DIST/bin/"

# GDK-Pixbuf loaders (SVG app icons, WebP/AVIF images in comic books)
PIXBUF_DIR=lib/gdk-pixbuf-2.0/2.10.0
mkdir -p "$DIST/$PIXBUF_DIR/loaders"
cp "$MSYS_PREFIX/$PIXBUF_DIR"/loaders/*.dll "$DIST/$PIXBUF_DIR/loaders/"

# GIO modules (TLS for opening remote documents)
mkdir -p "$DIST/lib/gio/modules"
cp "$MSYS_PREFIX"/lib/gio/modules/*.dll "$DIST/lib/gio/modules/" 2>/dev/null || true

# GSettings schemas: Papers' own plus GTK's
SCHEMAS=share/glib-2.0/schemas
mkdir -p "$DIST/$SCHEMAS"
cp "$MSYS_PREFIX/$SCHEMAS"/org.gtk.gtk4.*.gschema.xml "$DIST/$SCHEMAS/"
cp "$MSYS_PREFIX/$SCHEMAS"/gschema.dtd "$DIST/$SCHEMAS/" 2>/dev/null || true
glib-compile-schemas "$DIST/$SCHEMAS"

# Icon themes
mkdir -p "$DIST/share/icons"
cp -r "$MSYS_PREFIX/share/icons/Adwaita" "$DIST/share/icons/"
cp -rn "$MSYS_PREFIX/share/icons/hicolor/." "$DIST/share/icons/hicolor/"
for theme in Adwaita hicolor; do
	gtk4-update-icon-cache -q -f -t "$DIST/share/icons/$theme"
done

# Translations of the libraries whose strings appear in the UI
for langdir in "$DIST"/share/locale/*/; do
	lang=$(basename "$langdir")
	for domain in gtk40 gtk40-properties libadwaita glib20 libspelling; do
		src="$MSYS_PREFIX/share/locale/$lang/LC_MESSAGES/$domain.mo"
		[ -f "$src" ] && cp "$src" "$langdir/LC_MESSAGES/"
	done
done

# Poppler encoding data (CJK documents), fontconfig defaults, spelling dictionaries
cp -r "$MSYS_PREFIX/share/poppler" "$DIST/share/"
mkdir -p "$DIST/etc"
cp -r "$MSYS_PREFIX/etc/fonts" "$DIST/etc/"
if [ -d "$MSYS_PREFIX/share/hunspell" ]; then
	cp -r "$MSYS_PREFIX/share/hunspell" "$DIST/share/"
fi

# Modules loaded at run time, invisible to ldd:
# - NSS crypto modules, used by Poppler to verify and create digital signatures
for module in softokn3 freebl3 nssckbi; do
	cp "$MSYS_PREFIX/bin/$module.dll" "$DIST/bin/"
	if [ -f "$MSYS_PREFIX/bin/$module.chk" ]; then cp "$MSYS_PREFIX/bin/$module.chk" "$DIST/bin/"; fi
done
# - Enchant spell-checking providers: Windows' own spell checker and Hunspell
mkdir -p "$DIST/lib/enchant-2" "$DIST/share/enchant-2"
for provider in enchant_winspell enchant_hunspell; do
	if [ -f "$MSYS_PREFIX/lib/enchant-2/$provider.dll" ]; then
		cp "$MSYS_PREFIX/lib/enchant-2/$provider.dll" "$DIST/lib/enchant-2/"
	fi
done
cp "$MSYS_PREFIX/share/enchant-2/enchant.ordering" "$DIST/share/enchant-2/" 2>/dev/null || true

# Copy every MSYS2 DLL that the executables, backends and modules link
# against, transitively, next to papers.exe.
collect_dlls() {
	local changed=1
	while [ $changed -eq 1 ]; do
		changed=0
		while IFS= read -r -d '' binary; do
			while read -r dep; do
				local name
				name=$(basename "$dep")
				if [ ! -f "$DIST/bin/$name" ]; then
					cp "$dep" "$DIST/bin/"
					changed=1
				fi
			done < <(ldd "$binary" 2>/dev/null | awk -v p="$MSYS_PREFIX/bin/" 'index(tolower($3), tolower(p)) == 1 { print $3 }')
		done < <(find "$DIST" \( -iname '*.exe' -o -iname '*.dll' \) -print0)
	done
}
collect_dlls

# Pixbuf loader cache with paths relative to the bundle
cp "$MSYS_PREFIX/bin/gdk-pixbuf-query-loaders.exe" "$DIST/bin/"
(cd "$DIST/bin" && ./gdk-pixbuf-query-loaders.exe --update-cache)
rm "$DIST/bin/gdk-pixbuf-query-loaders.exe"
if [ -d "$DIST/lib/gio/modules" ]; then
	cp "$MSYS_PREFIX/bin/gio-querymodules.exe" "$DIST/bin/"
	(cd "$DIST/bin" && ./gio-querymodules.exe ../lib/gio/modules)
	rm "$DIST/bin/gio-querymodules.exe"
fi

# Licenses
mkdir -p "$DIST/share/licenses/papers"
cp "$WORK/papers/COPYING" "$DIST/share/licenses/papers/COPYING.txt"
cp "$ROOT/README.md" "$DIST/share/licenses/papers/README-windows.md"

echo "Bundle size: $(du -sh "$DIST" | cut -f1)"
echo "Files: $(find "$DIST" -type f | wc -l)"
ls "$DIST/bin"
