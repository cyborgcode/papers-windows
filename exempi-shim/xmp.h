/*
 * Minimal exempi-compatible XMP reader for Windows builds of Papers.
 *
 * Exempi (the XMP SDK wrapper) is not packaged for MSYS2. Papers only reads
 * XMP metadata, so this implements the read-only subset of the exempi 2.x
 * API that it calls, on top of libxml2.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#ifndef EXEMPI_SHIM_XMP_H
#define EXEMPI_SHIM_XMP_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct _Xmp *XmpPtr;
typedef struct _XmpString *XmpStringPtr;

/* Property option bits reported through propsBits (subset of exempi's). */
enum {
	XMP_PROP_VALUE_IS_URI = 0x00000002UL,
	XMP_PROP_HAS_LANG = 0x00000040UL,
	XMP_PROP_VALUE_IS_STRUCT = 0x00000100UL,
	XMP_PROP_VALUE_IS_ARRAY = 0x00000200UL,
	XMP_PROP_ARRAY_IS_ORDERED = 0x00000400UL,
	XMP_PROP_ARRAY_IS_ALT = 0x00000800UL,
	XMP_PROP_ARRAY_IS_ALTTEXT = 0x00001000UL,
};

bool xmp_init (void);
void xmp_terminate (void);

XmpPtr xmp_new_empty (void);
XmpPtr xmp_new (const char *buffer, size_t len);
bool xmp_free (XmpPtr xmp);

XmpStringPtr xmp_string_new (void);
void xmp_string_free (XmpStringPtr s);
const char *xmp_string_cstr (XmpStringPtr s);
size_t xmp_string_len (XmpStringPtr s);

bool xmp_get_property (XmpPtr xmp, const char *schema, const char *name,
                       XmpStringPtr property, uint32_t *propsBits);
bool xmp_get_property_bool (XmpPtr xmp, const char *schema, const char *name,
                            bool *property, uint32_t *propsBits);
bool xmp_get_array_item (XmpPtr xmp, const char *schema, const char *name,
                         int32_t index, XmpStringPtr property, uint32_t *propsBits);
bool xmp_get_localized_text (XmpPtr xmp, const char *schema, const char *name,
                             const char *genericLang, const char *specificLang,
                             XmpStringPtr actualLang, XmpStringPtr itemValue,
                             uint32_t *propBits);
bool xmp_has_property (XmpPtr xmp, const char *schema, const char *name);

#ifdef __cplusplus
}
#endif

#endif /* EXEMPI_SHIM_XMP_H */
