// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Document format auto-conversion for legacy printer support.
//
// Conversion chain: PDF → PostScript → PCL → Raster (rendered page images).
// The strategy is to query the printer's `document-format-supported` and
// automatically convert to the best format the printer understands.
// Rasterisation (rendering pages as images) is the ultimate fallback —
// every printer can print images.

use std::collections::HashSet;

use tracing::{debug, info, warn};

use presswerk_core::error::{PresswerkError, Result};
use presswerk_core::types::DocumentType;

/// Document converter with format chain.
pub struct DocumentConverter;

impl DocumentConverter {
    /// Choose the best document format for a printer and convert if needed.
    ///
    /// Strategy: Start with the original format. If the printer doesn't
    /// support it, try each fallback in order (PDF → PS → PCL → raster).
    /// Only convert when necessary — never degrade quality needlessly.
    ///
    /// SECURITY NOTE: The chain always prefers the format that preserves
    /// the most fidelity. Rasterisation is the last resort, not the default.
    pub fn auto_convert(
        document_bytes: &[u8],
        source_type: DocumentType,
        supported_formats: &HashSet<String>,
    ) -> Result<(Vec<u8>, DocumentType)> {
        // If the printer supports the source format, no conversion needed
        if supported_formats.is_empty() || supported_formats.contains(source_type.mime_type()) {
            debug!(
                format = source_type.mime_type(),
                "printer supports source format — no conversion needed"
            );
            return Ok((document_bytes.to_vec(), source_type));
        }

        // Try the conversion chain in preference order
        let chain = conversion_chain(source_type);

        for target_type in chain {
            if supported_formats.contains(target_type.mime_type()) {
                info!(
                    from = source_type.mime_type(),
                    to = target_type.mime_type(),
                    "converting document for printer compatibility"
                );
                let converted = convert(document_bytes, source_type, target_type)?;
                return Ok((converted, target_type));
            }
        }

        // If nothing in the chain matches, try rasterisation as ultimate fallback
        if supported_formats.contains("image/jpeg") || supported_formats.contains("image/png") {
            info!(
                from = source_type.mime_type(),
                "rasterising document as ultimate fallback"
            );
            let rasterised = rasterise_to_png(document_bytes, source_type)?;
            return Ok((rasterised, DocumentType::Png));
        }

        // Nothing works
        Err(PresswerkError::UnsupportedDocument(format!(
            "Printer does not support {} and no conversion path available. \
             Supported formats: {:?}",
            source_type.mime_type(),
            supported_formats
        )))
    }
}

/// Get the conversion chain for a source document type.
/// Each entry is a format we can try converting to, in preference order.
fn conversion_chain(source: DocumentType) -> Vec<DocumentType> {
    match source {
        DocumentType::Pdf => vec![
            DocumentType::PostScript,
            DocumentType::Pcl,
            DocumentType::PwgRaster,
        ],
        DocumentType::PostScript => vec![
            DocumentType::Pdf,
            DocumentType::Pcl,
            DocumentType::PwgRaster,
        ],
        DocumentType::PlainText => vec![
            DocumentType::Pdf,
            DocumentType::PostScript,
        ],
        DocumentType::Jpeg | DocumentType::Png | DocumentType::Tiff => vec![
            DocumentType::Pdf,
            DocumentType::PwgRaster,
        ],
        _ => vec![DocumentType::Pdf],
    }
}

/// Perform the actual conversion between formats.
///
/// Currently implements stub conversions — real implementations would use:
/// - PDF → PostScript: Ghostscript bindings or pure-Rust PS generator
/// - PDF → Raster: pdf-render or similar crate
/// - Text → PDF: Already handled by PdfWriter::create_from_text
fn convert(
    document_bytes: &[u8],
    from: DocumentType,
    to: DocumentType,
) -> Result<Vec<u8>> {
    match (from, to) {
        // Text → PDF: use PdfWriter
        (DocumentType::PlainText, DocumentType::Pdf) => {
            let text = String::from_utf8_lossy(document_bytes);
            let writer = crate::pdf::writer::PdfWriter::a4();
            let pdf_bytes = writer.create_from_text(&text)?;
            Ok(pdf_bytes)
        }

        // Image → PDF: use PdfWriter
        (DocumentType::Jpeg | DocumentType::Png | DocumentType::Tiff, DocumentType::Pdf) => {
            let writer = crate::pdf::writer::PdfWriter::a4();
            let pdf_bytes = writer.create_from_image(document_bytes)?;
            Ok(pdf_bytes)
        }

        // PDF → PostScript: stub (would need Ghostscript or equivalent)
        (DocumentType::Pdf, DocumentType::PostScript) => {
            warn!("PDF → PostScript conversion not yet implemented — passing through as PDF");
            // TODO: Implement with ghostscript bindings or pure-Rust PS generation
            Err(PresswerkError::UnsupportedDocument(
                "PDF to PostScript conversion not yet available".into(),
            ))
        }

        // PDF → PCL: stub
        (DocumentType::Pdf, DocumentType::Pcl) => {
            warn!("PDF → PCL conversion not yet implemented");
            Err(PresswerkError::UnsupportedDocument(
                "PDF to PCL conversion not yet available".into(),
            ))
        }

        // PDF → PWG Raster: stub (would need PDF renderer)
        (DocumentType::Pdf, DocumentType::PwgRaster) => {
            warn!("PDF → PWG Raster conversion not yet implemented");
            Err(PresswerkError::UnsupportedDocument(
                "PDF to PWG Raster conversion not yet available".into(),
            ))
        }

        _ => Err(PresswerkError::UnsupportedDocument(format!(
            "No conversion path from {} to {}",
            from.mime_type(),
            to.mime_type(),
        ))),
    }
}

/// Rasterise a document to PNG as the ultimate fallback.
fn rasterise_to_png(
    document_bytes: &[u8],
    source: DocumentType,
) -> Result<Vec<u8>> {
    match source {
        // Images: just convert to PNG
        DocumentType::Jpeg | DocumentType::Tiff => {
            let processor = crate::image::processor::ImageProcessor::from_bytes(document_bytes)?;
            processor.to_png_bytes()
        }
        DocumentType::Png => Ok(document_bytes.to_vec()),

        // PDF: would need rendering — stub for now
        DocumentType::Pdf => {
            warn!("PDF rasterisation not yet implemented");
            Err(PresswerkError::UnsupportedDocument(
                "PDF rasterisation not yet available. Try printing a different file format.".into(),
            ))
        }

        _ => Err(PresswerkError::UnsupportedDocument(format!(
            "Cannot rasterise {}",
            source.mime_type()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_format_passes_through() {
        let bytes = b"test pdf data";
        let mut supported = HashSet::new();
        supported.insert("application/pdf".into());

        let (result, doc_type) =
            DocumentConverter::auto_convert(bytes, DocumentType::Pdf, &supported).expect("TODO: handle error");
        assert_eq!(result, bytes);
        assert_eq!(doc_type, DocumentType::Pdf);
    }

    #[test]
    fn empty_supported_passes_through() {
        let bytes = b"test data";
        let supported = HashSet::new();

        let (result, doc_type) =
            DocumentConverter::auto_convert(bytes, DocumentType::Pdf, &supported).expect("TODO: handle error");
        assert_eq!(result, bytes);
        assert_eq!(doc_type, DocumentType::Pdf);
    }

    #[test]
    fn text_to_pdf_conversion() {
        let chain = conversion_chain(DocumentType::PlainText);
        assert_eq!(chain[0], DocumentType::Pdf);
    }
}
