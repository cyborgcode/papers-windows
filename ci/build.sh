#!/usr/bin/env bash
# Build GNOME Papers for Windows inside an MSYS2 UCRT64 shell.
#
# Fetches the upstream release, applies the Windows port patches, builds the
# exempi shim, then configures, compiles and installs Papers into _work/install.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
PAPERS_VERSION=${PAPERS_VERSION:-50.2}
WORK=$ROOT/_work
LOGDIR=$WORK/logs
SRC=$WORK/papers
INSTALL=$WORK/install
SHIM=$WORK/exempi-shim

mkdir -p "$WORK" "$LOGDIR"
exec > >(tee -a "$LOGDIR/build.log") 2>&1

winpath() { cygpath -m "$1"; }

echo "::group::Fetch Papers $PAPERS_VERSION"
# The build directory may already exist (restored Rust cache), so fetch the
# release into it instead of cloning.
if [ ! -d "$SRC/.git" ]; then
	mkdir -p "$SRC"
	git -C "$SRC" init -q
	git -C "$SRC" remote add origin https://gitlab.gnome.org/GNOME/papers.git
	git -C "$SRC" fetch -q --depth 1 origin "refs/tags/$PAPERS_VERSION:refs/tags/$PAPERS_VERSION"
	git -C "$SRC" checkout -q "$PAPERS_VERSION"
fi
git -C "$SRC" reset --hard -q
git -C "$SRC" clean -fdq -e build
python "$ROOT/ci/port-to-windows.py" "$SRC"
git -C "$SRC" add -A
git -C "$SRC" diff --cached > "$LOGDIR/windows-port.patch"
echo "::endgroup::"

echo "::group::Build exempi shim"
rm -rf "$SHIM"
mkdir -p "$SHIM/include/exempi-2.0/exempi" "$SHIM/lib/pkgconfig" "$SHIM/obj"
XML_CFLAGS=$(pkg-config --cflags libxml-2.0)
XML_LIBS=$(pkg-config --libs libxml-2.0)
# shellcheck disable=SC2086
gcc -std=c11 -O2 -Wall -Wextra -Werror $XML_CFLAGS -I"$ROOT/exempi-shim" \
	-c "$ROOT/exempi-shim/xmp.c" -o "$SHIM/obj/xmp.o"
ar rcs "$SHIM/lib/libexempi.a" "$SHIM/obj/xmp.o"
cp "$ROOT/exempi-shim/xmp.h" "$ROOT/exempi-shim/xmpconsts.h" "$SHIM/include/exempi-2.0/exempi/"
sed "s|@PREFIX@|$(winpath "$SHIM")|" "$ROOT/exempi-shim/exempi-2.0.pc.in" > "$SHIM/lib/pkgconfig/exempi-2.0.pc"
# shellcheck disable=SC2086
gcc -std=c11 -O2 -Wall $XML_CFLAGS -I"$ROOT/exempi-shim" "$ROOT/exempi-shim/test-xmp.c" \
	"$SHIM/lib/libexempi.a" $XML_LIBS -o "$SHIM/obj/test-xmp.exe"
"$SHIM/obj/test-xmp.exe"
# pkgconf here is a native Windows program: use a Windows path
export PKG_CONFIG_PATH="$(winpath "$SHIM/lib/pkgconfig")"
pkg-config --modversion exempi-2.0
echo "::endgroup::"

echo "::group::Application icon and version resource"
ICONDIR=$WORK/icon
mkdir -p "$ICONDIR"
pngs=()
for size in 16 20 24 32 40 48 64 96 128 256; do
	rsvg-convert -w $size -h $size "$SRC/data/icons/scalable/apps/org.gnome.Papers.svg" -o "$ICONDIR/papers-$size.png"
	pngs+=("$ICONDIR/papers-$size.png")
done
python "$ROOT/ci/make-icon.py" "$ICONDIR/papers.ico" "${pngs[@]}"
major=${PAPERS_VERSION%%.*}
minor=${PAPERS_VERSION#*.}
minor=${minor%%.*}
cp "$ROOT/packaging/papers.rc" "$ICONDIR/papers.rc"
cat > "$ICONDIR/papers-version.h" <<EOF
#define PAPERS_VERSION_MAJOR $major
#define PAPERS_VERSION_MINOR $minor
#define PAPERS_VERSION_STRING "$PAPERS_VERSION"
EOF
(cd "$ICONDIR" && windres papers.rc -O coff -o papers-res.o)
# Linked into every Rust artifact of the build; only papers.exe shows it.
export RUSTFLAGS="-C link-arg=$(winpath "$ICONDIR/papers-res.o")"
echo "::endgroup::"

echo "::group::Configure"
cd "$SRC"
if [ ! -f build/build.ninja ]; then
	meson setup build \
		--prefix="$(winpath "$INSTALL")" \
		--buildtype=release \
		-Dnautilus=false \
		-Dpreviewer=false \
		-Dthumbnailer=false \
		-Dtests=false \
		-Dfile_tests=false \
		-Ddocumentation=false \
		-Duser_doc=false \
		-Dintrospection=disabled \
		-Dsysprof=disabled \
		-Dkeyring=disabled \
		-Dgtk_unix_print=disabled \
		-Dspell_check=enabled \
		-Dcomics=enabled \
		-Ddjvu=enabled \
		-Dtiff=enabled \
		-Dpdf=enabled
else
	meson setup --reconfigure build
fi
echo "::endgroup::"

echo "::group::Compile"
# Keep going after failures so one CI run reports as many errors as possible
ninja -C build -k 0
echo "::endgroup::"

echo "::group::Install"
rm -rf "$INSTALL"
meson install -C build
echo "::endgroup::"

ls -la "$INSTALL/bin" "$INSTALL/lib/papers"/*/backends
