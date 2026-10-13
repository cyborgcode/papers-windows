/* Tests for the exempi shim, using the same calls as Papers' pps-xmp.c.
 * SPDX-License-Identifier: GPL-2.0-or-later */

#include <stdio.h>
#include <string.h>

#include "xmp.h"
#include "xmpconsts.h"

static int failures;

#define CHECK(cond)                                                     \
	do {                                                                \
		if (!(cond)) {                                                  \
			fprintf (stderr, "%s:%d: FAILED: %s\n", __FILE__, __LINE__, #cond); \
			failures++;                                                 \
		}                                                               \
	} while (0)

static const char packet[] =
    "<?xpacket begin=\"\xef\xbb\xbf\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n"
    "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n"
    " <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n"
    "  <rdf:Description rdf:about=\"\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"\n"
    "     pdf:Producer=\"LibreOffice 7.6\"/>\n"
    "  <rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n"
    "   <xmp:CreateDate>2024-03-01T10:20:30+01:00</xmp:CreateDate>\n"
    "   <xmp:CreatorTool>Writer</xmp:CreatorTool>\n"
    "  </rdf:Description>\n"
    "  <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n"
    "   <dc:creator><rdf:Seq><rdf:li>Ada Lovelace</rdf:li><rdf:li>Alan Turing</rdf:li></rdf:Seq></dc:creator>\n"
    "   <dc:subject><rdf:Bag><rdf:li>math</rdf:li><rdf:li>engines</rdf:li></rdf:Bag></dc:subject>\n"
    "   <dc:title><rdf:Alt>\n"
    "     <rdf:li xml:lang=\"x-default\">Default Title</rdf:li>\n"
    "     <rdf:li xml:lang=\"fr-FR\">Titre</rdf:li>\n"
    "     <rdf:li xml:lang=\"de\">Titel</rdf:li>\n"
    "   </rdf:Alt></dc:title>\n"
    "  </rdf:Description>\n"
    "  <rdf:Description rdf:about=\"\" xmlns:xmpRights=\"http://ns.adobe.com/xap/1.0/rights/\"\n"
    "      xmlns:cc=\"http://creativecommons.org/ns#\" xmpRights:Marked=\"True\">\n"
    "   <xmpRights:UsageTerms><rdf:Alt><rdf:li xml:lang=\"x-default\">CC BY 4.0</rdf:li></rdf:Alt></xmpRights:UsageTerms>\n"
    "   <xmpRights:WebStatement>https://example.org/license</xmpRights:WebStatement>\n"
    "   <cc:license rdf:resource=\"https://creativecommons.org/licenses/by/4.0/\"/>\n"
    "  </rdf:Description>\n"
    " </rdf:RDF>\n"
    "</x:xmpmeta>\n"
    "<?xpacket end=\"w\"?>";

int
main (void)
{
	XmpPtr xmp;
	XmpStringPtr s = xmp_string_new ();
	bool marked = false;
	uint32_t bits = 0;

	xmp_init ();

	CHECK (xmp_new ("not xml", 7) == NULL);
	CHECK (xmp_new ("", 0) == NULL);

	/* Papers passes the packet length, possibly including NUL padding */
	xmp = xmp_new (packet, sizeof (packet));
	CHECK (xmp != NULL);

	CHECK (xmp_get_property (xmp, NS_PDF, "Producer", s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "LibreOffice 7.6") == 0);
	CHECK (xmp_get_property (xmp, NS_XAP, "CreateDate", s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "2024-03-01T10:20:30+01:00") == 0);
	CHECK (xmp_get_property (xmp, NS_XAP, "CreatorTool", s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "Writer") == 0);
	CHECK (!xmp_get_property (xmp, NS_XAP, "ModifyDate", s, NULL));

	CHECK (xmp_get_property (xmp, NS_DC, "creator", s, &bits));
	CHECK (bits & XMP_PROP_VALUE_IS_ARRAY);
	CHECK (xmp_get_array_item (xmp, NS_DC, "creator", 1, s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "Ada Lovelace") == 0);
	CHECK (xmp_get_array_item (xmp, NS_DC, "creator", 2, s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "Alan Turing") == 0);
	CHECK (!xmp_get_array_item (xmp, NS_DC, "creator", 3, s, NULL));
	CHECK (xmp_get_array_item (xmp, NS_DC, "subject", 2, s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "engines") == 0);

	CHECK (xmp_get_localized_text (xmp, NS_DC, "title", "fr", "fr-FR", NULL, s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "Titre") == 0);
	CHECK (xmp_get_localized_text (xmp, NS_DC, "title", "de", "de-AT", NULL, s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "Titel") == 0);
	CHECK (xmp_get_localized_text (xmp, NS_DC, "title", "en", "en-us", NULL, s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "Default Title") == 0);
	CHECK (xmp_get_localized_text (xmp, NS_DC, "title", NULL, "", NULL, s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "Default Title") == 0);

	CHECK (xmp_get_property_bool (xmp, NS_XAP_RIGHTS, "Marked", &marked, NULL));
	CHECK (marked);
	CHECK (xmp_get_localized_text (xmp, NS_XAP_RIGHTS, "UsageTerms", "en", "en-us", NULL, s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "CC BY 4.0") == 0);
	CHECK (xmp_get_property (xmp, NS_CC, "license", s, &bits));
	CHECK (strcmp (xmp_string_cstr (s), "https://creativecommons.org/licenses/by/4.0/") == 0);
	CHECK (bits & XMP_PROP_VALUE_IS_URI);
	CHECK (xmp_get_property (xmp, NS_XAP_RIGHTS, "WebStatement", s, NULL));
	CHECK (strcmp (xmp_string_cstr (s), "https://example.org/license") == 0);
	CHECK (!xmp_get_property_bool (xmp, NS_XAP_RIGHTS, "Missing", &marked, NULL));

	CHECK (xmp_free (xmp));
	xmp_string_free (s);

	if (failures == 0)
		printf ("all exempi shim tests passed\n");
	return failures != 0;
}
