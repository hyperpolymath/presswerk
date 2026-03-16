// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// PDF writer — create new PDF documents from text or images using `printpdf` 0.8.
//
// printpdf 0.8 uses a data-oriented API: documents are built by constructing
// `PdfPage` structs containing `Vec<Op>` operation lists, then serialised via
// `PdfDocument::save()`.

use std::path::Path;

use presswerk_core::PaperSize;
use presswerk_core::error::PresswerkError;
use printpdf::{
    BuiltinFont, Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, PdfWarnMsg, Point, Pt, RawImage,
    RawImageData, RawImageFormat, TextItem, XObjectTransform,
};
use tracing::{debug, info, instrument};

/// Creates new PDF documents from text content or raster images.
///
/// Uses `printpdf` 0.8 for generation, producing standards-compliant PDF output
/// suitable for printing.
pub struct PdfWriter {
    /// Paper size for page creation.
    paper_size: PaperSize,
    /// Title metadata embedded in the PDF /Info dictionary.
    title: Option<String>,
}

impl PdfWriter {
    /// Create a new writer targeting the given paper size.
    pub fn new(paper_size: PaperSize) -> Self {
        Self {
            paper_size,
            title: None,
        }
    }

    /// Create a new writer defaulting to A4.
    pub fn a4() -> Self {
        Self::new(PaperSize::A4)
    }

    /// Set the paper size.
    pub fn set_paper_size(&mut self, paper_size: PaperSize) {
        self.paper_size = paper_size;
    }

    /// Set a title for the PDF metadata.
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = Some(title.into());
    }

    /// Paper dimensions in printpdf's Mm units.
    fn page_dimensions(&self) -> (Mm, Mm) {
        let (w_mm, h_mm) = self.paper_size.dimensions_mm();
        (Mm(w_mm as f32), Mm(h_mm as f32))
    }

    // -- Text to PDF ----------------------------------------------------------

    /// Create a PDF from plain text content.
    ///
    /// The text is laid out in a simple top-to-bottom flow using the built-in
    /// Helvetica font. Long lines are wrapped at an estimated character width
    /// and pages break automatically.
    #[instrument(skip(self, text), fields(text_len = text.len()))]
    pub fn create_from_text(&self, text: &str) -> Result<Vec<u8>, PresswerkError> {
        let (page_w, page_h) = self.page_dimensions();
        let title = self.title.as_deref().unwrap_or("Presswerk Document");

        info!(
            paper = ?self.paper_size,
            title,
            "Creating text PDF"
        );

        let font_size_pt: f32 = 11.0;
        let line_height_pt: f32 = 14.0;
        let margin_mm: f32 = 20.0;
        let margin_pt: f32 = Mm(margin_mm).into_pt().0;
        let usable_width_mm = page_w.0 - 2.0 * margin_mm;

        // Approximate characters per line based on Helvetica at 11pt.
        // Average Helvetica glyph width is roughly 0.50 * font_size in pt,
        // converted to mm (1pt = 0.3528mm).
        let avg_char_width_mm: f32 = 0.50 * font_size_pt * 0.3528;
        let max_chars_per_line = (usable_width_mm / avg_char_width_mm) as usize;

        let wrapped_lines = wrap_text(text, max_chars_per_line);
        let page_h_pt = page_h.into_pt().0;
        let usable_height_pt = page_h_pt - 2.0 * margin_pt;
        let lines_per_page = (usable_height_pt / line_height_pt) as usize;

        let mut doc = PdfDocument::new(title);
        let mut pages: Vec<PdfPage> = Vec::new();

        // Process lines in chunks of `lines_per_page`.
        let mut line_iter = wrapped_lines.iter().peekable();
        while line_iter.peek().is_some() {
            let mut ops: Vec<Op> = Vec::new();

            // Collect up to `lines_per_page` lines for this page.
            let mut line_idx: usize = 0;
            while line_idx < lines_per_page {
                let line = match line_iter.next() {
                    Some(l) => l,
                    None => break,
                };

                // Position: top-left of the page, moving downward.
                let y_pt = page_h_pt - margin_pt - (line_idx as f32 * line_height_pt);

                ops.push(Op::StartTextSection);
                ops.push(Op::SetTextCursor {
                    pos: Point {
                        x: Pt(margin_pt),
                        y: Pt(y_pt),
                    },
                });
                ops.push(Op::SetFontSizeBuiltinFont {
                    size: Pt(font_size_pt),
                    font: BuiltinFont::Helvetica,
                });
                ops.push(Op::WriteTextBuiltinFont {
                    items: vec![TextItem::Text(line.clone())],
                    font: BuiltinFont::Helvetica,
                });
                ops.push(Op::EndTextSection);

                line_idx += 1;
            }

            pages.push(PdfPage::new(page_w, page_h, ops));
        }

        // If there were no lines at all, add a single blank page.
        if pages.is_empty() {
            pages.push(PdfPage::new(page_w, page_h, Vec::new()));
        }

        doc.with_pages(pages);

        debug!(
            total_lines = wrapped_lines.len(),
            pages = doc.pages.len(),
            "Text layout complete"
        );

        let mut warnings: Vec<PdfWarnMsg> = Vec::new();
        let output = doc.save(&PdfSaveOptions::default(), &mut warnings);

        Ok(output)
    }

    // -- Image to PDF ---------------------------------------------------------

    /// Create a single-page PDF containing the given image.
    ///
    /// The image is scaled to fit within the page margins while preserving its
    /// aspect ratio.
    #[instrument(skip(self, image_bytes), fields(bytes_len = image_bytes.len()))]
    pub fn create_from_image(&self, image_bytes: &[u8]) -> Result<Vec<u8>, PresswerkError> {
        let (page_w, page_h) = self.page_dimensions();
        let title = self.title.as_deref().unwrap_or("Presswerk Image");

        info!(paper = ?self.paper_size, title, "Creating image PDF");

        // Decode the image to get its dimensions and pixel data.
        let dynamic_image = ::image::load_from_memory(image_bytes).map_err(|err| {
            PresswerkError::ImageError(format!("failed to decode image for PDF: {}", err))
        })?;

        let img_width = dynamic_image.width() as usize;
        let img_height = dynamic_image.height() as usize;

        // Convert to RGB8 for printpdf.
        let rgb_image = dynamic_image.to_rgb8();
        let raw = RawImage {
            pixels: RawImageData::U8(rgb_image.into_raw()),
            width: img_width,
            height: img_height,
            data_format: RawImageFormat::RGB8,
            tag: Vec::new(),
        };

        let mut doc = PdfDocument::new(title);
        let xobject_id = doc.add_image(&raw);

        // Compute transform to place the image on the page with margins.
        let margin_mm: f32 = 15.0;
        let usable_w_pt = Mm(page_w.0 - 2.0 * margin_mm).into_pt().0;
        let usable_h_pt = Mm(page_h.0 - 2.0 * margin_mm).into_pt().0;

        // Image native size at a default DPI of 150 (reasonable for print).
        let dpi: f32 = 150.0;
        let img_w_pt = img_width as f32 / dpi * 72.0;
        let img_h_pt = img_height as f32 / dpi * 72.0;

        // Scale to fit while preserving aspect ratio; do not upscale.
        let scale_x = usable_w_pt / img_w_pt;
        let scale_y = usable_h_pt / img_h_pt;
        let scale = scale_x.min(scale_y).min(1.0);

        let rendered_w_pt = img_w_pt * scale;
        let rendered_h_pt = img_h_pt * scale;

        // Centre the image on the page.
        let margin_pt = Mm(margin_mm).into_pt().0;
        let x_offset = margin_pt + (usable_w_pt - rendered_w_pt) / 2.0;
        let y_offset = margin_pt + (usable_h_pt - rendered_h_pt) / 2.0;

        let ops = vec![Op::UseXobject {
            id: xobject_id,
            transform: XObjectTransform {
                translate_x: Some(Pt(x_offset)),
                translate_y: Some(Pt(y_offset)),
                scale_x: Some(scale),
                scale_y: Some(scale),
                dpi: Some(dpi),
                rotate: None,
            },
        }];

        let page = PdfPage::new(page_w, page_h, ops);
        doc.with_pages(vec![page]);

        debug!(rendered_w_pt, rendered_h_pt, scale, "Image placed on page");

        let mut warnings: Vec<PdfWarnMsg> = Vec::new();
        let output = doc.save(&PdfSaveOptions::default(), &mut warnings);

        Ok(output)
    }

    // -- File output convenience ----------------------------------------------

    /// Create a text PDF and write it directly to a file.
    pub fn write_text_to_file(
        &self,
        text: &str,
        path: impl AsRef<Path>,
    ) -> Result<(), PresswerkError> {
        let bytes = self.create_from_text(text)?;
        std::fs::write(path.as_ref(), &bytes)?;
        info!("Wrote text PDF to {}", path.as_ref().display());
        Ok(())
    }

    /// Create an image PDF and write it directly to a file.
    pub fn write_image_to_file(
        &self,
        image_bytes: &[u8],
        path: impl AsRef<Path>,
    ) -> Result<(), PresswerkError> {
        let bytes = self.create_from_image(image_bytes)?;
        std::fs::write(path.as_ref(), &bytes)?;
        info!("Wrote image PDF to {}", path.as_ref().display());
        Ok(())
    }
}

// -- Text wrapping helper -----------------------------------------------------

/// Wrap a multi-line string so that no line exceeds `max_width` characters.
///
/// Splits on existing newlines first, then performs simple word-wrap within each
/// paragraph. Words longer than `max_width` are force-broken.
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let mut result = Vec::new();

    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            result.push(String::new());
            continue;
        }

        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            result.push(String::new());
            continue;
        }

        let mut current_line = String::with_capacity(max_width);

        for word in words {
            if word.len() > max_width {
                // Flush any accumulated line.
                if !current_line.is_empty() {
                    result.push(current_line.clone());
                    current_line.clear();
                }
                // Force-break the oversized word.
                let mut remaining = word;
                while remaining.len() > max_width {
                    let (chunk, rest) = remaining.split_at(max_width);
                    result.push(chunk.to_string());
                    remaining = rest;
                }
                if !remaining.is_empty() {
                    current_line.push_str(remaining);
                }
            } else if current_line.is_empty() {
                current_line.push_str(word);
            } else if current_line.len() + 1 + word.len() <= max_width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                result.push(current_line.clone());
                current_line.clear();
                current_line.push_str(word);
            }
        }

        if !current_line.is_empty() {
            result.push(current_line);
        }
    }

    result
}
