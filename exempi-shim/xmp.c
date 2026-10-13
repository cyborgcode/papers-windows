/*
 * Minimal exempi-compatible XMP reader for Windows builds of Papers.
 * See xmp.h for scope. Follows the XMP data model for RDF/XML serialization:
 * properties are attributes or child elements of any rdf:Description, arrays
 * are rdf:Seq/rdf:Bag/rdf:Alt containers of rdf:li items, and language
 * alternatives carry xml:lang on their items.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include <stdlib.h>
#include <string.h>

#include <libxml/parser.h>
#include <libxml/tree.h>

#include "xmp.h"
#include "xmpconsts.h"

struct _Xmp {
	xmlDocPtr doc;
};

struct _XmpString {
	char *str;
};

/* A property found in the packet: either an attribute value on an
 * rdf:Description (simple, unqualified) or a property element. */
typedef struct {
	xmlChar *attr_value;
	xmlNodePtr element;
} XmpFound;

static bool
node_is (xmlNodePtr node, const char *ns, const char *local)
{
	return node != NULL && node->type == XML_ELEMENT_NODE &&
	       node->ns != NULL && node->ns->href != NULL &&
	       strcmp ((const char *) node->ns->href, ns) == 0 &&
	       strcmp ((const char *) node->name, local) == 0;
}

static void
string_set (XmpStringPtr s, const char *value)
{
	size_t len;

	if (s == NULL)
		return;
	if (value == NULL)
		value = "";
	len = strlen (value);
	free (s->str);
	s->str = malloc (len + 1);
	if (s->str != NULL)
		memcpy (s->str, value, len + 1);
}

static void
string_take (XmpStringPtr s, xmlChar *value)
{
	string_set (s, (const char *) value);
	xmlFree (value);
}

static bool
find_property (xmlNodePtr node, const char *schema, const char *name, XmpFound *found)
{
	for (; node != NULL; node = node->next) {
		if (node->type != XML_ELEMENT_NODE)
			continue;

		if (node_is (node, NS_RDF, "Description")) {
			xmlChar *value = xmlGetNsProp (node, (const xmlChar *) name,
			                               (const xmlChar *) schema);
			xmlNodePtr child;

			if (value != NULL) {
				found->attr_value = value;
				found->element = NULL;
				return true;
			}
			for (child = node->children; child != NULL; child = child->next) {
				if (node_is (child, schema, name)) {
					found->attr_value = NULL;
					found->element = child;
					return true;
				}
			}
		}

		if (find_property (node->children, schema, name, found))
			return true;
	}

	return false;
}

static bool
lookup (XmpPtr xmp, const char *schema, const char *name, XmpFound *found)
{
	if (xmp == NULL || xmp->doc == NULL || schema == NULL || name == NULL)
		return false;

	return find_property (xmlDocGetRootElement (xmp->doc), schema, name, found);
}

static xmlNodePtr
first_element_child (xmlNodePtr node)
{
	xmlNodePtr child;

	for (child = node->children; child != NULL; child = child->next)
		if (child->type == XML_ELEMENT_NODE)
			return child;

	return NULL;
}

/* Returns the rdf:Seq/rdf:Bag/rdf:Alt container of a property element. */
static xmlNodePtr
array_container (xmlNodePtr element, uint32_t *bits)
{
	xmlNodePtr child = first_element_child (element);

	if (node_is (child, NS_RDF, "Seq")) {
		*bits = XMP_PROP_VALUE_IS_ARRAY | XMP_PROP_ARRAY_IS_ORDERED;
		return child;
	}
	if (node_is (child, NS_RDF, "Bag")) {
		*bits = XMP_PROP_VALUE_IS_ARRAY;
		return child;
	}
	if (node_is (child, NS_RDF, "Alt")) {
		*bits = XMP_PROP_VALUE_IS_ARRAY | XMP_PROP_ARRAY_IS_ORDERED | XMP_PROP_ARRAY_IS_ALT;
		return child;
	}

	return NULL;
}

/* Value of a simple element (property element or rdf:li). */
static xmlChar *
element_value (xmlNodePtr element, uint32_t *bits)
{
	xmlChar *resource = xmlGetNsProp (element, (const xmlChar *) "resource",
	                                  (const xmlChar *) NS_RDF);

	if (resource != NULL) {
		*bits |= XMP_PROP_VALUE_IS_URI;
		return resource;
	}

	return xmlNodeGetContent (element);
}

bool
xmp_init (void)
{
	xmlInitParser ();
	return true;
}

void
xmp_terminate (void)
{
}

XmpPtr
xmp_new_empty (void)
{
	return calloc (1, sizeof (struct _Xmp));
}

XmpPtr
xmp_new (const char *buffer, size_t len)
{
	XmpPtr xmp;
	xmlDocPtr doc;

	if (buffer == NULL)
		return NULL;

	/* Packets are often padded with NULs */
	while (len > 0 && buffer[len - 1] == '\0')
		len--;
	if (len == 0 || len > INT32_MAX)
		return NULL;

	doc = xmlReadMemory (buffer, (int) len, "xmp.xml", NULL,
	                     XML_PARSE_NONET | XML_PARSE_NOERROR | XML_PARSE_NOWARNING);
	if (doc == NULL)
		return NULL;

	if (xmlDocGetRootElement (doc) == NULL) {
		xmlFreeDoc (doc);
		return NULL;
	}

	xmp = xmp_new_empty ();
	if (xmp == NULL) {
		xmlFreeDoc (doc);
		return NULL;
	}
	xmp->doc = doc;

	return xmp;
}

bool
xmp_free (XmpPtr xmp)
{
	if (xmp == NULL)
		return false;
	if (xmp->doc != NULL)
		xmlFreeDoc (xmp->doc);
	free (xmp);
	return true;
}

XmpStringPtr
xmp_string_new (void)
{
	return calloc (1, sizeof (struct _XmpString));
}

void
xmp_string_free (XmpStringPtr s)
{
	if (s == NULL)
		return;
	free (s->str);
	free (s);
}

const char *
xmp_string_cstr (XmpStringPtr s)
{
	return (s != NULL && s->str != NULL) ? s->str : "";
}

size_t
xmp_string_len (XmpStringPtr s)
{
	return strlen (xmp_string_cstr (s));
}

bool
xmp_has_property (XmpPtr xmp, const char *schema, const char *name)
{
	XmpFound found;

	if (!lookup (xmp, schema, name, &found))
		return false;
	xmlFree (found.attr_value);
	return true;
}

bool
xmp_get_property (XmpPtr xmp, const char *schema, const char *name,
                  XmpStringPtr property, uint32_t *propsBits)
{
	XmpFound found;
	uint32_t bits = 0;

	if (!lookup (xmp, schema, name, &found))
		return false;

	if (found.attr_value != NULL) {
		string_take (property, found.attr_value);
	} else if (array_container (found.element, &bits) != NULL) {
		/* Like exempi: arrays have no value of their own */
		string_set (property, "");
	} else if (node_is (first_element_child (found.element), NS_RDF, "Description")) {
		bits |= XMP_PROP_VALUE_IS_STRUCT;
		string_set (property, "");
	} else {
		string_take (property, element_value (found.element, &bits));
	}

	if (propsBits != NULL)
		*propsBits = bits;

	return true;
}

bool
xmp_get_property_bool (XmpPtr xmp, const char *schema, const char *name,
                       bool *property, uint32_t *propsBits)
{
	XmpStringPtr value = xmp_string_new ();
	bool ok = false;

	if (value != NULL && xmp_get_property (xmp, schema, name, value, propsBits)) {
		const char *v = xmp_string_cstr (value);

		/* XMP Boolean values are "True" and "False" */
		while (*v == ' ' || *v == '\t' || *v == '\n' || *v == '\r')
			v++;
		if (strncmp (v, "True", 4) == 0 || strncmp (v, "true", 4) == 0 || strcmp (v, "1") == 0) {
			if (property != NULL)
				*property = true;
			ok = true;
		} else if (strncmp (v, "False", 5) == 0 || strncmp (v, "false", 5) == 0 || strcmp (v, "0") == 0) {
			if (property != NULL)
				*property = false;
			ok = true;
		}
	}

	xmp_string_free (value);
	return ok;
}

bool
xmp_get_array_item (XmpPtr xmp, const char *schema, const char *name,
                    int32_t index, XmpStringPtr property, uint32_t *propsBits)
{
	XmpFound found;
	xmlNodePtr container, item;
	uint32_t bits = 0;
	int32_t i = 0;

	if (index < 1 || !lookup (xmp, schema, name, &found))
		return false;

	/* Be lenient with writers that store a plain value instead of an array */
	if (found.attr_value != NULL) {
		if (index != 1) {
			xmlFree (found.attr_value);
			return false;
		}
		string_take (property, found.attr_value);
		if (propsBits != NULL)
			*propsBits = 0;
		return true;
	}

	container = array_container (found.element, &bits);
	if (container == NULL) {
		if (index != 1)
			return false;
		bits = 0;
		string_take (property, element_value (found.element, &bits));
		if (propsBits != NULL)
			*propsBits = bits;
		return true;
	}

	for (item = container->children; item != NULL; item = item->next) {
		if (!node_is (item, NS_RDF, "li"))
			continue;
		if (++i == index) {
			uint32_t item_bits = 0;

			string_take (property, element_value (item, &item_bits));
			if (propsBits != NULL)
				*propsBits = item_bits;
			return true;
		}
	}

	return false;
}

static xmlChar *
item_lang (xmlNodePtr item)
{
	return xmlGetNsProp (item, (const xmlChar *) "lang", XML_XML_NAMESPACE);
}

static bool
lang_is_generic_match (const char *lang, const char *generic)
{
	size_t n = strlen (generic);

	return xmlStrncasecmp ((const xmlChar *) lang, (const xmlChar *) generic, (int) n) == 0 &&
	       (lang[n] == '\0' || lang[n] == '-');
}

bool
xmp_get_localized_text (XmpPtr xmp, const char *schema, const char *name,
                        const char *genericLang, const char *specificLang,
                        XmpStringPtr actualLang, XmpStringPtr itemValue,
                        uint32_t *propBits)
{
	XmpFound found;
	xmlNodePtr container, item;
	xmlNodePtr specific = NULL, generic = NULL, x_default = NULL, first = NULL;
	xmlNodePtr chosen;
	uint32_t bits = 0;

	if (!lookup (xmp, schema, name, &found))
		return false;

	if (found.attr_value != NULL) {
		string_take (itemValue, found.attr_value);
		string_set (actualLang, "x-default");
		if (propBits != NULL)
			*propBits = 0;
		return true;
	}

	container = array_container (found.element, &bits);
	if (container == NULL) {
		bits = 0;
		string_take (itemValue, element_value (found.element, &bits));
		string_set (actualLang, "x-default");
		if (propBits != NULL)
			*propBits = bits;
		return true;
	}

	/* Same preference order as the XMP SDK's ChooseLocalizedText */
	for (item = container->children; item != NULL; item = item->next) {
		xmlChar *lang;

		if (!node_is (item, NS_RDF, "li"))
			continue;
		if (first == NULL)
			first = item;

		lang = item_lang (item);
		if (lang != NULL) {
			if (specific == NULL && specificLang != NULL && *specificLang &&
			    xmlStrcasecmp (lang, (const xmlChar *) specificLang) == 0)
				specific = item;
			if (generic == NULL && genericLang != NULL && *genericLang &&
			    lang_is_generic_match ((const char *) lang, genericLang))
				generic = item;
			if (x_default == NULL && xmlStrcasecmp (lang, (const xmlChar *) "x-default") == 0)
				x_default = item;
			xmlFree (lang);
		}
	}

	chosen = specific ? specific : generic ? generic : x_default ? x_default : first;
	if (chosen == NULL)
		return false;

	{
		xmlChar *lang = item_lang (chosen);
		uint32_t item_bits = 0;

		string_set (actualLang, lang ? (const char *) lang : "");
		xmlFree (lang);
		string_take (itemValue, element_value (chosen, &item_bits));
		if (propBits != NULL)
			*propBits = item_bits | XMP_PROP_HAS_LANG;
	}

	return true;
}
