// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Core domain types for the Presswerk print router.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use uuid::Uuid;

/// Unique identifier for a print job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(pub Uuid);

impl JobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a print job originated from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobSource {
    /// User selected a file on this device.
    Local,
    /// Received over the network via the IPP print server.
    Network { remote_addr: IpAddr },
    /// Created from the built-in scanner.
    Scan,
    /// Created from the built-in text editor.
    TextEditor,
}

/// Lifecycle states of a print job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    /// Queued, waiting to be sent.
    Pending,
    /// Currently being transmitted to the printer.
    Processing,
    /// Successfully printed.
    Completed,
    /// Printing failed — see job error field.
    Failed,
    /// User cancelled the job.
    Cancelled,
    /// Held for user review (e.g. network-received jobs in preview mode).
    Held,
    /// Waiting for retry after a transient failure.
    RetryPending,
}

/// Supported input document types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentType {
    Pdf,
    Jpeg,
    Png,
    Tiff,
    PlainText,
    /// PostScript (auto-converted from PDF for legacy printers).
    PostScript,
    /// PCL (Printer Command Language, legacy support).
    Pcl,
    /// PWG Raster (rendered page images, ultimate fallback).
    PwgRaster,
    /// Format delegated to native OS print dialog (DOCX, XLS, etc.)
    NativeDelegate,
}

impl DocumentType {
    /// MIME type string for IPP Content-Type.
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Tiff => "image/tiff",
            Self::PlainText => "text/plain",
            Self::PostScript => "application/postscript",
            Self::Pcl => "application/vnd.hp-pcl",
            Self::PwgRaster => "image/pwg-raster",
            Self::NativeDelegate => "application/octet-stream",
        }
    }

    /// Infer document type from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "tif" | "tiff" => Some(Self::Tiff),
            "txt" => Some(Self::PlainText),
            "ps" | "eps" => Some(Self::PostScript),
            "pcl" => Some(Self::Pcl),
            "docx" | "doc" | "xlsx" | "xls" | "pptx" | "ppt" | "odt" | "ods" => {
                Some(Self::NativeDelegate)
            }
            _ => None,
        }
    }
}

/// Standard paper sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaperSize {
    A4,
    A3,
    A5,
    Letter,
    Legal,
    Tabloid,
    Custom { width_mm: u32, height_mm: u32 },
}

impl PaperSize {
    /// Dimensions in millimetres (width, height).
    pub fn dimensions_mm(&self) -> (u32, u32) {
        match self {
            Self::A4 => (210, 297),
            Self::A3 => (297, 420),
            Self::A5 => (148, 210),
            Self::Letter => (216, 279),
            Self::Legal => (216, 356),
            Self::Tabloid => (279, 432),
            Self::Custom {
                width_mm,
                height_mm,
            } => (*width_mm, *height_mm),
        }
    }

    /// IPP `media` keyword (RFC 8011 §5.2.13) for this paper size.
    pub fn ipp_media_keyword(&self) -> &'static str {
        match self {
            Self::A4 => "iso_a4_210x297mm",
            Self::A3 => "iso_a3_297x420mm",
            Self::A5 => "iso_a5_148x210mm",
            Self::Letter => "na_letter_8.5x11in",
            Self::Legal => "na_legal_8.5x14in",
            Self::Tabloid => "na_ledger_11x17in",
            Self::Custom { .. } => "custom", // custom sizes need special handling
        }
    }
}

/// Duplex printing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DuplexMode {
    Simplex,
    LongEdge,
    ShortEdge,
}

impl DuplexMode {
    /// IPP `sides` keyword (RFC 8011 §5.2.8).
    pub fn ipp_sides_keyword(&self) -> &'static str {
        match self {
            Self::Simplex => "one-sided",
            Self::LongEdge => "two-sided-long-edge",
            Self::ShortEdge => "two-sided-short-edge",
        }
    }
}

/// Page orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    Portrait,
    Landscape,
    ReversePortrait,
    ReverseLandscape,
}

impl Orientation {
    /// IPP `orientation-requested` enum value (RFC 8011 §5.2.10).
    pub fn ipp_enum_value(&self) -> i32 {
        match self {
            Self::Portrait => 3,
            Self::Landscape => 4,
            Self::ReversePortrait => 5,
            Self::ReverseLandscape => 6,
        }
    }
}

/// Print settings for a job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintSettings {
    pub copies: u32,
    pub paper_size: PaperSize,
    pub duplex: DuplexMode,
    pub orientation: Orientation,
    pub color: bool,
    pub page_range: Option<PageRange>,
    pub scale_to_fit: bool,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            copies: 1,
            paper_size: PaperSize::A4,
            duplex: DuplexMode::Simplex,
            orientation: Orientation::Portrait,
            color: true,
            page_range: None,
            scale_to_fit: true,
        }
    }
}

/// Page range specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRange {
    pub start: u32,
    pub end: u32,
}

// ---------------------------------------------------------------------------
// Form provenance — FFP v1.0.0 (issue #118, ruling D189)
//
// The wire types live in `crate::provenance` (ffp = "1.0").
// This module re-exports them and keeps the legacy `FormOrigin`/`FormProvenance`
// names as aliases so existing DB rows and tests can migrate gradually.
// New code should use `Ffp*` via `crate::provenance` or `crate::*`.
// ---------------------------------------------------------------------------

pub use crate::provenance::{
    FfpAppearances, FfpClassification, FfpDeclared, FfpForm, FfpRecord, FormOrigin, FormProvenance,
    ProvenanceConfidence,
};

/// What the print path does with a machine-filled form (issue #118).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FormProvenancePolicy {
    /// Do not inspect documents at all.
    Off,
    /// Inspect, record on the job, and write to the audit trail. Never hold.
    #[default]
    Record,
    /// As [`Self::Record`], plus machine-filled jobs are held for review.
    HoldForReview,
}

impl FormProvenancePolicy {
    /// Stable token for config files and logs.
    pub fn as_token(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Record => "Record",
            Self::HoldForReview => "HoldForReview",
        }
    }

    /// Inverse of [`Self::as_token`]; unrecognised tokens fall back to the
    /// default policy.
    pub fn from_token(token: &str) -> Self {
        match token {
            "Off" => Self::Off,
            "Record" => Self::Record,
            "HoldForReview" => Self::HoldForReview,
            _ => Self::default(),
        }
    }

    /// Short label for the settings UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Record => "Record in audit trail",
            Self::HoldForReview => "Record and hold for review",
        }
    }

    /// Whether documents should be inspected at all under this policy.
    pub fn inspects(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Classification of errors for retry logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorClass {
    /// Network blip, timeout, busy printer — safe to retry automatically.
    Transient,
    /// User must take action (add paper, close door, clear jam).
    UserAction,
    /// Permanent failure — unsupported format, invalid URI, etc.
    Permanent,
}

/// A complete print job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintJob {
    pub id: JobId,
    pub source: JobSource,
    pub status: JobStatus,
    pub document_type: DocumentType,
    pub document_name: String,
    /// SHA-256 hash of the original document bytes.
    pub document_hash: String,
    pub settings: PrintSettings,
    pub printer_uri: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub error_message: Option<String>,
    /// Number of retry attempts so far.
    pub retry_count: u32,
    /// Maximum retries before giving up.
    pub max_retries: u32,
    /// Classification of the last error (for retry logic).
    pub error_class: Option<ErrorClass>,
    /// History of error messages from each attempt.
    pub error_history: Vec<String>,
    /// Bytes successfully sent (for resume support in raw/LPR protocols).
    pub bytes_sent: u64,
    /// Total document size in bytes.
    pub total_bytes: u64,
    /// What the print path determined about the document's form values.
    ///
    /// Recorded for every job so audit and routing can tell a machine-filled
    /// application form from one completed by hand (issue #118, ruling D189).
    /// Defaults to "not inspected" for documents that are not PDFs.
    pub form_provenance: FormProvenance,
}

impl PrintJob {
    pub fn new(
        source: JobSource,
        document_type: DocumentType,
        document_name: String,
        document_hash: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: JobId::new(),
            source,
            status: JobStatus::Pending,
            document_type,
            document_name,
            document_hash,
            settings: PrintSettings::default(),
            printer_uri: None,
            created_at: now,
            updated_at: now,
            error_message: None,
            retry_count: 0,
            max_retries: 5,
            error_class: None,
            error_history: Vec::new(),
            bytes_sent: 0,
            total_bytes: 0,
            form_provenance: FormProvenance::default(),
        }
    }
}

/// A printer discovered on the local network via mDNS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPrinter {
    pub name: String,
    pub uri: String,
    pub ip: IpAddr,
    pub port: u16,
    pub supports_color: bool,
    pub supports_duplex: bool,
    pub supports_tls: bool,
    pub paper_sizes: Vec<PaperSize>,
    pub make_and_model: Option<String>,
    pub location: Option<String>,
    /// When this printer was last seen on the network.
    pub last_seen: DateTime<Utc>,
    /// Whether mDNS has gone silent for this printer (grace period active).
    pub stale: bool,
    /// Whether this printer was added manually (IP entry) rather than via mDNS.
    pub manually_added: bool,
}

/// Status of the embedded IPP print server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerStatus {
    Stopped,
    Starting,
    Running,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_id_unique() {
        let id1 = JobId::new();
        let id2 = JobId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_job_id_default() {
        let id1 = JobId::default();
        let id2 = JobId::default();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_job_id_display() {
        let id = JobId::new();
        let display_str = id.to_string();
        assert!(!display_str.is_empty());
        assert_eq!(display_str.len(), 36); // UUID format
    }

    #[test]
    fn test_document_type_mime_types() {
        assert_eq!(DocumentType::Pdf.mime_type(), "application/pdf");
        assert_eq!(DocumentType::Jpeg.mime_type(), "image/jpeg");
        assert_eq!(DocumentType::Png.mime_type(), "image/png");
        assert_eq!(DocumentType::Tiff.mime_type(), "image/tiff");
        assert_eq!(DocumentType::PlainText.mime_type(), "text/plain");
    }

    #[test]
    fn test_document_type_extensions() {
        assert_eq!(DocumentType::from_extension("pdf"), Some(DocumentType::Pdf));
        assert_eq!(
            DocumentType::from_extension("jpg"),
            Some(DocumentType::Jpeg)
        );
        assert_eq!(
            DocumentType::from_extension("jpeg"),
            Some(DocumentType::Jpeg)
        );
        assert_eq!(DocumentType::from_extension("png"), Some(DocumentType::Png));
        assert_eq!(
            DocumentType::from_extension("txt"),
            Some(DocumentType::PlainText)
        );
        assert_eq!(DocumentType::from_extension("unknown"), None);
    }

    #[test]
    fn test_document_type_case_insensitive() {
        assert_eq!(DocumentType::from_extension("PDF"), Some(DocumentType::Pdf));
        assert_eq!(
            DocumentType::from_extension("JPG"),
            Some(DocumentType::Jpeg)
        );
        assert_eq!(DocumentType::from_extension("Pdf"), Some(DocumentType::Pdf));
    }

    #[test]
    fn test_paper_size_a4_dimensions() {
        let (w, h) = PaperSize::A4.dimensions_mm();
        assert_eq!(w, 210);
        assert_eq!(h, 297);
    }

    #[test]
    fn test_paper_size_custom_dimensions() {
        let custom = PaperSize::Custom {
            width_mm: 100,
            height_mm: 150,
        };
        let (w, h) = custom.dimensions_mm();
        assert_eq!(w, 100);
        assert_eq!(h, 150);
    }

    #[test]
    fn test_paper_size_ipp_keywords() {
        assert_eq!(PaperSize::A4.ipp_media_keyword(), "iso_a4_210x297mm");
        assert_eq!(PaperSize::Letter.ipp_media_keyword(), "na_letter_8.5x11in");
        assert_eq!(PaperSize::Legal.ipp_media_keyword(), "na_legal_8.5x14in");
    }

    #[test]
    fn test_duplex_mode_keywords() {
        assert_eq!(DuplexMode::Simplex.ipp_sides_keyword(), "one-sided");
        assert_eq!(
            DuplexMode::LongEdge.ipp_sides_keyword(),
            "two-sided-long-edge"
        );
        assert_eq!(
            DuplexMode::ShortEdge.ipp_sides_keyword(),
            "two-sided-short-edge"
        );
    }

    #[test]
    fn test_orientation_enum_values() {
        assert_eq!(Orientation::Portrait.ipp_enum_value(), 3);
        assert_eq!(Orientation::Landscape.ipp_enum_value(), 4);
        assert_eq!(Orientation::ReversePortrait.ipp_enum_value(), 5);
        assert_eq!(Orientation::ReverseLandscape.ipp_enum_value(), 6);
    }

    #[test]
    fn test_print_settings_default() {
        let settings = PrintSettings::default();
        assert_eq!(settings.copies, 1);
        assert_eq!(settings.paper_size, PaperSize::A4);
        assert_eq!(settings.duplex, DuplexMode::Simplex);
        assert!(settings.color);
        assert!(settings.scale_to_fit);
    }

    #[test]
    fn test_print_job_new() {
        let job = PrintJob::new(
            JobSource::Local,
            DocumentType::Pdf,
            "test.pdf".to_string(),
            "hash123".to_string(),
        );

        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.document_name, "test.pdf");
        assert_eq!(job.document_hash, "hash123");
        assert_eq!(job.retry_count, 0);
        assert_eq!(job.max_retries, 5);
        assert_eq!(job.bytes_sent, 0);
        assert_eq!(job.total_bytes, 0);
    }

    #[test]
    fn test_print_job_timestamps() {
        let job = PrintJob::new(
            JobSource::Local,
            DocumentType::Pdf,
            "test.pdf".to_string(),
            "hash".to_string(),
        );

        let diff = job.updated_at.signed_duration_since(job.created_at);
        assert!(diff.num_seconds() <= 1);
    }

    #[test]
    fn test_page_range_ordering() {
        let range = PageRange { start: 1, end: 10 };
        assert!(range.start <= range.end);
    }

    #[test]
    fn test_job_source_network_variant() {
        let ip: std::net::IpAddr = "192.168.1.1".parse().expect("valid IP");
        let source = JobSource::Network { remote_addr: ip };

        match source {
            JobSource::Network { remote_addr } => {
                assert_eq!(remote_addr.to_string(), "192.168.1.1");
            }
            _ => panic!("Expected Network variant"),
        }
    }

    #[test]
    fn test_error_class_variants() {
        assert_eq!(ErrorClass::Transient, ErrorClass::Transient);
        assert_eq!(ErrorClass::UserAction, ErrorClass::UserAction);
        assert_eq!(ErrorClass::Permanent, ErrorClass::Permanent);
    }

    #[test]
    fn test_job_status_variants() {
        let mut job = PrintJob::new(
            JobSource::Local,
            DocumentType::Pdf,
            "test.pdf".to_string(),
            "hash".to_string(),
        );

        job.status = JobStatus::Processing;
        assert_eq!(job.status, JobStatus::Processing);

        job.status = JobStatus::Failed;
        assert_eq!(job.status, JobStatus::Failed);

        job.status = JobStatus::Completed;
        assert_eq!(job.status, JobStatus::Completed);
    }

    // -- Form provenance (issue #118) ---------------------------------------

    /// Build a machine-filled determination the way the document crate does (FFP v1.0).
    fn machine_filled() -> FormProvenance {
        FormProvenance {
            ffp: "1.0".to_string(),
            classification: crate::provenance::FfpClassification::MachineFilled,
            form: crate::provenance::FfpForm::Present,
            filled_fields: 12,
            total_fields: 14,
            appearances: crate::provenance::FfpAppearances::Incomplete,
            declared: Some(crate::provenance::FfpDeclared {
                filled_by: "machine".to_string(),
                tool: Some("blocky-writer 0.4.2".to_string()),
                tool_version: Some("0.4.2".to_string()),
                filled_at: None,
                appearances_generated: Some(false),
            }),
            evidence: vec![
                "FFP-E-DECL-MACHINE".to_string(),
                "FFP-E-NEED-APPEARANCES".to_string(),
                "FFP-E-AP-INCOMPLETE".to_string(),
            ],
        }
    }

    #[test]
    fn test_form_origin_tokens_roundtrip() {
        for origin in [
            FormOrigin::NotAForm,
            FormOrigin::Empty,
            FormOrigin::Human,
            FormOrigin::Machine,
            FormOrigin::Unknown,
        ] {
            let token = origin.as_token();
            assert_eq!(FormOrigin::from_token(token), origin);
            assert_eq!(token.to_string(), origin.to_string());
            assert!(!token.contains(' '));
        }
    }

    #[test]
    fn test_form_origin_unknown_token_is_unknown() {
        assert_eq!(FormOrigin::from_token(""), FormOrigin::Unknown);
        assert_eq!(FormOrigin::from_token("machine"), FormOrigin::Unknown);
        assert_eq!(FormOrigin::from_token("Machine"), FormOrigin::Machine);
    }

    #[test]
    fn test_form_origin_is_machine() {
        assert!(FormOrigin::Machine.is_machine());
        assert!(!FormOrigin::Human.is_machine());
        assert!(!FormOrigin::Unknown.is_machine());
        assert!(!FormOrigin::Empty.is_machine());
        assert!(!FormOrigin::NotAForm.is_machine());
    }

    #[test]
    fn test_form_provenance_default_is_not_inspected() {
        let provenance = FormProvenance::default();
        // Default is unreadable (form unknown, not a form)
        assert!(!provenance.is_form());
        assert_eq!(
            provenance.classification,
            crate::provenance::FfpClassification::Unreadable
        );
        assert_eq!(provenance.form, crate::provenance::FfpForm::Unknown);
        assert!(!provenance.is_machine_filled());
        assert_eq!(provenance.evidence, vec!["FFP-E-UNREADABLE".to_string()]);
    }

    #[test]
    fn test_print_job_defaults_to_uninspected_provenance() {
        let job = PrintJob::new(
            JobSource::Local,
            DocumentType::Pdf,
            "test.pdf".to_string(),
            "hash".to_string(),
        );
        assert_eq!(
            job.form_provenance.classification,
            crate::provenance::FfpClassification::Unreadable
        );
        assert_eq!(
            job.form_provenance.form,
            crate::provenance::FfpForm::Unknown
        );
    }

    #[test]
    fn test_form_provenance_summary_machine() {
        let summary = machine_filled().summary();
        assert!(summary.contains("machine-filled form"), "{summary}");
        assert!(summary.contains("blocky-writer 0.4.2"), "{summary}");
        assert!(summary.contains("12/14 fields filled"), "{summary}");
    }

    #[test]
    fn test_form_provenance_summary_not_a_form_omits_counts() {
        let provenance = FormProvenance {
            ffp: "1.0".to_string(),
            classification: crate::provenance::FfpClassification::NoForm,
            form: crate::provenance::FfpForm::Absent,
            filled_fields: 0,
            total_fields: 0,
            appearances: crate::provenance::FfpAppearances::NotApplicable,
            declared: None,
            evidence: vec!["FFP-E-NO-ACROFORM".to_string()],
        };
        assert_eq!(provenance.summary(), "not an interactive form");
    }

    #[test]
    fn test_form_provenance_declared_tool() {
        let mut provenance = machine_filled();
        assert_eq!(
            provenance.declared.as_ref().and_then(|d| d.tool.as_deref()),
            Some("blocky-writer 0.4.2")
        );

        // Declared tool is preferred
        provenance.declared = Some(crate::provenance::FfpDeclared {
            filled_by: "machine".to_string(),
            tool: Some("other-tool".to_string()),
            tool_version: None,
            filled_at: None,
            appearances_generated: None,
        });
        assert_eq!(
            provenance.declared.as_ref().and_then(|d| d.tool.as_deref()),
            Some("other-tool")
        );
    }

    #[test]
    fn test_form_provenance_audit_json_roundtrips() {
        let provenance = machine_filled();
        let json = provenance.audit_json();
        assert!(
            json.contains("\"classification\":\"machine-filled\""),
            "{json}"
        );
        assert!(json.contains("\"ffp\":\"1.0\""), "{json}");

        let restored: FormProvenance = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, provenance);
    }

    #[test]
    fn test_should_hold_only_for_positive_machine_determination() {
        let machine = machine_filled();
        assert!(machine.should_hold(FormProvenancePolicy::HoldForReview));
        assert!(!machine.should_hold(FormProvenancePolicy::Record));
        assert!(!machine.should_hold(FormProvenancePolicy::Off));

        // Undetermined (unreadable) must never stall a printout.
        let unknown = FormProvenance::default();
        assert!(!unknown.should_hold(FormProvenancePolicy::HoldForReview));

        // Filled-unknown with generated appearances must not be held (no hazard, no machine)
        let human = FormProvenance {
            ffp: "1.0".to_string(),
            classification: crate::provenance::FfpClassification::FilledUnknown,
            form: crate::provenance::FfpForm::Present,
            filled_fields: 2,
            total_fields: 2,
            appearances: crate::provenance::FfpAppearances::Generated,
            declared: None,
            evidence: vec![],
        };
        assert!(!human.should_hold(FormProvenancePolicy::HoldForReview));

        // But filled-unknown with incomplete appearances (hazard) SHOULD be held
        let hazard = FormProvenance {
            ffp: "1.0".to_string(),
            classification: crate::provenance::FfpClassification::FilledUnknown,
            form: crate::provenance::FfpForm::Present,
            filled_fields: 2,
            total_fields: 2,
            appearances: crate::provenance::FfpAppearances::Incomplete,
            declared: None,
            evidence: vec!["FFP-E-AP-INCOMPLETE".to_string()],
        };
        assert!(hazard.should_hold(FormProvenancePolicy::HoldForReview));
        // Machine-filled-suspected must be held
        let suspected = FormProvenance {
            ffp: "1.0".to_string(),
            classification: crate::provenance::FfpClassification::MachineFilledSuspected,
            form: crate::provenance::FfpForm::Present,
            filled_fields: 2,
            total_fields: 2,
            appearances: crate::provenance::FfpAppearances::Incomplete,
            declared: None,
            evidence: vec![
                "FFP-E-NEED-APPEARANCES".to_string(),
                "FFP-E-AP-INCOMPLETE".to_string(),
            ],
        };
        assert!(suspected.should_hold(FormProvenancePolicy::HoldForReview));
    }

    #[test]
    fn test_form_provenance_policy_defaults_and_tokens() {
        assert_eq!(
            FormProvenancePolicy::default(),
            FormProvenancePolicy::Record
        );
        for policy in [
            FormProvenancePolicy::Off,
            FormProvenancePolicy::Record,
            FormProvenancePolicy::HoldForReview,
        ] {
            assert_eq!(FormProvenancePolicy::from_token(policy.as_token()), policy);
            assert!(!policy.label().is_empty());
        }
        assert_eq!(
            FormProvenancePolicy::from_token("nonsense"),
            FormProvenancePolicy::Record
        );
    }

    #[test]
    fn test_form_provenance_policy_inspects() {
        assert!(!FormProvenancePolicy::Off.inspects());
        assert!(FormProvenancePolicy::Record.inspects());
        assert!(FormProvenancePolicy::HoldForReview.inspects());
    }

    #[test]
    fn test_provenance_confidence_is_ordered() {
        assert!(ProvenanceConfidence::None < ProvenanceConfidence::Speculative);
        assert!(ProvenanceConfidence::Speculative < ProvenanceConfidence::Probable);
        assert!(ProvenanceConfidence::Probable < ProvenanceConfidence::Strong);
        assert!(ProvenanceConfidence::Strong < ProvenanceConfidence::Explicit);
        assert_eq!(ProvenanceConfidence::Strong.as_token(), "Strong");
    }
}
