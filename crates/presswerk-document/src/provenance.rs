// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// FFP v1.0.0 — Detection and classification (DETECTION.adoc)
// See presswerk-core/src/provenance.rs for the record types.

use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId};
use presswerk_core::provenance::{
    FfpAppearances, FfpClassification, FfpDeclared, FfpForm, FfpRecord,
};
use presswerk_core::types::{DocumentType, FormProvenancePolicy};
use tracing::{debug, instrument};

/// Documents larger than this are not inspected.
pub const MAX_INSPECT_BYTES: usize = 8 * 1024 * 1024;
const MAX_FIELD_NODES: u32 = 100_000;
const MAX_FIELD_DEPTH: u32 = 32;

/// Namespace URI for FFP.
const FFP_NS: &[u8] = b"https://hyperpolymath.dev/ns/form-fill-provenance/1.0/";

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

#[instrument(skip_all, fields(bytes = data.len(), policy = %policy.as_token()))]
pub fn classify_for_print(
    data: &[u8],
    document_type: DocumentType,
    policy: FormProvenancePolicy,
) -> FfpRecord {
    if !policy.inspects() {
        debug!("form provenance inspection disabled by policy");
        return FfpRecord::unreadable("form provenance inspection is disabled");
    }
    if document_type != DocumentType::Pdf {
        // Non-PDF is not a form. Use no-form rather than unreadable so it
        // doesn't pollute audit as a failure. This is honest: a JPEG has no
        // AcroForm.
        return FfpRecord::no_form(vec!["FFP-E-NO-ACROFORM".to_string()]);
    }
    classify_pdf(data)
}

#[instrument(skip_all, fields(bytes = data.len()))]
pub fn classify_pdf(data: &[u8]) -> FfpRecord {
    if data.is_empty() {
        return FfpRecord::unreadable("document is empty");
    }
    if data.len() > MAX_INSPECT_BYTES {
        return FfpRecord::unreadable(&format!(
            "document is {} bytes, above the {} byte inspection budget",
            data.len(),
            MAX_INSPECT_BYTES
        ));
    }
    let document = match Document::load_mem(data) {
        Ok(d) => d,
        Err(e) => {
            debug!(error = %e, "PDF parse failed during provenance inspection");
            return FfpRecord::unreadable(&format!("PDF parse failed: {e}"));
        }
    };
    classify_document(&document, data)
}

#[instrument(skip_all, fields(raw_bytes = raw.len()))]
pub fn classify_document(document: &Document, raw: &[u8]) -> FfpRecord {
    // Cheap pre-check (matches the probe): when raw bytes are available and
    // lack a %PDF- header, the document is unreadable. `raw` may be empty
    // when called from [`PdfReader`], which no longer retains the bytes;
    // then we rely on the already-parsed object graph.
    if !raw.is_empty() && !contains_bytes(raw, b"%PDF-") {
        return FfpRecord::unreadable("no %PDF- header in raw bytes");
    }

    let mut evidence: HashSet<String> = HashSet::new();
    let mut declared: Option<FfpDeclared> = None;
    let mut declared_filled_by: Option<String> = None;

    // Step 2 — declared marker (XMP in catalog Metadata)
    if let Ok(catalog) = document.catalog() {
        if let Ok(meta_obj) = catalog.get(b"Metadata") {
            let payload = get_metadata_payload(document, meta_obj);
            if payload.is_empty() {
                evidence.insert("FFP-E-XMP-UNREADABLE".to_string());
            } else if contains_bytes(&payload, FFP_NS) {
                // Namespace present — extract properties
                let filled_by = xmp_extract(&payload, "filledBy");
                let tool = xmp_extract(&payload, "tool");
                let tool_version = xmp_extract(&payload, "toolVersion");
                let filled_at = xmp_extract(&payload, "filledAt");
                let ap_gen_str = xmp_extract(&payload, "appearancesGenerated");
                let ap_gen = ap_gen_str
                    .as_deref()
                    .map(|s| s.eq_ignore_ascii_case("true"));

                if let Some(fb) = filled_by {
                    let fb_trim = fb.trim().to_string();
                    if fb_trim == "machine" {
                        evidence.insert("FFP-E-DECL-MACHINE".to_string());
                        declared_filled_by = Some(fb_trim.clone());
                        declared = Some(FfpDeclared {
                            filled_by: fb_trim,
                            tool: tool.filter(|s| !s.trim().is_empty()),
                            tool_version: tool_version.filter(|s| !s.trim().is_empty()),
                            filled_at: filled_at.filter(|s| !s.trim().is_empty()),
                            appearances_generated: ap_gen,
                        });
                    } else {
                        evidence.insert("FFP-E-DECL-UNRECOGNISED".to_string());
                        // Still record declared verbatim per spec
                        declared = Some(FfpDeclared {
                            filled_by: fb_trim.clone(),
                            tool: tool.filter(|s| !s.trim().is_empty()),
                            tool_version: tool_version.filter(|s| !s.trim().is_empty()),
                            filled_at: filled_at.filter(|s| !s.trim().is_empty()),
                            appearances_generated: ap_gen,
                        });
                        declared_filled_by = Some(fb_trim);
                        // For classification, unrecognised does not count as machine
                    }
                } else {
                    evidence.insert("FFP-E-DECL-UNRECOGNISED".to_string());
                    // Namespace present but filledBy absent
                }
            } else {
                // Metadata present but no FFP namespace — no declaration evidence
            }
        }
    }
    // If Metadata not present, no XMP evidence.

    // Step 3 — field tree
    let catalog = match document.catalog() {
        Ok(c) => c,
        Err(_) => {
            return FfpRecord {
                ffp: "1.0".to_string(),
                classification: FfpClassification::Unreadable,
                form: FfpForm::Unknown,
                filled_fields: 0,
                total_fields: 0,
                appearances: FfpAppearances::Unknown,
                declared,
                evidence: vec!["FFP-E-UNREADABLE".to_string()],
            };
        }
    };

    let acroform_dict = match catalog
        .get(b"AcroForm")
        .ok()
        .and_then(|obj| resolve_dict(document, obj))
    {
        Some(d) => d,
        None => {
            evidence.insert("FFP-E-NO-ACROFORM".to_string());
            // No form
            let mut ev: Vec<String> = evidence.into_iter().collect();
            ev.sort();
            return FfpRecord {
                ffp: "1.0".to_string(),
                classification: FfpClassification::NoForm,
                form: FfpForm::Absent,
                filled_fields: 0,
                total_fields: 0,
                appearances: FfpAppearances::NotApplicable,
                declared,
                evidence: ev,
            };
        }
    };

    // NeedAppearances
    if matches!(
        acroform_dict.get(b"NeedAppearances"),
        Ok(Object::Boolean(true))
    ) {
        evidence.insert("FFP-E-NEED-APPEARANCES".to_string());
    }

    // Walk fields
    let mut scan = FieldScan {
        total: 0,
        filled: 0,
        ap_missing: 0,
        visited: 0,
    };
    let mut seen: HashSet<ObjectId> = HashSet::new();
    if let Ok(fields_obj) = acroform_dict.get(b"Fields") {
        let inherit = Inherited {
            ft: "",
            ff: "",
            v: "",
        };
        walk_field(document, fields_obj, inherit, 0, &mut seen, &mut scan);
    }

    let total = scan.total;
    let filled = scan.filled;
    let ap_missing = scan.ap_missing;

    if total == 0 {
        evidence.insert("FFP-E-NO-ACROFORM".to_string());
        let mut ev: Vec<String> = evidence.into_iter().collect();
        ev.sort();
        return FfpRecord {
            ffp: "1.0".to_string(),
            classification: FfpClassification::NoForm,
            form: FfpForm::Absent,
            filled_fields: 0,
            total_fields: 0,
            appearances: FfpAppearances::NotApplicable,
            declared,
            evidence: ev,
        };
    }

    // Step 5 — appearances
    let appearances = if filled == 0 {
        FfpAppearances::NotApplicable
    } else if ap_missing > 0 {
        evidence.insert("FFP-E-AP-INCOMPLETE".to_string());
        FfpAppearances::Incomplete
    } else {
        FfpAppearances::Generated
    };

    if filled == 0 {
        evidence.insert("FFP-E-NO-VALUES".to_string());
    }

    // Step 6 — classify
    let form = if total > 0 {
        FfpForm::Present
    } else {
        FfpForm::Absent
    };
    let classification = if filled == 0 {
        FfpClassification::BlankForm
    } else if declared_filled_by.as_deref() == Some("machine") {
        FfpClassification::MachineFilled
    } else if evidence.contains("FFP-E-NEED-APPEARANCES")
        && evidence.contains("FFP-E-AP-INCOMPLETE")
    {
        FfpClassification::MachineFilledSuspected
    } else {
        FfpClassification::FilledUnknown
    };

    let mut ev: Vec<String> = evidence.into_iter().collect();
    ev.sort();
    // For blank-form, ensure NO-VALUES present, and appearance not-applicable
    // For unreadable, we already returned.

    FfpRecord {
        ffp: "1.0".to_string(),
        classification,
        form,
        filled_fields: filled,
        total_fields: total,
        appearances,
        declared,
        evidence: ev,
    }
}

// ---------------------------------------------------------------------------
// Field scan
// ---------------------------------------------------------------------------

struct FieldScan {
    total: u32,
    filled: u32,
    ap_missing: u32,
    visited: u32,
}

/// Field attributes inherited from ancestor nodes (ISO 32000 field
/// inheritance): field type `/FT`, field flags `/Ff`, and value `/V`.
#[derive(Clone, Copy)]
struct Inherited<'a> {
    ft: &'a str,
    ff: &'a str,
    v: &'a str,
}

fn walk_field(
    document: &Document,
    node: &Object,
    inherit: Inherited<'_>,
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
                walk_field(document, resolved, inherit, depth + 1, seen, scan);
            }
        }
        Object::Array(items) => {
            for item in items {
                walk_field(document, item, inherit, depth + 1, seen, scan);
            }
        }
        Object::Dictionary(dict) => {
            scan.visited += 1;
            // Inherit
            let ft = get_string_value(dict, b"FT").unwrap_or_else(|| inherit.ft.to_string());
            let ff = get_string_value(dict, b"Ff").unwrap_or_else(|| inherit.ff.to_string());
            let v_raw = dict
                .get(b"V")
                .ok()
                .map(object_to_string)
                .unwrap_or_else(|| inherit.v.to_string());

            // Check Kids
            if let Ok(kids_obj) = dict.get(b"Kids") {
                let kids_ids = get_array_ids(document, kids_obj);
                if !kids_ids.is_empty() {
                    // If the first kid is a widget annotation this is a
                    // terminal field with widget kids; otherwise the kids are
                    // intermediate field nodes and we recurse into them.
                    let is_widget = kids_ids
                        .first()
                        .and_then(|id| document.get_object(*id).ok())
                        .and_then(|o| {
                            let dict_opt = match o {
                                Object::Dictionary(d) => Some(d),
                                Object::Stream(s) => Some(&s.dict),
                                _ => None,
                            };
                            dict_opt.map(|d| {
                                d.get(b"Subtype")
                                    .ok()
                                    .map(|v| matches!(v, Object::Name(n) if n == b"Widget"))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);
                    if is_widget {
                        // Terminal field with widget kids
                        register_field(document, dict, &ft, &ff, &v_raw, &kids_ids, scan);
                    } else {
                        // Intermediate — recurse
                        let next = Inherited {
                            ft: &ft,
                            ff: &ff,
                            v: &v_raw,
                        };
                        for kid_id in kids_ids {
                            if let Ok(kid_obj) = document.get_object(kid_id) {
                                walk_field(document, kid_obj, next, depth + 1, seen, scan);
                            }
                        }
                    }
                    return;
                }
            }
            // No kids — terminal field (or Kids empty)
            // Need to check if this dict itself is a field (has FT or inherited)
            // The probe registers if FT matches Tx/Ch/Btn
            register_field(document, dict, &ft, &ff, &v_raw, &[], scan);
        }
        Object::Stream(stream) => {
            scan.visited += 1;
            let dict = &stream.dict;
            let ft = get_string_value(dict, b"FT").unwrap_or_else(|| inherit.ft.to_string());
            let ff = get_string_value(dict, b"Ff").unwrap_or_else(|| inherit.ff.to_string());
            let v_raw = dict
                .get(b"V")
                .ok()
                .map(object_to_string)
                .unwrap_or_else(|| inherit.v.to_string());
            if let Ok(kids_obj) = dict.get(b"Kids") {
                let kids_ids = get_array_ids(document, kids_obj);
                if !kids_ids.is_empty() {
                    // Similar widget check
                    let is_widget = kids_ids
                        .first()
                        .and_then(|id| document.get_object(*id).ok())
                        .map(|o| match o {
                            Object::Dictionary(d) => d
                                .get(b"Subtype")
                                .ok()
                                .map(|v| matches!(v, Object::Name(n) if n == b"Widget"))
                                .unwrap_or(false),
                            Object::Stream(s) => s
                                .dict
                                .get(b"Subtype")
                                .ok()
                                .map(|v| matches!(v, Object::Name(n) if n == b"Widget"))
                                .unwrap_or(false),
                            _ => false,
                        })
                        .unwrap_or(false);
                    if is_widget {
                        register_field(document, dict, &ft, &ff, &v_raw, &kids_ids, scan);
                    } else {
                        let next = Inherited {
                            ft: &ft,
                            ff: &ff,
                            v: &v_raw,
                        };
                        for kid_id in kids_ids {
                            if let Ok(kid_obj) = document.get_object(kid_id) {
                                walk_field(document, kid_obj, next, depth + 1, seen, scan);
                            }
                        }
                    }
                    return;
                }
            }
            register_field(document, dict, &ft, &ff, &v_raw, &[], scan);
        }
        _ => {}
    }
}

fn register_field(
    document: &Document,
    field_dict: &Dictionary,
    ft: &str,
    ff_str: &str,
    v_raw: &str,
    widget_ids: &[ObjectId],
    scan: &mut FieldScan,
) {
    // Normalize FT: should be like "/Tx", "/Ch", "/Btn"
    let ft_norm = if ft.starts_with('/') {
        ft.to_string()
    } else if !ft.is_empty() {
        format!("/{}", ft)
    } else {
        "".to_string()
    };
    if !matches!(ft_norm.as_str(), "/Tx" | "/Ch" | "/Btn") {
        return;
    }
    // Pushbuttons are not fillable: bit 17 (`0x10000`, ISO 32000 `/Ff`
    // Pushbutton) — the probe's `int(ff / 65536) % 2 == 1`.
    if ft_norm == "/Btn" {
        let f_num = ff_str.trim().parse::<i32>().unwrap_or(0);
        if (f_num & 0x10000) != 0 {
            return; // pushbutton, not fillable
        }
    }

    scan.total += 1;
    // Check meaningful value
    // Need to get actual V object for meaningful check, not just string
    let v_obj = field_dict.get(b"V").ok();
    let meaningful = is_meaningful(document, &ft_norm, v_obj, v_raw);
    if !meaningful {
        return;
    }
    scan.filled += 1;

    // Appearance check per widget.
    if widget_ids.is_empty() {
        // The field dictionary is itself the widget; check its own /AP.
        if !has_normal_appearance_dict(document, field_dict, v_raw) {
            scan.ap_missing += 1;
        }
        return;
    }

    for &wid in widget_ids {
        let Ok(widget_obj) = document.get_object(wid) else {
            scan.ap_missing += 1;
            continue;
        };
        let widget_dict = match widget_obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &s.dict,
            _ => continue,
        };
        if !has_normal_appearance_dict(document, widget_dict, v_raw) {
            scan.ap_missing += 1;
        }
    }
}

fn has_normal_appearance_dict(document: &Document, widget_dict: &Dictionary, v_raw: &str) -> bool {
    // Check /AP
    let Ok(ap_obj) = widget_dict.get(b"AP") else {
        return false;
    };
    let Some(ap_dict) = resolve_dict(document, ap_obj) else {
        return false;
    };
    let Ok(n_obj) = ap_dict.get(b"N") else {
        return false;
    };
    // Resolve /N: a stream is a normal appearance; a dictionary is keyed by
    // appearance state and needs the current state to select an entry.
    match n_obj {
        Object::Stream(_) => true,
        Object::Reference(id) => match document.get_object(*id) {
            Ok(Object::Stream(_)) => true,
            Ok(Object::Dictionary(d)) => state_appearance_exists(document, widget_dict, d, v_raw),
            _ => false,
        },
        Object::Dictionary(d) => state_appearance_exists(document, widget_dict, d, v_raw),
        _ => false,
    }
}

/// `/AP /N` is a subdictionary: the appearance exists only if the current
/// appearance state (`/AS`, falling back to a name `/V` such as `/Yes`)
/// selects an entry in it.
fn state_appearance_exists(
    document: &Document,
    widget_dict: &Dictionary,
    n_dict: &Dictionary,
    v_raw: &str,
) -> bool {
    let state = get_current_state(widget_dict, v_raw);
    if state.is_empty() {
        return false;
    }
    // Dictionary keys are the state without its leading `/`.
    let key = state.trim_start_matches('/');
    match n_dict.get(key.as_bytes()) {
        Ok(Object::Reference(sid)) => {
            matches!(
                document.get_object(*sid),
                Ok(Object::Stream(_)) | Ok(Object::Dictionary(_))
            )
        }
        Ok(Object::Stream(_)) | Ok(Object::Dictionary(_)) => true,
        _ => false,
    }
}

fn get_current_state(widget_dict: &Dictionary, v_raw: &str) -> String {
    // Try /AS on widget, then field? For now only widget
    if let Ok(Object::Name(name)) = widget_dict.get(b"AS") {
        return format!("/{}", String::from_utf8_lossy(name));
    }
    if let Ok(Object::String(bytes, _)) = widget_dict.get(b"AS") {
        return String::from_utf8_lossy(bytes).to_string();
    }
    // Try V raw if it's a name like /Yes
    let v_trim = v_raw.trim();
    if v_trim.starts_with('/') {
        return v_trim.to_string();
    }
    "".to_string()
}

fn is_meaningful(document: &Document, ft: &str, v_obj: Option<&Object>, v_raw: &str) -> bool {
    match ft {
        "/Btn" => {
            // V must be name other than /Off
            if let Some(obj) = v_obj {
                match obj {
                    Object::Name(name) => {
                        return !name.eq_ignore_ascii_case(b"Off") && !name.is_empty();
                    }
                    Object::String(bytes, _) => {
                        // Could be string? treat as name?
                        return !bytes.eq_ignore_ascii_case(b"Off") && !bytes.is_empty();
                    }
                    Object::Reference(id) => {
                        if let Ok(resolved) = document.get_object(*id) {
                            match resolved {
                                Object::Name(name) => {
                                    return !name.eq_ignore_ascii_case(b"Off") && !name.is_empty();
                                }
                                Object::String(bytes, _) => {
                                    return !bytes.eq_ignore_ascii_case(b"Off") && !bytes.is_empty();
                                }
                                _ => return false,
                            }
                        }
                        return false;
                    }
                    _ => return false,
                }
            }
            // Fallback to raw string
            let t = v_raw.trim();
            if t.is_empty() || t.eq_ignore_ascii_case("/Off") || t.eq_ignore_ascii_case("Off") {
                return false;
            }
            t.starts_with('/')
        }
        "/Tx" => {
            if let Some(obj) = v_obj {
                return is_string_object_meaningful(document, obj);
            }
            is_string_meaningful(v_raw.as_bytes())
        }
        "/Ch" => {
            if let Some(obj) = v_obj {
                match obj {
                    Object::String(_, _) => return is_string_object_meaningful(document, obj),
                    Object::Name(n) => return !n.is_empty() && !n.eq_ignore_ascii_case(b"Off"),
                    Object::Array(items) => {
                        for item in items {
                            if is_string_object_meaningful(document, item) {
                                return true;
                            }
                            if let Object::Reference(id) = item {
                                if let Ok(res) = document.get_object(*id) {
                                    if is_string_object_meaningful(document, res) {
                                        return true;
                                    }
                                }
                            }
                        }
                        return false;
                    }
                    Object::Reference(id) => {
                        if let Ok(res) = document.get_object(*id) {
                            match res {
                                Object::Array(items) => {
                                    for item in items {
                                        if is_string_object_meaningful(document, item) {
                                            return true;
                                        }
                                    }
                                    return false;
                                }
                                _ => return is_string_object_meaningful(document, res),
                            }
                        }
                        return false;
                    }
                    _ => return false,
                }
            }
            is_string_meaningful(v_raw.as_bytes())
        }
        _ => false,
    }
}

fn is_string_object_meaningful(document: &Document, obj: &Object) -> bool {
    match obj {
        Object::String(bytes, _) => is_string_meaningful(bytes),
        Object::Name(name) => !name.is_empty() && !name.eq_ignore_ascii_case(b"Off"),
        Object::Reference(id) => {
            if let Ok(res) = document.get_object(*id) {
                is_string_object_meaningful(document, res)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn is_string_meaningful(bytes: &[u8]) -> bool {
    // For Tx: string containing at least one non-whitespace
    // Whitespace is U+0020, 0009, 000D, 000A
    // For raw v_raw string, it may include parentheses like "(Jewell)"
    // We need to handle both raw object string and decoded bytes.
    // If bytes contains parentheses, strip them? For object string, bytes is content without parens.
    // For v_raw string like "(Jewell)" or "(   )" or "()", we need to detect.
    let mut s = bytes;
    // If it looks like "(...)" from v_raw, strip outer parens and unescape?
    // Probe's string_meaningful handles `(`, `<` hex, etc.
    // We'll normalize: trim whitespace, then check if contains non-whitespace
    // For hex strings like "<...>", similar.
    // Simplify: remove surrounding `(` `)` or `<` `>` if present, then trim.
    if s.len() >= 2 && s[0] == b'(' && s[s.len() - 1] == b')' {
        s = &s[1..s.len() - 1];
        // Unescape \( \) \\
        // For meaningful check, just check if after trimming whitespace there's content
    }
    if s.len() >= 2 && s[0] == b'<' && s[s.len() - 1] == b'>' {
        s = &s[1..s.len() - 1];
        // Hex: remove whitespace and 0s? Probe does gsub(/[ \t\r\n0]/,"",s) for hex.
        // We'll filter.
        let filtered: Vec<u8> = s
            .iter()
            .copied()
            .filter(|&b| !b.is_ascii_whitespace() && b != b'0')
            .collect();
        return !filtered.is_empty();
    }
    // Trim whitespace bytes
    let trimmed: Vec<u8> = s
        .iter()
        .copied()
        .filter(|&b| b != b' ' && b != b'\t' && b != b'\r' && b != b'\n')
        .collect();
    !trimmed.is_empty()
}

// ---------------------------------------------------------------------------
// Helpers for PDF object graph
// ---------------------------------------------------------------------------

fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(&stream.dict),
        Object::Reference(id) => {
            document
                .get_object(*id)
                .ok()
                .and_then(|resolved| match resolved {
                    Object::Dictionary(dict) => Some(dict),
                    Object::Stream(stream) => Some(&stream.dict),
                    _ => None,
                })
        }
        _ => None,
    }
}

fn get_string_value(dict: &Dictionary, key: &[u8]) -> Option<String> {
    match dict.get(key) {
        Ok(Object::Name(name)) => Some(String::from_utf8_lossy(name).to_string()),
        Ok(Object::String(bytes, _)) => Some(String::from_utf8_lossy(bytes).to_string()),
        Ok(Object::Integer(i)) => Some(i.to_string()),
        Ok(Object::Real(r)) => Some(r.to_string()),
        Ok(Object::Boolean(b)) => Some(if *b {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        Ok(Object::Reference(id)) => {
            // Try to resolve? For FT, Ff, V inheritance, the referenced value would be resolved elsewhere.
            // But we can try to get string from referenced object via dict lookup not available.
            // Fallback: return None so inheritance handled.
            let _ = id;
            None
        }
        _ => None,
    }
}

fn object_to_string(obj: &Object) -> String {
    match obj {
        Object::String(bytes, _) => format!("({})", String::from_utf8_lossy(bytes)),
        Object::Name(name) => format!("/{}", String::from_utf8_lossy(name)),
        Object::Boolean(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Object::Integer(i) => i.to_string(),
        Object::Real(r) => r.to_string(),
        Object::Null => "".to_string(),
        Object::Array(items) => {
            let mut s = String::from("[");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(&object_to_string(item));
            }
            s.push(']');
            s
        }
        Object::Dictionary(d) => format!("<<{}>>", d.len()),
        Object::Stream(_) => "<<stream>>".to_string(),
        Object::Reference(id) => format!("{} 0 R", id.0),
    }
}

fn get_array_ids(document: &Document, obj: &Object) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    match obj {
        Object::Array(items) => {
            for item in items {
                if let Object::Reference(id) = item {
                    ids.push(*id);
                }
            }
        }
        Object::Reference(id) => {
            if let Ok(Object::Array(items)) = document.get_object(*id) {
                for item in items {
                    if let Object::Reference(kid) = item {
                        ids.push(*kid);
                    }
                }
            }
        }
        _ => {}
    }
    ids
}

// ---------------------------------------------------------------------------
// XMP helpers
// ---------------------------------------------------------------------------

fn get_metadata_payload(document: &Document, meta_obj: &Object) -> Vec<u8> {
    let obj = match meta_obj {
        Object::Reference(id) => match document.get_object(*id) {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        },
        other => other,
    };
    if let Object::Stream(stream) = obj {
        stream.content.clone()
    } else {
        Vec::new()
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn xmp_extract(payload: &[u8], prop: &str) -> Option<String> {
    // Attribute form: prop="value" or prop='value'
    // Search for prop with optional prefix like ffp:prop or any:prop
    // We look for `prop` then `\s*=\s*"`
    // To handle prefix, we search for `prop` substring case-sensitive but allow preceding `:` or whitespace
    // Simpler: use regex-like search for `prop\s*=`
    // We'll do manual scan.
    let prop_bytes = prop.as_bytes();
    let payload_bytes = payload;

    // Try attribute form
    if let Some(pos) = find_ci(payload_bytes, prop_bytes) {
        // Ensure it's a property name: check char before is ':' or whitespace or '"' or '''
        // Check after prop is whitespace then =
        let after = pos + prop_bytes.len();
        if after < payload_bytes.len() {
            let mut rest = &payload_bytes[after..];
            // trim whitespace
            rest = trim_ascii_start(rest);
            if !rest.is_empty() && rest[0] == b'=' {
                rest = trim_ascii_start(&rest[1..]);
                if !rest.is_empty() {
                    match rest[0] {
                        b'"' | b'\'' => {
                            let quote = rest[0];
                            rest = &rest[1..];
                            if let Some(end) = rest.iter().position(|&b| b == quote) {
                                let val = &rest[..end];
                                let s = String::from_utf8_lossy(val).trim().to_string();
                                if !s.is_empty() {
                                    return Some(s);
                                }
                            }
                        }
                        _ => {
                            // Unquoted (should not happen for XMP but handle)
                            let mut end = 0;
                            while end < rest.len()
                                && !rest[end].is_ascii_whitespace()
                                && rest[end] != b'>'
                            {
                                end += 1;
                            }
                            if end > 0 {
                                let val = &rest[..end];
                                let s = String::from_utf8_lossy(val).trim().to_string();
                                if !s.is_empty() {
                                    return Some(s);
                                }
                            }
                        }
                    }
                }
            }
            // If not attribute, try element form
            // rest after prop may be `>value<`
            let mut rest2 = &payload_bytes[pos + prop_bytes.len()..];
            // Check if next char is `>` after optional whitespace?
            // Actually element form is `<prefix:prop>value</prefix:prop>`
            // After prop name, should be `>`
            if !rest2.is_empty() && rest2[0] == b'>' {
                rest2 = &rest2[1..];
                if let Some(end) = rest2.iter().position(|&b| b == b'<') {
                    let val = &rest2[..end];
                    let s = String::from_utf8_lossy(val).trim().to_string();
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
            // Also try with namespace prefix: search for `:prop` then `>`
            // The above already handles because we searched for prop without prefix, and after is `>`
            // For attribute form we already handled `=`
            // For element, we need to check `>value`
            // Already did.

            // Alternative: search for `<...:prop>` pattern more robustly
            // We can also do a second pass: find `:<prop>` then `>value<`
        }
    }

    // Second pass for element form with prefix: search for `:prop>`
    let prefixed = format!(":{}", prop);
    if let Some(pos) = find_ci(payload_bytes, prefixed.as_bytes()) {
        let after = pos + prefixed.len();
        if after < payload_bytes.len() && payload_bytes[after] == b'>' {
            let rest = &payload_bytes[after + 1..];
            if let Some(end) = rest.iter().position(|&b| b == b'<') {
                let val = &rest[..end];
                let s = String::from_utf8_lossy(val).trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        // Also check attribute with prefix: `:prop="value"`
        if after < payload_bytes.len() {
            let mut rest = &payload_bytes[after..];
            rest = trim_ascii_start(rest);
            if !rest.is_empty() && rest[0] == b'=' {
                rest = trim_ascii_start(&rest[1..]);
                if !rest.is_empty() && (rest[0] == b'"' || rest[0] == b'\'') {
                    let quote = rest[0];
                    rest = &rest[1..];
                    if let Some(end) = rest.iter().position(|&b| b == quote) {
                        let s = String::from_utf8_lossy(&rest[..end]).trim().to_string();
                        if !s.is_empty() {
                            return Some(s);
                        }
                    }
                }
            }
        }
    }

    None
}

fn find_ci(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    &bytes[i..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Dictionary, Document, Object, StringFormat};

    struct TestField {
        name: &'static str,
        value: Option<&'static str>,
        appearance: bool,
    }

    impl TestField {
        const fn human(name: &'static str, value: &'static str) -> Self {
            Self {
                name,
                value: Some(value),
                appearance: true,
            }
        }
        const fn machine(name: &'static str, value: &'static str) -> Self {
            Self {
                name,
                value: Some(value),
                appearance: false,
            }
        }
        const fn blank(name: &'static str) -> Self {
            Self {
                name,
                value: None,
                appearance: false,
            }
        }
    }

    fn build_pdf(fields: Vec<TestField>, need_appearances: bool, xmp: Option<String>) -> Vec<u8> {
        let mut document = Document::with_version("1.7");
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
        let mut field_refs = Vec::new();
        for field in &fields {
            let mut dict = Dictionary::new();
            dict.set(
                "T",
                Object::String(field.name.as_bytes().to_vec(), StringFormat::Literal),
            );
            dict.set("FT", Object::Name(b"Tx".to_vec()));
            if let Some(v) = field.value {
                dict.set(
                    "V",
                    Object::String(v.as_bytes().to_vec(), StringFormat::Literal),
                );
            }
            if field.appearance {
                // A viewer-written appearance: /N references a real
                // appearance stream, as in the conformance `viewer-filled`
                // vector.
                let stream_id = document.add_object(Object::Stream(lopdf::Stream::new(
                    Dictionary::new(),
                    Vec::new(),
                )));
                let mut ap = Dictionary::new();
                ap.set("N", Object::Reference(stream_id));
                dict.set("AP", Object::Dictionary(ap));
            }
            field_refs.push(Object::Reference(
                document.add_object(Object::Dictionary(dict)),
            ));
        }
        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        if !fields.is_empty() || need_appearances || xmp.is_some() {
            let mut acroform = Dictionary::new();
            if !field_refs.is_empty() {
                acroform.set("Fields", Object::Array(field_refs));
            }
            if need_appearances {
                acroform.set("NeedAppearances", Object::Boolean(true));
            }
            catalog.set("AcroForm", Object::Dictionary(acroform));
        }
        let catalog_id = document.add_object(Object::Dictionary(catalog));
        document.trailer.set("Root", Object::Reference(catalog_id));
        if let Some(xmp_str) = xmp {
            let mut meta_dict = Dictionary::new();
            meta_dict.set("Type", Object::Name(b"Metadata".to_vec()));
            meta_dict.set("Subtype", Object::Name(b"XML".to_vec()));
            let meta_id = document.add_object(Object::Stream(lopdf::Stream::new(
                meta_dict,
                xmp_str.into_bytes(),
            )));
            if let Ok(Object::Dictionary(cat)) = document.get_object_mut(catalog_id) {
                cat.set("Metadata", Object::Reference(meta_id));
            }
        }
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("serialise");
        bytes
    }

    #[test]
    fn empty_is_unreadable() {
        let r = classify_pdf(b"");
        assert_eq!(r.classification, FfpClassification::Unreadable);
    }

    #[test]
    fn blank_form_not_machine() {
        let bytes = build_pdf(
            vec![TestField::blank("surname"), TestField::blank("given")],
            false,
            None,
        );
        let r = classify_pdf(&bytes);
        assert_eq!(r.classification, FfpClassification::BlankForm);
        assert_eq!(r.filled_fields, 0);
    }

    #[test]
    fn machine_suspected_without_marker() {
        let bytes = build_pdf(
            vec![
                TestField::machine("surname", "Smith"),
                TestField::machine("given", "Ada"),
            ],
            true,
            None,
        );
        let r = classify_pdf(&bytes);
        assert_eq!(r.classification, FfpClassification::MachineFilledSuspected);
    }

    #[test]
    fn machine_filled_with_marker() {
        let xmp = r#"<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:ffp="https://hyperpolymath.dev/ns/form-fill-provenance/1.0/" ffp:filledBy="machine" ffp:tool="blocky-writer"/></rdf:RDF></x:xmpmeta>"#.to_string();
        let bytes = build_pdf(
            vec![TestField::machine("surname", "Smith")],
            true,
            Some(xmp),
        );
        let r = classify_pdf(&bytes);
        assert_eq!(r.classification, FfpClassification::MachineFilled);
        assert_eq!(
            r.declared.as_ref().unwrap().tool.as_deref(),
            Some("blocky-writer")
        );
    }

    #[test]
    fn viewer_filled_is_unknown() {
        let bytes = build_pdf(
            vec![
                TestField::human("surname", "Smith"),
                TestField::human("given", "Ada"),
            ],
            false,
            None,
        );
        let r = classify_pdf(&bytes);
        assert_eq!(r.classification, FfpClassification::FilledUnknown);
        assert_eq!(r.appearances, FfpAppearances::Generated);
    }
}
