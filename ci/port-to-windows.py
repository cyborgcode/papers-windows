"""Apply the Windows port changes to a Papers 50.2 source tree.

Each edit replaces one exact snippet of upstream code. Snippets are matched
with whitespace-insensitive patterns (upstream C code is tab-indented; the
replacements here use spaces), and every snippet must match exactly once,
so the script fails loudly if upstream changes underneath it.

Usage: port-to-windows.py PAPERS_SOURCE_DIR
"""
import pathlib
import re
import sys

SRC = pathlib.Path(sys.argv[1])

WIN32_PRIVATE_H = """\
/* Windows portability helpers for Papers (internal). */
#pragma once

#include <glib.h>

G_BEGIN_DECLS

#ifdef G_OS_WIN32
G_GNUC_INTERNAL gchar *_pps_win32_get_install_path (const gchar *relative_path);
G_GNUC_INTERNAL const gchar *_pps_win32_sniff_mime_type (const guchar *data,
                                                        gsize length,
                                                        const gchar *name);
#endif

G_END_DECLS
"""

WIN32_HELPERS = r'''#include "pps-file-helpers.h"
#include "pps-win32-private.h"

#ifndef O_CLOEXEC
#define O_CLOEXEC 0
#endif

#ifdef G_OS_WIN32
/* Windows has no shared-mime-info database: GIO content types there are
 * file extensions and cannot be derived from file contents. Papers selects
 * backends by MIME type, so sniff the well-known magic numbers ourselves and
 * fall back to the file extension. */
gchar *
_pps_win32_get_install_path (const gchar *relative_path)
{
    g_autofree gchar *prefix = g_win32_get_package_installation_directory_of_module (NULL);

    return g_build_filename (prefix, relative_path, NULL);
}

static gboolean
has_magic_at (const guchar *data, gsize length, gsize offset, const char *magic, gsize magic_len)
{
    return length >= offset + magic_len && memcmp (data + offset, magic, magic_len) == 0;
}

static const gchar *
mime_type_for_extension (const gchar *name)
{
    static const struct {
        const char *ext;
        const char *mime;
    } map[] = {
        { ".pdf", "application/pdf" },
        { ".ai", "application/illustrator" },
        { ".djvu", "image/vnd.djvu" },
        { ".djv", "image/vnd.djvu" },
        { ".tif", "image/tiff" },
        { ".tiff", "image/tiff" },
        { ".cbz", "application/x-cbz" },
        { ".cbr", "application/x-cbr" },
        { ".cb7", "application/x-cb7" },
        { ".cbt", "application/x-cbt" },
        { ".pdf.gz", "application/x-gzpdf" },
        { ".pdf.bz2", "application/x-bzpdf" },
        { ".pdf.xz", "application/x-xzpdf" },
    };
    g_autofree gchar *lower = NULL;
    guint i;

    if (name == NULL)
        return NULL;

    lower = g_ascii_strdown (name, -1);
    /* Longest suffixes are listed last */
    for (i = G_N_ELEMENTS (map); i > 0; i--) {
        if (g_str_has_suffix (lower, map[i - 1].ext))
            return map[i - 1].mime;
    }

    return NULL;
}

const gchar *
_pps_win32_sniff_mime_type (const guchar *data,
                            gsize length,
                            const gchar *name)
{
    const gchar *by_extension = mime_type_for_extension (name);
    gsize i;

    if (data != NULL && length > 0) {
        /* PDF allows junk before the header, within the first KiB */
        for (i = 0; i + 5 <= MIN (length, 1024); i++) {
            if (memcmp (data + i, "%PDF-", 5) == 0) {
                if (g_strcmp0 (by_extension, "application/illustrator") == 0)
                    return by_extension;
                return "application/pdf";
            }
        }
        if (has_magic_at (data, length, 0, "AT&TFORM", 8))
            return "image/vnd.djvu";
        if (has_magic_at (data, length, 0, "II*\0", 4) ||
            has_magic_at (data, length, 0, "MM\0*", 4) ||
            has_magic_at (data, length, 0, "II+\0", 4) ||
            has_magic_at (data, length, 0, "MM\0+", 4))
            return "image/tiff";
        if (has_magic_at (data, length, 0, "Rar!\x1a\x07", 6))
            return "application/x-cbr";
        if (has_magic_at (data, length, 0, "7z\xbc\xaf\x27\x1c", 6))
            return "application/x-cb7";
        if (has_magic_at (data, length, 0, "PK\x03\x04", 4) ||
            has_magic_at (data, length, 0, "PK\x05\x06", 4))
            return "application/x-cbz";
        if (has_magic_at (data, length, 257, "ustar", 5))
            return "application/x-cbt";
        if (has_magic_at (data, length, 0, "\x1f\x8b", 2))
            return "application/x-gzpdf";
        if (has_magic_at (data, length, 0, "BZh", 3))
            return "application/x-bzpdf";
        if (has_magic_at (data, length, 0, "\xfd" "7zXZ\0", 6))
            return "application/x-xzpdf";
    }

    return by_extension;
}
#endif /* G_OS_WIN32 */
'''

EDITS = [
    # ---- libdocument/pps-file-helpers.c: MIME sniffing, POSIX-only fd calls
    ("libdocument/pps-file-helpers.c",
     '#include "pps-file-helpers.h"',
     WIN32_HELPERS),
    ("libdocument/pps-file-helpers.c",
     """static gchar *
get_mime_type_from_uri (const gchar *uri, GError **error)
{""",
     """static gchar *get_mime_type_from_data (const gchar *uri, GError **error);

static gchar *
get_mime_type_from_uri (const gchar *uri, GError **error)
{
#ifdef G_OS_WIN32
    return get_mime_type_from_data (uri, error);
#endif
"""),
    ("libdocument/pps-file-helpers.c",
     """retval = g_input_stream_close (G_INPUT_STREAM (input_stream), NULL, error);
if (!retval)
return NULL;""",
     """retval = g_input_stream_close (G_INPUT_STREAM (input_stream), NULL, error);
    if (!retval)
        return NULL;

#ifdef G_OS_WIN32
    {
        g_autofree gchar *basename = g_file_get_basename (file);
        const gchar *sniffed = _pps_win32_sniff_mime_type (buffer, size_read, basename);

        if (sniffed == NULL) {
            g_set_error_literal (error, G_IO_ERROR, G_IO_ERROR_FAILED,
                                 _ ("Unknown MIME Type"));
            return NULL;
        }
        return g_strdup (sniffed);
    }
#endif
"""),
    ("libdocument/pps-file-helpers.c",
     """if (lseek (fd, pos, SEEK_SET) == (off_t) -1) {
int errsv = errno;
g_set_error (error, G_IO_ERROR,
g_io_error_from_errno (errsv),
"Failed to get MIME type: %s",
g_strerror (errsv));
return NULL;
}""",
     """if (lseek (fd, pos, SEEK_SET) == (off_t) -1) {
        int errsv = errno;
        g_set_error (error, G_IO_ERROR,
                     g_io_error_from_errno (errsv),
                     "Failed to get MIME type: %s",
                     g_strerror (errsv));
        return NULL;
    }

#ifdef G_OS_WIN32
    {
        const gchar *sniffed = _pps_win32_sniff_mime_type (buffer, r, NULL);

        if (sniffed == NULL) {
            g_set_error_literal (error, G_IO_ERROR, G_IO_ERROR_FAILED,
                                 _ ("Unknown MIME Type"));
            return NULL;
        }
        return g_strdup (sniffed);
    }
#endif
"""),
    ("libdocument/pps-file-helpers.c",
     """int fd = GPOINTER_TO_INT (fd_ptr);
int flags;

flags = fcntl (fd, F_GETFD);
if (flags >= 0 && (flags & FD_CLOEXEC)) {
flags &= ~FD_CLOEXEC;
fcntl (fd, F_SETFD, flags);
}""",
     """#ifndef G_OS_WIN32
    int fd = GPOINTER_TO_INT (fd_ptr);
    int flags;

    flags = fcntl (fd, F_GETFD);
    if (flags >= 0 && (flags & FD_CLOEXEC)) {
        flags &= ~FD_CLOEXEC;
        fcntl (fd, F_SETFD, flags);
    }
#endif"""),

    # ---- PDF backend: poppler_document_new_from_fd() is Unix-only. Papers
    # opens documents by URI; loading from a descriptor stays unsupported.
    ("libdocument/backend/pdf/pps-poppler.c",
     """static gboolean
pdf_document_load_fd (PpsDocument *document,""",
     """#ifndef G_OS_WIN32
static gboolean
pdf_document_load_fd (PpsDocument *document,"""),
    ("libdocument/backend/pdf/pps-poppler.c",
     """g_rw_lock_writer_unlock (&self->rwlock);

return success;
}

static int
pdf_document_get_n_pages (PpsDocument *document)""",
     """g_rw_lock_writer_unlock (&self->rwlock);

    return success;
}
#endif /* !G_OS_WIN32 */

static int
pdf_document_get_n_pages (PpsDocument *document)"""),
    ("libdocument/backend/pdf/pps-poppler.c",
     "pps_document_class->load_fd = pdf_document_load_fd;",
     """#ifndef G_OS_WIN32
    pps_document_class->load_fd = pdf_document_load_fd;
#endif"""),

    # ---- TIFF and comics backends: GLib file names are UTF-8 on Windows, the
    # narrow-character open functions expect the ANSI code page.
    ("libdocument/backend/tiff/tiff-document.c",
     """tiff = TIFFOpen (filename, "r");""",
     """#ifdef G_OS_WIN32
    {
        g_autofree gunichar2 *wide_filename = g_utf8_to_utf16 (filename, -1, NULL, NULL, NULL);

        tiff = wide_filename ? TIFFOpenW ((const wchar_t *) wide_filename, "r") : NULL;
    }
#else
    tiff = TIFFOpen (filename, "r");
#endif"""),
    ("libdocument/backend/comics/pps-archive.c",
     "r = archive_read_open_filename (archive->libar, path, BUFFER_SIZE);",
     """#ifdef G_OS_WIN32
    {
        g_autofree gunichar2 *wide_path = g_utf8_to_utf16 (path, -1, NULL, NULL, NULL);

        r = wide_path ? archive_read_open_filename_w (archive->libar, (const wchar_t *) wide_path, BUFFER_SIZE)
                      : ARCHIVE_FATAL;
    }
#else
    r = archive_read_open_filename (archive->libar, path, BUFFER_SIZE);
#endif"""),

    # ---- Paper sizes in the user's measurement system, as with
    # nl_langinfo(_NL_MEASUREMENT_MEASUREMENT) on Linux
    ("libdocument/pps-document-info.c",
     """#ifdef HAVE__NL_MEASUREMENT_MEASUREMENT
#include <langinfo.h>
#endif""",
     """#ifdef HAVE__NL_MEASUREMENT_MEASUREMENT
#include <langinfo.h>
#endif
#ifdef G_OS_WIN32
#include <windows.h>
#endif"""),
    ("libdocument/pps-document-info.c",
     """#ifdef HAVE__NL_MEASUREMENT_MEASUREMENT
gchar *imperial = NULL;""",
     """#ifdef G_OS_WIN32
    {
        wchar_t measure[2];

        /* LOCALE_IMEASURE: "0" metric, "1" U.S. */
        if (GetLocaleInfoEx (LOCALE_NAME_USER_DEFAULT, LOCALE_IMEASURE, measure, G_N_ELEMENTS (measure)) > 0)
            return measure[0] == L'1' ? GTK_UNIT_INCH : GTK_UNIT_MM;
    }
#endif

#ifdef HAVE__NL_MEASUREMENT_MEASUREMENT
    gchar *imperial = NULL;"""),

    # ---- Per-document metadata (last page, zoom, layout, sidebar): GIO's
    # "metadata::" attributes need GVfs, which Windows lacks. Keep one key
    # file per document under the user data directory instead.
    ("libview/context/pps-metadata.c",
     """#define PPS_METADATA_NAMESPACE "metadata::papers\"""",
     """#define PPS_METADATA_NAMESPACE "metadata::papers"

#ifdef G_OS_WIN32
#include <errno.h>

#define PPS_METADATA_GROUP "Metadata"

static gchar *
pps_metadata_win32_path (GFile *file)
{
    g_autofree gchar *uri = g_file_get_uri (file);
    g_autofree gchar *hash = g_compute_checksum_for_string (G_CHECKSUM_SHA256, uri, -1);
    g_autofree gchar *name = g_strconcat (hash, ".ini", NULL);

    return g_build_filename (g_get_user_data_dir (), "papers", "metadata", name, NULL);
}

static void
pps_metadata_win32_load (PpsMetadata *metadata)
{
    g_autofree gchar *path = pps_metadata_win32_path (metadata->file);
    g_autoptr (GKeyFile) keyfile = g_key_file_new ();
    g_auto (GStrv) keys = NULL;
    gsize i;

    if (!g_key_file_load_from_file (keyfile, path, G_KEY_FILE_NONE, NULL))
        return;

    keys = g_key_file_get_keys (keyfile, PPS_METADATA_GROUP, NULL, NULL);
    for (i = 0; keys != NULL && keys[i] != NULL; i++) {
        gchar *value = g_key_file_get_string (keyfile, PPS_METADATA_GROUP, keys[i], NULL);

        if (value != NULL)
            g_hash_table_insert (metadata->items, g_strdup (keys[i]), value);
    }
}

static void
pps_metadata_win32_save (PpsMetadata *metadata)
{
    g_autofree gchar *path = pps_metadata_win32_path (metadata->file);
    g_autofree gchar *dir = g_path_get_dirname (path);
    g_autofree gchar *uri = g_file_get_uri (metadata->file);
    g_autoptr (GKeyFile) keyfile = g_key_file_new ();
    g_autoptr (GError) error = NULL;
    GHashTableIter iter;
    gpointer key, value;

    g_key_file_set_string (keyfile, "Document", "uri", uri);
    g_hash_table_iter_init (&iter, metadata->items);
    while (g_hash_table_iter_next (&iter, &key, &value)) {
        if (value != NULL)
            g_key_file_set_string (keyfile, PPS_METADATA_GROUP, key, value);
    }

    if (g_mkdir_with_parents (dir, 0700) != 0) {
        g_warning ("Failed to create %s: %s", dir, g_strerror (errno));
        return;
    }
    if (!g_key_file_save_to_file (keyfile, path, &error))
        g_warning ("Failed to save document metadata: %s", error->message);
}
#endif /* G_OS_WIN32 */"""),
    ("libview/context/pps-metadata.c",
     """static void
pps_metadata_load (PpsMetadata *metadata)
{""",
     """static void
pps_metadata_load (PpsMetadata *metadata)
{
#ifdef G_OS_WIN32
    pps_metadata_win32_load (metadata);
#else"""),
    ("libview/context/pps-metadata.c",
     """g_strfreev (attrs);
g_object_unref (info);
}

PpsMetadata *""",
     """g_strfreev (attrs);
    g_object_unref (info);
#endif /* G_OS_WIN32 */
}

PpsMetadata *"""),
    ("libview/context/pps-metadata.c",
     """g_hash_table_insert (metadata->items, g_strdup (key), g_strdup (value));
if (!metadata->file)
return TRUE;""",
     """g_hash_table_insert (metadata->items, g_strdup (key), g_strdup (value));
    if (!metadata->file)
        return TRUE;

#ifdef G_OS_WIN32
    pps_metadata_win32_save (metadata);
    return TRUE;
#endif"""),

    # ---- libdocument/pps-document.c: no fcntl() on Windows
    ("libdocument/pps-document.c",
     """if (fstat (fd, &statbuf) == -1 ||
(fd_flags = fcntl (fd, F_GETFL, NULL)) == -1) {""",
     """#ifdef G_OS_WIN32
    /* No fcntl(): descriptors here come from our own read-only opens. */
    fd_flags = O_RDONLY;
    if (fstat (fd, &statbuf) == -1) {
#else
    if (fstat (fd, &statbuf) == -1 ||
        (fd_flags = fcntl (fd, F_GETFL, NULL)) == -1) {
#endif"""),
    ("libdocument/pps-document.c",
     "switch (fd_flags & O_ACCMODE) {",
     """#ifndef O_ACCMODE
#define O_ACCMODE (O_RDONLY | O_WRONLY | O_RDWR)
#endif
    switch (fd_flags & O_ACCMODE) {"""),

    # ---- libdocument/pps-document-factory.c: backend lookup and file filters
    ("libdocument/pps-document-factory.c",
     '#include "pps-file-helpers.h"',
     '#include "pps-file-helpers.h"\n#include "pps-win32-private.h"'),
    ("libdocument/pps-document-factory.c",
     """for (i = 0; mime_types[i] != NULL; ++i)
if (g_content_type_is_mime_type (content_type, mime_types[i]))
return info;""",
     """for (i = 0; mime_types[i] != NULL; ++i) {
            /* Content types are not MIME types on Windows */
            if (g_ascii_strcasecmp (mime_type, mime_types[i]) == 0)
                return info;
            if (content_type != NULL &&
                g_content_type_is_mime_type (content_type, mime_types[i]))
                return info;
        }"""),
    ("libdocument/pps-document-factory.c",
     """if (!pps_backends_dir) {
pps_backends_dir = g_strdup (PPS_BACKENDSDIR);
}""",
     """if (!pps_backends_dir) {
#ifdef G_OS_WIN32
        pps_backends_dir = _pps_win32_get_install_path (PPS_BACKENDS_RELDIR);
#else
        pps_backends_dir = g_strdup (PPS_BACKENDSDIR);
#endif
    }"""),
    ("libdocument/pps-document-factory.c",
     """for (i = 0; mime_types[i] != NULL; ++i)
gtk_file_filter_add_mime_type (filter, mime_types[i]);
}""",
     """for (i = 0; mime_types[i] != NULL; ++i) {
#ifdef G_OS_WIN32
        /* GTK cannot map these MIME types to file patterns on Windows */
        static const struct {
            const char *mime;
            const char *suffixes;
        } suffix_map[] = {
            { "application/pdf", "pdf" },
            { "application/illustrator", "ai" },
            { "image/vnd.djvu", "djvu;djv" },
            { "image/tiff", "tif;tiff" },
            { "application/x-cbz", "cbz" },
            { "application/x-cbr", "cbr" },
            { "application/x-cb7", "cb7" },
            { "application/x-cbt", "cbt" },
        };
        guint j;

        for (j = 0; j < G_N_ELEMENTS (suffix_map); j++) {
            if (g_ascii_strcasecmp (mime_types[i], suffix_map[j].mime) == 0) {
                g_auto (GStrv) suffixes = g_strsplit (suffix_map[j].suffixes, ";", -1);
                guint k;

                for (k = 0; suffixes[k] != NULL; k++)
                    gtk_file_filter_add_suffix (filter, suffixes[k]);
            }
        }
#else
        gtk_file_filter_add_mime_type (filter, mime_types[i]);
#endif
    }
}"""),

    # ---- libdocument/pps-init.c: relocatable translations
    ("libdocument/pps-init.c",
     '#include "pps-init.h"',
     '#include "pps-init.h"\n#include "pps-win32-private.h"'),
    ("libdocument/pps-init.c",
     "bindtextdomain (GETTEXT_PACKAGE, PPS_LOCALEDIR);",
     """#ifdef G_OS_WIN32
    {
        g_autofree gchar *localedir = _pps_win32_get_install_path (PPS_LOCALE_RELDIR);

        bindtextdomain (GETTEXT_PACKAGE, localedir);
    }
#else
    bindtextdomain (GETTEXT_PACKAGE, PPS_LOCALEDIR);
#endif"""),
    ("libdocument/pps-init.c",
     "gdk_pixbuf_init_modules (EXTRA_GDK_PIXBUF_LOADERS_DIR, NULL);",
     """#ifndef G_OS_WIN32
    gdk_pixbuf_init_modules (EXTRA_GDK_PIXBUF_LOADERS_DIR, NULL);
#endif"""),

    # ---- meson: relative directories for relocation, optional appstreamcli
    ("meson.build",
     "config_h.set_quoted('PPS_LOCALEDIR', pps_localedir)",
     """config_h.set_quoted('PPS_LOCALEDIR', pps_localedir)
config_h.set_quoted('PPS_LOCALE_RELDIR', get_option('localedir'))
config_h.set_quoted('PPS_BACKENDS_RELDIR', join_paths(get_option('libdir'), pps_backends_subdir))"""),
    ("meson.build",
     "appstreamcli = find_program('appstreamcli', required: true)",
     """# Only used to validate metainfo in tests; not packaged everywhere (Windows)
appstreamcli = find_program('appstreamcli', required: get_option('tests'))"""),

    # ---- backends: GModule on Windows resolves "<dir>/<module>.dll" (no lib prefix)
    ("libdocument/backend/meson.build",
     """shared_module(
backend_name,
sources: backend_sources,""",
     """shared_module(
    backend_name,
    name_prefix: host_machine.system() == 'windows' ? '' : [],
    sources: backend_sources,"""),

    # ---- libview/pps-jobs.c: dup() and a raster print fallback
    ("libview/pps-jobs.c",
     "new_fd = fcntl (fd, F_DUPFD_CLOEXEC, 3);",
     """#ifdef G_OS_WIN32
    new_fd = dup (fd);
#else
    new_fd = fcntl (fd, F_DUPFD_CLOEXEC, 3);
#endif"""),
    ("libview/pps-jobs.c",
     """pps_page = pps_document_get_page (pps_job_get_document (job), priv->page);
pps_document_print_print_page (PPS_DOCUMENT_PRINT (pps_job_get_document (job)),
pps_page, priv->cr);
g_object_unref (pps_page);""",
     """pps_page = pps_document_get_page (pps_job_get_document (job), priv->page);
    if (PPS_IS_DOCUMENT_PRINT (pps_job_get_document (job))) {
        pps_document_print_print_page (PPS_DOCUMENT_PRINT (pps_job_get_document (job)),
                                       pps_page, priv->cr);
    } else {
        /* Raster fallback for backends that only export to PostScript
         * (DjVu, TIFF): that path needs a Unix print dialog, which is not
         * available on every platform. */
        PpsDocument *document = pps_job_get_document (job);
        const gdouble print_scale = 300.0 / 72.0;
        PpsRenderContext *rc = pps_render_context_new (pps_page, 0, print_scale, PPS_RENDER_ANNOTS_PRINT_ALL);
        cairo_surface_t *surface = pps_document_render (document, rc);

        if (surface != NULL) {
            cairo_save (priv->cr);
            cairo_scale (priv->cr, 1.0 / print_scale, 1.0 / print_scale);
            cairo_set_source_surface (priv->cr, surface, 0, 0);
            cairo_paint (priv->cr);
            cairo_restore (priv->cr);
            cairo_surface_destroy (surface);
        }
        g_object_unref (rc);
    }
    g_object_unref (pps_page);"""),

    # ---- libview/pps-print-operation.c: use GtkPrintOperation for exporters too
    ("libview/pps-print-operation.c",
     """if (PPS_IS_DOCUMENT_PRINT (document) && g_strcmp0 (env, "export") != 0) {
type = PPS_TYPE_PRINT_OPERATION_PRINT;
} else if (PPS_IS_FILE_EXPORTER (document)) {""",
     """if (PPS_IS_DOCUMENT_PRINT (document) && g_strcmp0 (env, "export") != 0) {
        type = PPS_TYPE_PRINT_OPERATION_PRINT;
#ifdef G_OS_WIN32
    } else if (PPS_IS_FILE_EXPORTER (document)) {
        /* GtkPrintDialog-based export is Unix-only; print rasterized pages */
        type = PPS_TYPE_PRINT_OPERATION_PRINT;
#endif
    } else if (PPS_IS_FILE_EXPORTER (document)) {"""),

    # ---- shell build: the binary has an .exe suffix on Windows
    ("shell/src/meson.build",
     "cargo = find_program('cargo', required: true)",
     """cargo = find_program('cargo', required: true)
exe_suffix = host_machine.system() == 'windows' ? '.exe' : ''"""),
    ("shell/src/meson.build",
     """output: meson.project_name(),
command: [
'cp',
meson.current_build_dir() / rust_target / meson.project_name(),
'@OUTPUT@',
],""",
     """output: meson.project_name() + exe_suffix,
  command: [
    'cp',
    meson.current_build_dir() / rust_target / meson.project_name() + exe_suffix,
    '@OUTPUT@',
  ],"""),

    # ---- shell/src/main.rs: GUI subsystem, relocatable translations
    ("shell/src/main.rs",
     "mod config {",
     """// No console window for the GUI application on Windows
#![cfg_attr(windows, windows_subsystem = "windows")]

mod config {"""),
    ("shell/src/main.rs",
     """gettextrs::bindtextdomain(GETTEXT_PACKAGE, PPS_LOCALEDIR)""",
     """gettextrs::bindtextdomain(GETTEXT_PACKAGE, locale_dir())"""),
    ("shell/src/main.rs",
     "fn main() -> glib::ExitCode {",
     """/// The installation is relocatable on Windows: translations live in
/// `<prefix>/share/locale` next to `<prefix>/bin/papers.exe`.
#[cfg(windows)]
fn locale_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent()?.parent().map(|prefix| prefix.join("share").join("locale")))
        .unwrap_or_else(|| std::path::PathBuf::from(PPS_LOCALEDIR))
}

#[cfg(not(windows))]
fn locale_dir() -> &'static str {
    PPS_LOCALEDIR
}

fn main() -> glib::ExitCode {"""),

    # ---- shell/src/application.rs: online help, process spawning
    ("shell/src/application.rs",
     """if let Err(e) =
gio::AppInfo::launch_default_for_uri_future("help:papers", context.as_ref())
.await""",
     """// Yelp and the help: URI scheme are not available on Windows
                let uri = if cfg!(windows) {
                    "https://help.gnome.org/papers/"
                } else {
                    "help:papers"
                };
                if let Err(e) =
                    gio::AppInfo::launch_default_for_uri_future(uri, context.as_ref()).await"""),
    ("shell/src/application.rs",
     """pub fn spawn(file: Option<&gio::File>, dest: Option<&LinkDest>, mode: Option<WindowRunMode>) {
let mut cmd = String::new();""",
     """/// Windows: paths contain spaces and backslashes, which the shell-style
/// command line below cannot represent, so pass arguments directly.
#[cfg(windows)]
pub fn spawn(file: Option<&gio::File>, dest: Option<&LinkDest>, mode: Option<WindowRunMode>) {
    let exe = match env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            glib::g_critical!("", "Failed to find current executable: {}", e);
            return;
        }
    };
    let mut cmd = std::process::Command::new(exe);

    if let Some(dest) = dest {
        match dest.dest_type() {
            LinkDestType::PageLabel => {
                cmd.arg(format!("--page-label={}", dest.page_label().unwrap_or_default()));
            }
            LinkDestType::Page
            | LinkDestType::Xyz
            | LinkDestType::Fit
            | LinkDestType::Fith
            | LinkDestType::Fitv
            | LinkDestType::Fitr => {
                cmd.arg(format!("--page-index={}", dest.page() + 1));
            }
            LinkDestType::Named => {
                cmd.arg(format!("--named-dest={}", dest.named_dest().unwrap_or_default()));
            }
            _ => (),
        }
    }

    match mode {
        Some(WindowRunMode::Fullscreen) => {
            cmd.arg("-f");
        }
        Some(WindowRunMode::Presentation) => {
            cmd.arg("-s");
        }
        _ => (),
    }

    if let Some(file) = file {
        cmd.arg(file.uri().as_str());
    }

    if let Err(e) = cmd.spawn() {
        glib::g_printerr!("Error launching papers: {}\\n", e);
    }
}

#[cfg(not(windows))]
pub fn spawn(file: Option<&gio::File>, dest: Option<&LinkDest>, mode: Option<WindowRunMode>) {
    let mut cmd = String::new();"""),
]


def flexible_pattern(snippet):
    """Regex matching the snippet with any amount of whitespace between tokens."""
    parts = re.split(r"\s+", snippet.strip())
    return re.compile(r"\s*".join(re.escape(p) for p in parts))


def main():
    failed = False
    (SRC / "libdocument/pps-win32-private.h").write_text(WIN32_PRIVATE_H)
    for path, old, new in EDITS:
        file = SRC / path
        text = file.read_text(encoding="utf-8")
        matches = list(flexible_pattern(old).finditer(text))
        if len(matches) != 1:
            print(f"error: {path}: expected 1 match, found {len(matches)} for:\n{old}\n", file=sys.stderr)
            failed = True
            continue
        m = matches[0]
        text = text[: m.start()] + new + text[m.end():]
        file.write_text(text, encoding="utf-8", newline="\n")
        print(f"patched {path}")
    if failed:
        sys.exit(1)


main()
