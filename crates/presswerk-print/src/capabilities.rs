// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Printer capability querying and print settings validation.
//
// Queries Get-Printer-Attributes to determine what the printer actually
// supports, then validates and auto-corrects user settings to match.

use std::collections::HashSet;

use tracing::{debug, info};

use presswerk_core::types::{DuplexMode, PaperSize, PrintSettings};

use crate::ipp_client::{IppClient, PrinterAttributes};

/// Parsed printer capabilities from IPP Get-Printer-Attributes.
#[derive(Debug, Clone)]
pub struct PrinterCapabilities {
    /// Supported media keywords (e.g. "iso_a4_210x297mm").
    pub media_supported: HashSet<String>,
    /// Supported sides keywords (e.g. "one-sided", "two-sided-long-edge").
    pub sides_supported: HashSet<String>,
    /// Whether the printer supports colour output.
    pub color_supported: bool,
    /// Supported document format MIME types.
    pub document_formats_supported: HashSet<String>,
    /// Maximum copies the printer supports (0 = unknown).
    pub max_copies: u32,
}

impl PrinterCapabilities {
    /// Parse capabilities from raw IPP printer attributes.
    pub fn from_attributes(attrs: &PrinterAttributes) -> Self {
        let media_supported = parse_set(attrs.get("media-supported"));
        let sides_supported = parse_set(attrs.get("sides-supported"));
        let document_formats_supported = parse_set(attrs.get("document-format-supported"));

        // Default to true (assume colour) when attribute is absent — same
        // "unknown = assume yes" pattern used for media and sides.
        let color_supported = attrs
            .get("color-supported")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);

        let max_copies = attrs
            .get("copies-supported")
            .and_then(|v| {
                // Usually "1-999" range format
                v.split('-').next_back().and_then(|n| n.trim().parse().ok())
            })
            .unwrap_or(0);

        Self {
            media_supported,
            sides_supported,
            color_supported,
            document_formats_supported,
            max_copies,
        }
    }

    /// Query a printer's capabilities via IPP.
    pub async fn query(client: &IppClient) -> Result<Self, presswerk_core::error::PresswerkError> {
        let attrs = client.get_printer_attributes().await?;
        Ok(Self::from_attributes(&attrs))
    }

    /// Whether the printer supports a given paper size.
    pub fn supports_media(&self, paper: &PaperSize) -> bool {
        if self.media_supported.is_empty() {
            return true; // unknown capabilities = assume yes
        }
        self.media_supported.contains(paper.ipp_media_keyword())
    }

    /// Whether the printer supports a given duplex mode.
    pub fn supports_sides(&self, duplex: &DuplexMode) -> bool {
        if self.sides_supported.is_empty() {
            return true;
        }
        self.sides_supported.contains(duplex.ipp_sides_keyword())
    }

    /// Whether the printer accepts a given document format.
    pub fn supports_format(&self, mime_type: &str) -> bool {
        if self.document_formats_supported.is_empty() {
            return true;
        }
        self.document_formats_supported.contains(mime_type)
    }
}

/// A notice about a setting that was auto-corrected.
#[derive(Debug, Clone)]
pub struct CorrectionNotice {
    /// Which setting was changed.
    pub field: String,
    /// What the user originally set.
    pub original: String,
    /// What it was changed to.
    pub corrected: String,
    /// Why it was changed.
    pub reason: String,
}

/// Result of validating print settings against printer capabilities.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether all settings are valid without corrections.
    pub valid: bool,
    /// Warnings about settings that might not work (but weren't changed).
    pub warnings: Vec<String>,
    /// Settings that were auto-corrected with explanations.
    pub corrections: Vec<CorrectionNotice>,
}

/// Validate and auto-correct print settings to match printer capabilities.
///
/// Returns the corrected settings along with a report of what was changed.
/// Always prefers keeping the user's choice — only corrects when the setting
/// would definitely fail.
pub fn auto_correct_settings(
    settings: &PrintSettings,
    caps: &PrinterCapabilities,
) -> (PrintSettings, ValidationResult) {
    let mut corrected = settings.clone();
    let mut result = ValidationResult {
        valid: true,
        warnings: Vec::new(),
        corrections: Vec::new(),
    };

    // Validate copies
    if caps.max_copies > 0 && settings.copies > caps.max_copies {
        result.corrections.push(CorrectionNotice {
            field: "Copies".into(),
            original: settings.copies.to_string(),
            corrected: caps.max_copies.to_string(),
            reason: format!(
                "This printer supports up to {} copies at a time.",
                caps.max_copies
            ),
        });
        corrected.copies = caps.max_copies;
        result.valid = false;
    }

    // Validate paper size
    if !caps.supports_media(&settings.paper_size) {
        // Try to find the closest supported size
        let fallback = find_closest_media(&settings.paper_size, &caps.media_supported);
        if let Some(fb) = fallback {
            result.corrections.push(CorrectionNotice {
                field: "Paper size".into(),
                original: format!("{:?}", settings.paper_size),
                corrected: format!("{:?}", fb),
                reason: format!(
                    "This printer doesn't support {:?}. We'll scale your document to fit.",
                    settings.paper_size
                ),
            });
            corrected.paper_size = fb;
            corrected.scale_to_fit = true;
            result.valid = false;
        } else {
            result.warnings.push(format!(
                "Paper size {:?} may not be supported by this printer.",
                settings.paper_size
            ));
        }
    }

    // Validate duplex
    if !caps.supports_sides(&settings.duplex) && settings.duplex != DuplexMode::Simplex {
        result.corrections.push(CorrectionNotice {
            field: "Duplex".into(),
            original: format!("{:?}", settings.duplex),
            corrected: "Simplex (one-sided)".into(),
            reason: "This printer only prints one-sided.".into(),
        });
        corrected.duplex = DuplexMode::Simplex;
        result.valid = false;
    }

    // Validate colour
    if settings.color && !caps.color_supported {
        result.corrections.push(CorrectionNotice {
            field: "Colour".into(),
            original: "Colour".into(),
            corrected: "Black & white".into(),
            reason: "This printer only prints in black and white.".into(),
        });
        corrected.color = false;
        result.valid = false;
    }

    if !result.corrections.is_empty() {
        info!(
            corrections = result.corrections.len(),
            "auto-corrected print settings for printer capabilities"
        );
    } else {
        debug!("print settings valid for printer capabilities");
    }

    (corrected, result)
}

/// Validate settings and return the result without correcting.
pub fn validate_settings(settings: &PrintSettings, caps: &PrinterCapabilities) -> ValidationResult {
    let (_, result) = auto_correct_settings(settings, caps);
    result
}

/// Try to find the closest standard paper size from the supported set.
fn find_closest_media(requested: &PaperSize, supported: &HashSet<String>) -> Option<PaperSize> {
    let (req_w, req_h) = requested.dimensions_mm();
    let req_area = req_w * req_h;

    let candidates = [
        PaperSize::A4,
        PaperSize::Letter,
        PaperSize::A5,
        PaperSize::A3,
        PaperSize::Legal,
        PaperSize::Tabloid,
    ];

    candidates
        .iter()
        .filter(|c| supported.contains(c.ipp_media_keyword()))
        .min_by_key(|c| {
            let (w, h) = c.dimensions_mm();
            let area = w * h;
            (area as i64 - req_area as i64).unsigned_abs()
        })
        .copied()
}

/// Parse a comma-separated or multi-valued IPP attribute into a HashSet.
fn parse_set(value: Option<&String>) -> HashSet<String> {
    match value {
        Some(v) => v
            .split([',', ';'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => HashSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_caps() -> PrinterCapabilities {
        let mut attrs = HashMap::new();
        attrs.insert(
            "media-supported".into(),
            "iso_a4_210x297mm, na_letter_8.5x11in".into(),
        );
        attrs.insert(
            "sides-supported".into(),
            "one-sided, two-sided-long-edge".into(),
        );
        attrs.insert("color-supported".into(), "true".into());
        attrs.insert("copies-supported".into(), "1-99".into());
        attrs.insert(
            "document-format-supported".into(),
            "application/pdf, image/jpeg".into(),
        );
        PrinterCapabilities::from_attributes(&attrs)
    }

    #[test]
    fn valid_settings_pass() {
        let caps = test_caps();
        let settings = PrintSettings::default();
        let (_, result) = auto_correct_settings(&settings, &caps);
        assert!(result.valid);
        assert!(result.corrections.is_empty());
    }

    #[test]
    fn duplex_corrected_on_simplex_printer() {
        let mut attrs = HashMap::new();
        attrs.insert("sides-supported".into(), "one-sided".into());
        let caps = PrinterCapabilities::from_attributes(&attrs);

        let mut settings = PrintSettings::default();
        settings.duplex = DuplexMode::LongEdge;

        let (corrected, result) = auto_correct_settings(&settings, &caps);
        assert!(!result.valid);
        assert_eq!(corrected.duplex, DuplexMode::Simplex);
        assert_eq!(result.corrections.len(), 1);
        assert_eq!(result.corrections[0].field, "Duplex");
    }

    #[test]
    fn color_corrected_on_bw_printer() {
        let mut attrs = HashMap::new();
        attrs.insert("color-supported".into(), "false".into());
        let caps = PrinterCapabilities::from_attributes(&attrs);

        let settings = PrintSettings::default(); // color = true by default

        let (corrected, result) = auto_correct_settings(&settings, &caps);
        assert!(!result.valid);
        assert!(!corrected.color);
    }

    #[test]
    fn copies_capped_at_printer_max() {
        let mut attrs = HashMap::new();
        attrs.insert("copies-supported".into(), "1-10".into());
        let caps = PrinterCapabilities::from_attributes(&attrs);

        let mut settings = PrintSettings::default();
        settings.copies = 50;

        let (corrected, result) = auto_correct_settings(&settings, &caps);
        assert_eq!(corrected.copies, 10);
        assert!(!result.valid);
    }

    #[test]
    fn unknown_caps_allows_everything() {
        let caps = PrinterCapabilities::from_attributes(&HashMap::new());
        let mut settings = PrintSettings::default();
        settings.duplex = DuplexMode::ShortEdge;
        settings.copies = 99;

        let (_, result) = auto_correct_settings(&settings, &caps);
        // No corrections when capabilities are unknown
        assert!(result.corrections.is_empty());
    }
}
