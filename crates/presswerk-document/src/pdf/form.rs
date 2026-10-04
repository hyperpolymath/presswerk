// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Form provenance — decide whether the values in a PDF form were written by
// software or by a person.
//
// WHY THIS EXISTS (issue #118, ruling D189)
// -----------------------------------------
// Upstream form-fillers such as `blocky-writer` emit an ordinary PDF byte
// stream: `/V`, `/DV` and `/AS` entries are written and `NeedAppearances` is
// set, but nothing in the file says *who* wrote the values. Presswerk is
// usually the next thing that touches those bytes — it prints them — so the
// print path classifies the document at the boundary and records the result.
//
// The classification is deliberately conservative and evidence-based:
//
//   Explicit      an upstream provenance marker is present (`/FillOrigin` in
//                 the document information dictionary, or a `fillOrigin`
//                 attribute in an XMP packet)
//   Strong        `/Producer` or `/Creator` names a tool known to fill forms
//                 programmatically
//   Probable      a characteristic combination of structural signals — values
//                 without appearance streams, `NeedAppearances` set, values
//                 written alongside `/DV` defaults
//
// Only a *positive* machine determination is ever acted upon; everything else
// prints as normal. See `docs/ecosystem/FORM-PROVENANCE.adoc`.

use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId};
use presswerk_core::types::{
    DocumentType, FormOrigin, FormProvenance, FormProvenancePolicy, ProvenanceConfidence,
};
use tracing::{debug, instrument};

/// Documents larger than this are not inspected.
///
/// Inspection happens inline on the print path, so the budget keeps a huge
/// document from stalling the IPP accept loop. Documents over the budget are
/// reported as "not inspected" rather than guessed at.
pub const MAX_INSPECT_BYTES: usize = 8 * 1024 * 1024;

/// Hard cap on field-tree nodes visited, so a malformed or hostile field tree
/// cannot make inspection unbounded.
const MAX_FIELD_NODES: u32 = 100_000;

/// Hard cap on field-tree nesting depth.
const MAX_FIELD_DEPTH: u32 = 32;

/// How far down a field's own subtree to look for an appearance stream before
/// concluding the value has none.
const APPEARANCE_SEARCH_DEPTH: u32 = 3;

/// Substrings that identify a tool which writes form values programmatically.
///
/// Matched case-insensitively against `/Producer` and `/Creator`. Document
/// generators that merely *create* blank PDFs are deliberately absent: a blank
/// form produced by such a tool is still an empty form.
const KNOWN_MACHINE_FILLERS: &[&str] = &[
    "blocky-writer",
    "blocky_writer",
    "blockywriter",
    "fill_blocks",
    "lopdf",
    "itext",
    "openpdf",
    "pdf-lib",
    "pypdf",
    "pdftk",
    "pdfbox",
    "aspose",
    "pdfsharp",
    "docusign",
    "adobe livecycle",
    "livecycle",
];

/// Attribute name scanned for in raw bytes / XMP packets.
const MARKER_ATTRIBUTE: &[u8] = b"fillorigin";

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Classify a document arriving on the print path.
///
/// This is the entry point both print paths (local printing and the embedded
/// IPP server) call. It never fails: anything that stops inspection — a
/// non-PDF, an oversized document, a parse error, a disabled policy — is
/// reported inside the returned value with `inspected == false`.
#[instrument(skip_all, fields(bytes = data.len(), policy = %policy.as_token()))]
pub fn classify_for_print(
    data: &[u8],
    document_type: DocumentType,
    policy: FormProvenancePolicy,
) -> FormProvenance {
    if !policy.inspects() {
        debug!("form provenance inspection disabled by policy");
        return FormProvenance::not_inspected("form provenance inspection is disabled");
    }
    if document_type != DocumentType::Pdf {
        return FormProvenance::not_inspected("document is not a PDF");
    }
    classify_pdf(data)
}

/// Classify the form provenance of raw PDF bytes.
///
/// Never fails; see [`classify_for_print`].
#[instrument(skip_all, fields(bytes = data.len()))]
pub fn classify_pdf(data: &[u8]) -> FormProvenance {
    if data.is_empty() {
        return FormProvenance::not_inspected("document is empty");
    }
    if data.len() > MAX_INSPECT_BYTES {
        return FormProvenance::not_inspected(&format!(
            "document is {} bytes, above the {} byte inspection budget",
            data.len(),
            MAX_INSPECT_BYTES
        ));
    }

    let document = match Document::load_mem(data) {
        Ok(document) => document,
        Err(err) => {
            debug!(error = %err, "PDF parse failed during provenance inspection");
            return FormProvenance::not_inspected(&format!("PDF parse failed: {err}"));
        }
    };

    classify_document(&document, data)
}

/// Classify an already-parsed document.
///
/// `raw` is the original byte stream and is used only to find unstructured
/// provenance markers (for example inside an XMP packet); pass an empty slice
/// to rely on the structured `/Info` marker alone.
#[instrument(skip_all, fields(raw_bytes = raw.len()))]
pub fn classify_document(document: &Document, raw: &[u8]) -> FormProvenance {
    let mut provenance = inspected_blank();

    let info = document_info(document);
    provenance.producer = info.producer.clone();
    provenance.creator = info.creator.clone();

    // -- Explicit marker ------------------------------------------------------
    let mut marker = structured_marker(&info);
    if marker.is_none() {
        marker = scan_for_marker(raw);
    }
    if marker.is_none() {
        for packet in xmp_packets(document) {
            if let Some(found) = scan_for_marker(&packet) {
                marker = Some(found);
                break;
            }
        }
    }
    if let Some(found) = marker
        && mentions_machine(&found)
    {
        provenance.marker = Some(found.clone());
        provenance
            .evidence
            .push(format!("explicit provenance marker present: {found}"));
    }
    let explicit = provenance.marker.is_some();

    // -- Known tool -----------------------------------------------------------
    let known_tool = known_filler(provenance.producer.as_deref(), provenance.creator.as_deref());
    if let Some(tool) = known_tool {
        provenance.evidence.push(format!(
            "document producer/creator names '{tool}', a known form-filling tool"
        ));
    }

    // -- Structure ------------------------------------------------------------
    let catalog = match document.catalog() {
        Ok(catalog) => catalog,
        Err(err) => {
            provenance.origin = FormOrigin::NotAForm;
            provenance.confidence = ProvenanceConfidence::Speculative;
            provenance
                .evidence
                .push(format!("document has no readable catalog ({err})"));
            return finish(provenance);
        }
    };

    let acroform = match catalog
        .get(b"AcroForm")
        .ok()
        .and_then(|obj| resolve_dict(document, obj))
    {
        Some(acroform) => acroform,
        None => {
            provenance.origin = FormOrigin::NotAForm;
            provenance.confidence = ProvenanceConfidence::Strong;
            provenance
                .evidence
                .push("document catalog has no /AcroForm entry".to_string());
            return finish(provenance);
        }
    };

    provenance.is_form = true;
    provenance.need_appearances =
        matches!(acroform.get(b"NeedAppearances"), Ok(Object::Boolean(true)));

    let scan = scan_fields(document, acroform);
    provenance.field_count = scan.field_count;
    provenance.filled_field_count = scan.filled;
    provenance.filled_with_default = scan.filled_with_default;
    provenance.values_without_appearance = scan.without_appearance;

    // -- Determination --------------------------------------------------------
    if explicit {
        // An explicit marker outranks every structural signal, including an
        // apparently empty form: the writer is telling us it filled the form.
        provenance.origin = FormOrigin::Machine;
        provenance.confidence = ProvenanceConfidence::Explicit;
    } else if provenance.filled_field_count == 0 {
        provenance.origin = FormOrigin::Empty;
        provenance.confidence = ProvenanceConfidence::Strong;
        provenance.evidence.push(format!(
            "/AcroForm declares {} field(s), none of which carries a value",
            provenance.field_count
        ));
    } else if known_tool.is_some() {
        provenance.origin = FormOrigin::Machine;
        provenance.confidence = ProvenanceConfidence::Strong;
    } else if provenance.need_appearances {
        provenance.origin = FormOrigin::Machine;
        provenance.confidence = ProvenanceConfidence::Probable;
        provenance.evidence.push(format!(
            "{} of {} field(s) carry values and /NeedAppearances is set — values were written without appearances",
            provenance.filled_field_count, provenance.field_count
        ));
    } else if provenance.values_without_appearance > 0 {
        provenance.origin = FormOrigin::Machine;
        provenance.confidence = ProvenanceConfidence::Probable;
        provenance.evidence.push(format!(
            "{} field value(s) have no /AP appearance stream",
            provenance.values_without_appearance
        ));
    } else if provenance.filled_with_default > 0 {
        provenance.origin = FormOrigin::Machine;
        provenance.confidence = ProvenanceConfidence::Speculative;
        provenance.evidence.push(format!(
            "{} field value(s) were written alongside a /DV default",
            provenance.filled_with_default
        ));
    } else {
        provenance.origin = FormOrigin::Human;
        provenance.confidence = ProvenanceConfidence::Probable;
        provenance.evidence.push(format!(
            "{} of {} field(s) carry values, each with an appearance stream and no machine-fill signal",
            provenance.filled_field_count, provenance.field_count
        ));
    }

    finish(provenance)
}

// ---------------------------------------------------------------------------
// Determination bookkeeping
// ---------------------------------------------------------------------------

/// A provenance value for a document that *was* inspected, before any evidence
/// has been gathered.
fn inspected_blank() -> FormProvenance {
    FormProvenance {
        origin: FormOrigin::Unknown,
        confidence: ProvenanceConfidence::None,
        inspected: true,
        is_form: false,
        field_count: 0,
        filled_field_count: 0,
        values_without_appearance: 0,
        filled_with_default: 0,
        need_appearances: false,
        producer: None,
        creator: None,
        marker: None,
        evidence: Vec::new(),
    }
}

/// Log the determination and hand it back.
fn finish(provenance: FormProvenance) -> FormProvenance {
    debug!(
        origin = %provenance.origin,
        confidence = %provenance.confidence,
        fields = provenance.field_count,
        filled = provenance.filled_field_count,
        "form provenance determined"
    );
    provenance
}

// ---------------------------------------------------------------------------
// Object graph helpers
// ---------------------------------------------------------------------------

/// Resolve an object that is expected to be a dictionary (possibly indirect).
fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(&stream.dict),
        Object::Reference(id) => document.get_object(*id).ok().and_then(|resolved| {
            match resolved {
                Object::Dictionary(dict) => Some(dict),
                Object::Stream(stream) => Some(&stream.dict),
                _ => None,
            }
        }),
        _ => None,
    }
}

/// Resolve an object that is expected to be an array (possibly indirect).
fn resolve_array<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Vec<Object>> {
    match object {
        Object::Array(items) => Some(items),
        Object::Reference(id) => document
            .get_object(*id)
            .ok()
            .and_then(|resolved| match resolved {
                Object::Array(items) => Some(items),
                _ => None,
            }),
        _ => None,
    }
}

/// Whether a dictionary carries a non-empty value under `key`.
///
/// `Off` is treated as *no* value in either representation: for check boxes and
/// radio buttons it is the explicitly unselected state, not an entry.
fn has_value(dict: &Dictionary, key: &[u8]) -> bool {
    match dict.get(key) {
        Ok(Object::Null) | Err(_) => false,
        Ok(Object::String(bytes, _)) => !is_blank_or_off(bytes),
        Ok(Object::Name(name)) => !is_blank_or_off(name),
        Ok(Object::Array(items)) => !items.is_empty(),
        Ok(_) => true,
    }
}

/// Whether a PDF name or string represents "no value".
fn is_blank_or_off(bytes: &[u8]) -> bool {
    bytes.is_empty() || bytes.eq_ignore_ascii_case(b"Off")
}

// ---------------------------------------------------------------------------
// Field tree
// ---------------------------------------------------------------------------

/// What a walk of the AcroForm field tree counted.
struct FieldScan {
    field_count: u32,
    filled: u32,
    filled_with_default: u32,
    without_appearance: u32,
    visited: u32,
}

/// Walk the AcroForm `/Fields` tree and count fields, values, and values that
/// have no appearance stream anywhere in their own subtree.
fn scan_fields(document: &Document, acroform: &Dictionary) -> FieldScan {
    let mut scan = FieldScan {
        field_count: 0,
        filled: 0,
        filled_with_default: 0,
        without_appearance: 0,
        visited: 0,
    };
    let mut seen = HashSet::new();

    if let Ok(fields) = acroform.get(b"Fields") {
        walk_fields(document, fields, 0, &mut seen, &mut scan);
    }

    scan
}

/// Recursive field-tree walk with cycle and size guards.
fn walk_fields(
    document: &Document,
    node: &Object,
    depth: u32,
    seen: &mut HashSet<ObjectId>,
    scan: &mut FieldScan,
) {
    if depth > MAX_FIELD_DEPTH || scan.visited >= MAX_FIELD_NODES {
        return;
    }

    match node {
        Object::Reference(id) => {
            if !seen.insert(*id) {
                return;
            }
            scan.visited += 1;
            if let Ok(resolved) = document.get_object(*id) {
                walk_fields(document, resolved, depth + 1, seen, scan);
            }
        }
        Object::Array(items) => {
            for item in items {
                walk_fields(document, item, depth + 1, seen, scan);
            }
        }
        Object::Dictionary(dict) => {
            scan.visited += 1;

            // A node with a partial field name (/T) is a field; widget-only
            // annotations inherit their name from the parent field.
            if dict.get(b"T").is_ok() {
                scan.field_count += 1;
                if has_value(dict, b"V") {
                    scan.filled += 1;
                    if has_value(dict, b"DV") {
                        scan.filled_with_default += 1;
                    }
                    if !subtree_has_appearance(document, dict, 0) {
                        scan.without_appearance += 1;
                    }
                }
            }

            if let Ok(kids) = dict.get(b"Kids") {
                walk_fields(document, kids, depth + 1, seen, scan);
            }
        }
        Object::Stream(stream) => {
            scan.visited += 1;
            if let Ok(kids) = stream.dict.get(b"Kids") {
                walk_fields(document, kids, depth + 1, seen, scan);
            }
        }
        _ => {}
    }
}

/// Whether a field or anything in its subtree carries an `/AP` appearance
/// dictionary.
///
/// Widget annotations are frequently kept apart from the field that owns the
/// value, so the value and its appearance do not always live in the same
/// dictionary.
fn subtree_has_appearance(document: &Document, dict: &Dictionary, depth: u32) -> bool {
    if dict.get(b"AP").is_ok() {
        return true;
    }
    if depth >= APPEARANCE_SEARCH_DEPTH {
        return false;
    }

    for key in [b"Kids".as_slice(), b"Annots".as_slice()] {
        if let Some(items) = dict.get(key).ok().and_then(|obj| resolve_array(document, obj)) {
            for item in items {
                if let Some(item_dict) = resolve_dict(document, item)
                    && subtree_has_appearance(document, item_dict, depth + 1)
                {
                    return true;
                }
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

/// Interesting entries from the document information dictionary.
struct DocInfo {
    producer: Option<String>,
    creator: Option<String>,
    /// Non-standard `Fill*` entries — the explicit marker convention.
    fill_entries: Vec<(String, String)>,
}

/// Read `/Producer`, `/Creator` and any `Fill*` marker entries from the
/// document information dictionary.
fn document_info(document: &Document) -> DocInfo {
    let mut info = DocInfo {
        producer: None,
        creator: None,
        fill_entries: Vec::new(),
    };

    let Some(dict) = document
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|obj| resolve_dict(document, obj))
    else {
        return info;
    };

    for (key, value) in dict.iter() {
        let Object::String(bytes, _) = value else {
            continue;
        };
        let key = String::from_utf8_lossy(key).to_string();
        let text = String::from_utf8_lossy(bytes).trim().to_string();
        match key.as_str() {
            "Producer" => info.producer = Some(text),
            "Creator" => info.creator = Some(text),
            other if other.starts_with("Fill") => info.fill_entries.push((other.into(), text)),
            _ => {}
        }
    }

    info
}

/// Build the explicit marker from `Fill*` information dictionary entries.
fn structured_marker(info: &DocInfo) -> Option<String> {
    let mut origin = None;
    let mut tool = None;
    let mut time = None;

    for (key, value) in &info.fill_entries {
        match key.as_str() {
            "FillOrigin" => origin = Some(value.as_str()),
            "FillTool" => tool = Some(value.as_str()),
            "FillTime" => time = Some(value.as_str()),
            _ => {}
        }
    }

    let origin = origin?;
    if !mentions_machine(origin) {
        return None;
    }

    Some(describe_marker(origin, tool, time))
}

/// Render a marker's parts as one stable, readable string.
fn describe_marker(origin: &str, tool: Option<&str>, time: Option<&str>) -> String {
    let mut parts = vec![format!("FillOrigin={origin}")];
    if let Some(tool) = tool.filter(|t| !t.trim().is_empty()) {
        parts.push(format!("FillTool={tool}"));
    }
    if let Some(time) = time.filter(|t| !t.trim().is_empty()) {
        parts.push(format!("FillTime={time}"));
    }
    parts.join("; ")
}

/// Whether a marker value asserts machine filling.
fn mentions_machine(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    lowered.contains("machine") || lowered.contains("software") || lowered.contains("automatic")
}

/// Return the first tool from [`KNOWN_MACHINE_FILLERS`] named by the producer
/// or creator string.
fn known_filler(producer: Option<&str>, creator: Option<&str>) -> Option<&'static str> {
    for candidate in [producer, creator].into_iter().flatten() {
        let lowered = candidate.to_ascii_lowercase();
        for tool in KNOWN_MACHINE_FILLERS {
            if lowered.contains(tool) {
                return Some(tool);
            }
        }
    }
    None
}

/// Contents of the document's XMP metadata streams.
///
/// XMP packets are usually written uncompressed, so the raw stream content is
/// normally enough. Compressed packets are not decoded; the structured
/// `/Info` marker is the authoritative location.
fn xmp_packets(document: &Document) -> Vec<Vec<u8>> {
    let mut packets = Vec::new();

    let Ok(catalog) = document.catalog() else {
        return packets;
    };
    let Some(metadata) = catalog.get(b"Metadata").ok() else {
        return packets;
    };
    let metadata = match metadata {
        Object::Reference(id) => match document.get_object(*id) {
            Ok(resolved) => resolved,
            Err(_) => return packets,
        },
        other => other,
    };
    if let Object::Stream(stream) = metadata {
        packets.push(stream.content.clone());
    }

    packets
}

/// Scan raw bytes for a `fillOrigin` attribute or element and return its value.
///
/// Accepts the attribute form (`fillOrigin="machine"`), single quotes, an
/// unquoted attribute value, and the RDF element-text form
/// (`<bw:fillOrigin>machine</bw:fillOrigin>`).
fn scan_for_marker(bytes: &[u8]) -> Option<String> {
    let start = find_ci(bytes, MARKER_ATTRIBUTE)?;
    let mut rest = trim_ascii_start(&bytes[start + MARKER_ATTRIBUTE.len()..]);
    if rest.is_empty() {
        return None;
    }

    if rest[0] == b'=' {
        // Attribute form.
        rest = trim_ascii_start(&rest[1..]);
        if rest.is_empty() {
            return None;
        }
        return match rest[0] {
            b'"' | b'\'' => {
                let quote = rest[0];
                rest = &rest[1..];
                let end = rest.iter().position(|&byte| byte == quote)?;
                decode_marker_value(&rest[..end])
            }
            _ => {
                let end = rest
                    .iter()
                    .position(|&byte| byte.is_ascii_whitespace() || byte == b'>')?;
                decode_marker_value(&rest[..end])
            }
        };
    }

    // Element form: the value is the element text.
    if rest[0] != b'>' {
        return None;
    }
    rest = &rest[1..];
    let end = rest.iter().position(|&byte| byte == b'<')?;
    decode_marker_value(&rest[..end])
}

/// Decode a marker value, rejecting empty ones.
fn decode_marker_value(bytes: &[u8]) -> Option<String> {
    let value = String::from_utf8_lossy(bytes).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Case-insensitive substring search over bytes.
fn find_ci(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

/// Drop leading ASCII whitespace.
fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    &bytes[index..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::StringFormat;

    /// A field specification for the test PDF builder.
    struct TestField {
        name: &'static str,
        value: Option<&'static str>,
        default: Option<&'static str>,
        appearance: bool,
    }

    impl TestField {
        /// A field a human filled in a viewer: value plus appearance stream.
        const fn human(name: &'static str, value: &'static str) -> Self {
            Self {
                name,
                value: Some(value),
                default: None,
                appearance: true,
            }
        }

        /// A field a program filled: value, default, and no appearance stream.
        const fn machine(name: &'static str, value: &'static str) -> Self {
            Self {
                name,
                value: Some(value),
                default: Some(value),
                appearance: false,
            }
        }

        /// A field with no value at all.
        const fn blank(name: &'static str) -> Self {
            Self {
                name,
                value: None,
                default: None,
                appearance: false,
            }
        }
    }

    /// Build a real, serialisable PDF carrying an AcroForm.
    fn build_pdf(
        fields: Vec<TestField>,
        need_appearances: bool,
        info_entries: Vec<(&'static str, &'static str)>,
    ) -> Vec<u8> {
        let mut document = Document::with_version("1.7");

        // Minimal one-page page tree so the document is well formed.
        let content_id = document.add_object(Object::Stream(lopdf::Stream::new(
            Dictionary::new(),
            Vec::new(),
        )));
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(595),
                Object::Integer(842),
            ]),
        );
        page.set("Contents", Object::Reference(content_id));
        let page_id = document.add_object(Object::Dictionary(page));

        let mut pages = Dictionary::new();
        pages.set("Type", Object::Name(b"Pages".to_vec()));
        pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        pages.set("Count", Object::Integer(1));
        let pages_id = document.add_object(Object::Dictionary(pages));

        if let Ok(Object::Dictionary(page_dict)) = document.get_object_mut(page_id) {
            page_dict.set("Parent", Object::Reference(pages_id));
        }

        // Field dictionaries.
        let mut field_refs = Vec::new();
        for field in &fields {
            let mut dict = Dictionary::new();
            dict.set(
                "T",
                Object::String(field.name.as_bytes().to_vec(), StringFormat::Literal),
            );
            dict.set("FT", Object::Name(b"Tx".to_vec()));
            if let Some(value) = field.value {
                dict.set(
                    "V",
                    Object::String(value.as_bytes().to_vec(), StringFormat::Literal),
                );
            }
            if let Some(default) = field.default {
                dict.set(
                    "DV",
                    Object::String(default.as_bytes().to_vec(), StringFormat::Literal),
                );
            }
            if field.appearance {
                let mut appearance = Dictionary::new();
                appearance.set("N", Object::Null);
                dict.set("AP", Object::Dictionary(appearance));
            }
            field_refs.push(Object::Reference(
                document.add_object(Object::Dictionary(dict)),
            ));
        }

        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        if !fields.is_empty() || need_appearances {
            let mut acroform = Dictionary::new();
            acroform.set("Fields", Object::Array(field_refs));
            if need_appearances {
                acroform.set("NeedAppearances", Object::Boolean(true));
            }
            catalog.set("AcroForm", Object::Dictionary(acroform));
        }
        let catalog_id = document.add_object(Object::Dictionary(catalog));
        document.trailer.set("Root", Object::Reference(catalog_id));

        if !info_entries.is_empty() {
            let mut info = Dictionary::new();
            for (key, value) in info_entries {
                info.set(
                    key.as_bytes().to_vec(),
                    Object::String(value.as_bytes().to_vec(), StringFormat::Literal),
                );
            }
            let info_id = document.add_object(Object::Dictionary(info));
            document.trailer.set("Info", Object::Reference(info_id));
        }

        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("serialise test PDF");
        bytes
    }

    #[test]
    fn non_pdf_bytes_are_not_inspected() {
        let provenance = classify_pdf(b"this is not a PDF at all");
        assert!(!provenance.inspected);
        assert_eq!(provenance.origin, FormOrigin::Unknown);
        assert!(
            provenance.evidence[0].contains("PDF parse failed"),
            "{:?}",
            provenance.evidence
        );
    }

    #[test]
    fn empty_input_is_not_inspected() {
        let provenance = classify_pdf(b"");
        assert!(!provenance.inspected);
        assert_eq!(provenance.evidence[0], "document is empty");
    }

    #[test]
    fn oversize_document_is_not_inspected() {
        // The size guard runs before parsing, so padding is enough.
        let oversized = vec![b'%'; MAX_INSPECT_BYTES + 1];
        let provenance = classify_pdf(&oversized);
        assert!(!provenance.inspected);
        assert!(
            provenance.evidence[0].contains("inspection budget"),
            "{:?}",
            provenance.evidence
        );
    }

    #[test]
    fn pdf_without_acroform_is_not_a_form() {
        let bytes = build_pdf(Vec::new(), false, Vec::new());
        let provenance = classify_pdf(&bytes);

        assert!(provenance.inspected);
        assert!(!provenance.is_form());
        assert_eq!(provenance.origin, FormOrigin::NotAForm);
        assert_eq!(provenance.confidence, ProvenanceConfidence::Strong);
        assert!(!provenance.is_machine_filled());
    }

    #[test]
    fn blank_form_is_empty_not_machine_filled() {
        let bytes = build_pdf(
            vec![TestField::blank("surname"), TestField::blank("given")],
            false,
            Vec::new(),
        );
        let provenance = classify_pdf(&bytes);

        assert!(provenance.is_form());
        assert_eq!(provenance.origin, FormOrigin::Empty);
        assert_eq!(provenance.field_count, 2);
        assert_eq!(provenance.filled_field_count, 0);
        assert!(!provenance.is_machine_filled());
        assert!(!provenance.should_hold(FormProvenancePolicy::HoldForReview));
    }

    #[test]
    fn blocky_writer_shaped_output_is_machine_filled() {
        // Exactly what `fill_blocks` produces: values plus /DV defaults, no
        // appearance streams, and /NeedAppearances set.
        let bytes = build_pdf(
            vec![
                TestField::machine("surname", "Smith"),
                TestField::machine("given", "Ada"),
                TestField::machine("postcode", "MK7 6AA"),
            ],
            true,
            Vec::new(),
        );
        let provenance = classify_pdf(&bytes);

        assert!(provenance.inspected);
        assert!(provenance.is_form());
        assert_eq!(provenance.origin, FormOrigin::Machine);
        assert_eq!(provenance.confidence, ProvenanceConfidence::Probable);
        assert!(provenance.is_machine_filled());
        assert_eq!(provenance.field_count, 3);
        assert_eq!(provenance.filled_field_count, 3);
        assert_eq!(provenance.filled_with_default, 3);
        assert_eq!(provenance.values_without_appearance, 3);
        assert!(provenance.need_appearances);
        assert!(provenance.should_hold(FormProvenancePolicy::HoldForReview));
    }

    #[test]
    fn values_without_appearances_alone_are_machine_filled() {
        let bytes = build_pdf(
            vec![TestField {
                name: "surname",
                value: Some("Smith"),
                default: None,
                appearance: false,
            }],
            false,
            Vec::new(),
        );
        let provenance = classify_pdf(&bytes);

        assert_eq!(provenance.origin, FormOrigin::Machine);
        assert_eq!(provenance.confidence, ProvenanceConfidence::Probable);
        assert_eq!(provenance.values_without_appearance, 1);
        assert!(!provenance.need_appearances);
    }

    #[test]
    fn hand_filled_form_is_human() {
        // A viewer-filled form: values with appearance streams, no
        // /NeedAppearances, and no /DV rewrites.
        let bytes = build_pdf(
            vec![
                TestField::human("surname", "Smith"),
                TestField::human("given", "Ada"),
            ],
            false,
            Vec::new(),
        );
        let provenance = classify_pdf(&bytes);

        assert!(provenance.is_form());
        assert_eq!(provenance.origin, FormOrigin::Human);
        assert_eq!(provenance.confidence, ProvenanceConfidence::Probable);
        assert_eq!(provenance.filled_field_count, 2);
        assert_eq!(provenance.values_without_appearance, 0);
        assert_eq!(provenance.filled_with_default, 0);
        assert!(!provenance.is_machine_filled());
    }

    #[test]
    fn checkbox_off_state_is_not_a_value() {
        let mut unchecked = TestField::blank("consent");
        unchecked.value = Some("Off");
        let bytes = build_pdf(vec![unchecked], false, Vec::new());
        let provenance = classify_pdf(&bytes);

        assert_eq!(provenance.field_count, 1);
        assert_eq!(provenance.filled_field_count, 0);
        assert_eq!(provenance.origin, FormOrigin::Empty);
    }

    #[test]
    fn known_producer_marks_form_machine_filled() {
        let bytes = build_pdf(
            vec![TestField::human("surname", "Smith")],
            false,
            vec![("Producer", "blocky-writer 0.4.2")],
        );
        let provenance = classify_pdf(&bytes);

        assert_eq!(provenance.origin, FormOrigin::Machine);
        assert_eq!(provenance.confidence, ProvenanceConfidence::Strong);
        assert_eq!(provenance.producer.as_deref(), Some("blocky-writer 0.4.2"));
        assert_eq!(provenance.tool(), Some("blocky-writer 0.4.2"));
        assert!(provenance.summary().contains("blocky-writer"));
    }

    #[test]
    fn generator_producer_does_not_mark_blank_form() {
        // A blank template from a general PDF generator is still an empty
        // form, not a machine-filled one.
        let bytes = build_pdf(
            vec![TestField::blank("surname")],
            false,
            vec![("Producer", "iText 2.1.7")],
        );
        let provenance = classify_pdf(&bytes);

        assert_eq!(provenance.origin, FormOrigin::Empty);
        assert!(!provenance.is_machine_filled());
    }

    #[test]
    fn explicit_info_marker_outranks_structure() {
        // Structurally this looks hand-filled; the marker says otherwise.
        let bytes = build_pdf(
            vec![TestField::human("surname", "Smith")],
            false,
            vec![
                ("FillOrigin", "machine"),
                ("FillTool", "blocky-writer 0.4.2"),
                ("FillTime", "2026-09-27T10:15:00Z"),
            ],
        );
        let provenance = classify_pdf(&bytes);

        assert_eq!(provenance.origin, FormOrigin::Machine);
        assert_eq!(provenance.confidence, ProvenanceConfidence::Explicit);
        let marker = provenance.marker.expect("marker recorded");
        assert!(marker.contains("FillOrigin=machine"), "{marker}");
        assert!(marker.contains("blocky-writer 0.4.2"), "{marker}");
        assert!(marker.contains("2026-09-27T10:15:00Z"), "{marker}");
        assert_eq!(provenance.tool(), Some(marker.as_str()));
    }

    #[test]
    fn explicit_human_marker_does_not_trigger() {
        let bytes = build_pdf(
            vec![TestField::human("surname", "Smith")],
            false,
            vec![("FillOrigin", "human")],
        );
        let provenance = classify_pdf(&bytes);

        assert!(provenance.marker.is_none());
        assert_eq!(provenance.origin, FormOrigin::Human);
    }

    #[test]
    fn marker_scanner_understands_all_three_forms() {
        assert_eq!(
            scan_for_marker(br#"bw:fillOrigin="machine""#).as_deref(),
            Some("machine")
        );
        assert_eq!(
            scan_for_marker(br#"FILLORIGIN = 'machine-filled'"#).as_deref(),
            Some("machine-filled")
        );
        assert_eq!(
            scan_for_marker(b"bw:fillOrigin=machine bw:x=1").as_deref(),
            Some("machine")
        );
        assert_eq!(
            scan_for_marker(b"<bw:fillOrigin>machine</bw:fillOrigin>").as_deref(),
            Some("machine")
        );
        assert_eq!(scan_for_marker(b"no marker here"), None);
        assert_eq!(scan_for_marker(br#"fillOrigin="""#), None);
        // A different attribute whose name merely starts with the needle.
        assert_eq!(scan_for_marker(b"fillOriginTime=\"now\""), None);
    }

    #[test]
    fn unstructured_marker_in_raw_bytes_is_honoured() {
        // An uncompressed XMP-style packet carried in the file. The element
        // form is used because lopdf escapes quotes inside literal strings.
        let bytes = build_pdf(
            vec![TestField::human("surname", "Smith")],
            false,
            vec![("Note", "x:fillOrigin>machine</x:fillOrigin")],
        );
        let provenance = classify_pdf(&bytes);

        assert_eq!(provenance.origin, FormOrigin::Machine);
        assert_eq!(provenance.confidence, ProvenanceConfidence::Explicit);
        assert_eq!(provenance.marker.as_deref(), Some("machine"));
    }

    #[test]
    fn find_ci_is_case_insensitive() {
        assert_eq!(find_ci(b"abc FillOrigin xyz", b"fillorigin"), Some(4));
        assert_eq!(find_ci(b"abc", b"fillorigin"), None);
        assert_eq!(find_ci(b"abc", b""), None);
    }

    #[test]
    fn classify_for_print_skips_non_pdf_documents() {
        let provenance = classify_for_print(
            b"\xff\xd8\xff jpeg bytes",
            DocumentType::Jpeg,
            FormProvenancePolicy::Record,
        );
        assert!(!provenance.inspected);
        assert_eq!(provenance.evidence[0], "document is not a PDF");
    }

    #[test]
    fn classify_for_print_respects_off_policy() {
        let bytes = build_pdf(vec![TestField::machine("surname", "Smith")], true, Vec::new());
        let provenance = classify_for_print(&bytes, DocumentType::Pdf, FormProvenancePolicy::Off);
        assert!(!provenance.inspected);
        assert_eq!(provenance.origin, FormOrigin::Unknown);
    }

    #[test]
    fn classify_for_print_inspects_pdfs_under_record_policy() {
        let bytes = build_pdf(vec![TestField::machine("surname", "Smith")], true, Vec::new());
        let provenance =
            classify_for_print(&bytes, DocumentType::Pdf, FormProvenancePolicy::Record);
        assert!(provenance.inspected);
        assert!(provenance.is_machine_filled());
    }

    #[test]
    fn provenance_survives_json_roundtrip() {
        let bytes = build_pdf(
            vec![TestField::machine("surname", "Smith")],
            true,
            vec![("Producer", "blocky-writer 0.4.2")],
        );
        let provenance = classify_pdf(&bytes);
        let restored: FormProvenance =
            serde_json::from_str(&provenance.audit_json()).expect("deserialize");
        assert_eq!(restored, provenance);
    }

    #[test]
    fn reader_convenience_matches_standalone_classification() {
        use crate::PdfReader;

        let bytes = build_pdf(vec![TestField::machine("surname", "Smith")], true, Vec::new());
        let via_reader = PdfReader::from_bytes(&bytes)
            .expect("open test PDF")
            .form_provenance();
        let standalone = classify_pdf(&bytes);

        assert_eq!(via_reader.origin, standalone.origin);
        assert_eq!(via_reader.field_count, standalone.field_count);
        assert_eq!(
            via_reader.filled_field_count,
            standalone.filled_field_count
        );
    }
}
