// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// PDF reader — open, inspect, merge, split, and rotate existing PDF documents
// using the `lopdf` crate.

use std::path::Path;

use lopdf::{Document, Object, ObjectId};
use presswerk_core::error::PresswerkError;
use tracing::{debug, info, instrument, warn};

/// Reads and manipulates existing PDF files.
///
/// Wraps `lopdf::Document` and provides higher-level operations such as merging
/// multiple files, extracting page ranges, and rotating pages.
pub struct PdfReader {
    /// The underlying lopdf document.
    document: Document,
    /// Source path, if opened from a file (useful for diagnostics).
    source_path: Option<String>,
}

impl PdfReader {
    // -- Construction ---------------------------------------------------------

    /// Open a PDF from the filesystem.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PresswerkError> {
        let path_ref = path.as_ref();
        info!("Opening PDF: {}", path_ref.display());

        let document = Document::load(path_ref).map_err(|err| {
            PresswerkError::PdfError(format!("failed to open {}: {}", path_ref.display(), err))
        })?;

        debug!(pages = document.get_pages().len(), "PDF loaded");

        Ok(Self {
            document,
            source_path: Some(path_ref.display().to_string()),
        })
    }

    /// Create a reader from raw PDF bytes already in memory.
    #[instrument(skip_all, fields(bytes_len = data.len()))]
    pub fn from_bytes(data: &[u8]) -> Result<Self, PresswerkError> {
        let document = Document::load_mem(data).map_err(|err| {
            PresswerkError::PdfError(format!("failed to load PDF from memory: {}", err))
        })?;

        debug!(pages = document.get_pages().len(), "PDF loaded from bytes");

        Ok(Self {
            document,
            source_path: None,
        })
    }

    // -- Inspection -----------------------------------------------------------

    /// Number of pages in the document.
    pub fn page_count(&self) -> usize {
        self.document.get_pages().len()
    }

    /// Return the source path if the reader was created via [`PdfReader::open`].
    pub fn source_path(&self) -> Option<&str> {
        self.source_path.as_deref()
    }

    // -- Extraction -----------------------------------------------------------

    /// Extract a single page (1-indexed) into a new standalone PDF document.
    ///
    /// Returns the serialised bytes of the single-page PDF.
    #[instrument(skip(self), fields(page_number))]
    pub fn extract_page(&self, page_number: u32) -> Result<Vec<u8>, PresswerkError> {
        let pages = self.document.get_pages();
        if page_number == 0 || page_number as usize > pages.len() {
            return Err(PresswerkError::PdfError(format!(
                "page {} out of range (document has {} pages)",
                page_number,
                pages.len()
            )));
        }

        // lopdf pages are keyed by 1-indexed page number.
        let page_object_id: ObjectId = *pages.get(&page_number).ok_or_else(|| {
            PresswerkError::PdfError(format!("page {} not found in page tree", page_number))
        })?;

        let mut new_doc = Document::with_version("1.5");
        clone_page_into(&self.document, &mut new_doc, page_object_id)?;

        let mut output = Vec::new();
        new_doc.save_to(&mut output).map_err(|err| {
            PresswerkError::PdfError(format!("failed to serialise extracted page: {}", err))
        })?;

        debug!(page_number, output_bytes = output.len(), "Page extracted");
        Ok(output)
    }

    /// Split the document at `after_page` (1-indexed, inclusive) producing two
    /// byte-vectors: pages [1..=after_page] and pages [after_page+1..=end].
    #[instrument(skip(self), fields(after_page))]
    pub fn split(&self, after_page: u32) -> Result<(Vec<u8>, Vec<u8>), PresswerkError> {
        let total = self.page_count() as u32;
        if after_page == 0 || after_page >= total {
            return Err(PresswerkError::PdfError(format!(
                "split point {} invalid for {} page document",
                after_page, total
            )));
        }

        info!(after_page, total, "Splitting PDF");

        let first = self.extract_page_range(1, after_page)?;
        let second = self.extract_page_range(after_page + 1, total)?;

        Ok((first, second))
    }

    /// Merge this document with one or more other PDF byte-slices, producing a
    /// combined PDF. Pages appear in the order: self, then each supplied
    /// document in order.
    #[instrument(skip_all, fields(additional_count = others.len()))]
    pub fn merge(&self, others: &[&[u8]]) -> Result<Vec<u8>, PresswerkError> {
        info!(
            base_pages = self.page_count(),
            additional_documents = others.len(),
            "Merging PDFs"
        );

        let mut merged = self.document.clone();

        for (index, other_bytes) in others.iter().enumerate() {
            let other_doc = Document::load_mem(other_bytes).map_err(|err| {
                PresswerkError::PdfError(format!(
                    "failed to load additional PDF #{}: {}",
                    index + 1,
                    err
                ))
            })?;

            let other_pages = other_doc.get_pages();
            let mut page_numbers: Vec<u32> = other_pages.keys().copied().collect();
            page_numbers.sort();

            for page_num in page_numbers {
                let page_id = other_pages[&page_num];
                clone_page_into(&other_doc, &mut merged, page_id)?;
            }
        }

        let mut output = Vec::new();
        merged.save_to(&mut output).map_err(|err| {
            PresswerkError::PdfError(format!("failed to serialise merged PDF: {}", err))
        })?;

        debug!(output_bytes = output.len(), "Merge complete");
        Ok(output)
    }

    /// Rotate a specific page by `degrees` (must be a multiple of 90).
    ///
    /// Returns the full document as bytes with the rotation applied.
    #[instrument(skip(self), fields(page_number, degrees))]
    pub fn rotate_page(&self, page_number: u32, degrees: i32) -> Result<Vec<u8>, PresswerkError> {
        if degrees % 90 != 0 {
            return Err(PresswerkError::PdfError(format!(
                "rotation must be a multiple of 90, got {}",
                degrees
            )));
        }

        let mut doc = self.document.clone();
        let pages = doc.get_pages();

        let page_id = *pages.get(&page_number).ok_or_else(|| {
            PresswerkError::PdfError(format!(
                "page {} not found (document has {} pages)",
                page_number,
                pages.len()
            ))
        })?;

        // Read existing /Rotate value, default 0.
        let existing_rotation = doc
            .get_object(page_id)
            .ok()
            .and_then(|obj| match obj {
                Object::Dictionary(dict) => dict
                    .get(b"Rotate")
                    .ok()
                    .and_then(|r| r.as_i64().ok())
                    .map(|v| v as i32),
                _ => None,
            })
            .unwrap_or(0);

        let new_rotation = (existing_rotation + degrees).rem_euclid(360);

        // Set /Rotate on the page dictionary.
        if let Ok(Object::Dictionary(dict)) = doc.get_object_mut(page_id) {
            dict.set("Rotate", Object::Integer(new_rotation as i64));
        }

        info!(page_number, existing_rotation, new_rotation, "Page rotated");

        let mut output = Vec::new();
        doc.save_to(&mut output).map_err(|err| {
            PresswerkError::PdfError(format!("failed to serialise rotated PDF: {}", err))
        })?;

        Ok(output)
    }

    // -- Helpers --------------------------------------------------------------

    /// Extract a contiguous range of pages [start..=end] (1-indexed) into a new
    /// PDF returned as bytes.
    fn extract_page_range(&self, start: u32, end: u32) -> Result<Vec<u8>, PresswerkError> {
        let pages = self.document.get_pages();
        let mut new_doc = Document::with_version("1.5");

        for page_num in start..=end {
            let page_id = *pages.get(&page_num).ok_or_else(|| {
                PresswerkError::PdfError(format!(
                    "page {} not found during range extraction",
                    page_num
                ))
            })?;
            clone_page_into(&self.document, &mut new_doc, page_id)?;
        }

        let mut output = Vec::new();
        new_doc.save_to(&mut output).map_err(|err| {
            PresswerkError::PdfError(format!("failed to serialise page range: {}", err))
        })?;

        Ok(output)
    }
}

/// Clone a single page object (and its referenced resources) from `source` into
/// `target`, appending it as the last page.
///
/// This performs a shallow clone — stream data, fonts, and images referenced by
/// the page dictionary are copied as new objects in the target document.
fn clone_page_into(
    source: &Document,
    target: &mut Document,
    page_id: ObjectId,
) -> Result<(), PresswerkError> {
    let page_object = source.get_object(page_id).map_err(|err| {
        PresswerkError::PdfError(format!("cannot read page object {:?}: {}", page_id, err))
    })?;

    // Deep-clone the page object and all objects it transitively references.
    let cloned_id = clone_object_recursive(source, target, page_id, page_object)?;

    // Retrieve the document's page tree root (/Pages) and append the new page.
    let pages_id = target
        .catalog()
        .map_err(|err| PresswerkError::PdfError(format!("no catalog: {}", err)))
        .and_then(|catalog| {
            catalog
                .get(b"Pages")
                .map_err(|err| PresswerkError::PdfError(format!("no /Pages: {}", err)))
                .and_then(|pages_ref| match pages_ref {
                    Object::Reference(id) => Ok(*id),
                    _ => Err(PresswerkError::PdfError(
                        "/Pages is not a reference".to_string(),
                    )),
                })
        })?;

    // Add page reference to the /Kids array.
    if let Ok(Object::Dictionary(pages_dict)) = target.get_object_mut(pages_id) {
        if let Ok(Object::Array(kids)) = pages_dict.get_mut(b"Kids") {
            kids.push(Object::Reference(cloned_id));
        }
        // Increment /Count.
        if let Ok(count_obj) = pages_dict.get_mut(b"Count")
            && let Object::Integer(count) = count_obj
        {
            *count += 1;
        }
    }

    // Set the cloned page's /Parent to point at the target's /Pages node.
    if let Ok(Object::Dictionary(page_dict)) = target.get_object_mut(cloned_id) {
        page_dict.set("Parent", Object::Reference(pages_id));
    }

    Ok(())
}

/// Recursively clone an object from `source` into `target`, returning the new
/// object ID in `target`.
///
/// References within the object graph are followed and cloned, avoiding infinite
/// loops through the /Parent back-reference (which is patched by the caller).
fn clone_object_recursive(
    source: &Document,
    target: &mut Document,
    _source_id: ObjectId,
    object: &Object,
) -> Result<ObjectId, PresswerkError> {
    let cloned_object = deep_clone_object(source, target, object)?;
    let new_id = target.add_object(cloned_object);
    Ok(new_id)
}

/// Deep-clone a single lopdf Object, recursively resolving references (except
/// /Parent which is deliberately skipped to avoid circular cloning).
fn deep_clone_object(
    source: &Document,
    target: &mut Document,
    object: &Object,
) -> Result<Object, PresswerkError> {
    match object {
        Object::Dictionary(dict) => {
            let mut new_dict = lopdf::Dictionary::new();
            for (key, value) in dict.iter() {
                // Skip /Parent to avoid circular references; the caller patches it.
                if key == b"Parent" {
                    continue;
                }
                let cloned_value = deep_clone_object(source, target, value)?;
                new_dict.set(key.clone(), cloned_value);
            }
            Ok(Object::Dictionary(new_dict))
        }
        Object::Array(arr) => {
            let mut new_arr = Vec::with_capacity(arr.len());
            for item in arr {
                new_arr.push(deep_clone_object(source, target, item)?);
            }
            Ok(Object::Array(new_arr))
        }
        Object::Reference(ref_id) => {
            // Resolve the reference in the source, clone it, and return a new
            // reference in the target.
            match source.get_object(*ref_id) {
                Ok(referenced) => {
                    let cloned = deep_clone_object(source, target, referenced)?;
                    let new_id = target.add_object(cloned);
                    Ok(Object::Reference(new_id))
                }
                Err(err) => {
                    warn!(?ref_id, %err, "Cannot resolve reference, using Null");
                    Ok(Object::Null)
                }
            }
        }
        Object::Stream(stream) => {
            let mut new_dict = lopdf::Dictionary::new();
            for (key, value) in stream.dict.iter() {
                if key == b"Parent" {
                    continue;
                }
                let cloned_value = deep_clone_object(source, target, value)?;
                new_dict.set(key.clone(), cloned_value);
            }
            Ok(Object::Stream(lopdf::Stream::new(
                new_dict,
                stream.content.clone(),
            )))
        }
        // All other object types (Boolean, Integer, Real, String, Name, Null)
        // are trivially cloneable.
        other => Ok(other.clone()),
    }
}
