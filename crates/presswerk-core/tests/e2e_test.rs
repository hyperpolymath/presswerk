// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// End-to-end tests for presswerk-core pipeline.
// Tests the complete workflows: configuration, job creation, serialization.

use presswerk_core::{
    AppConfig, DocumentType, DuplexMode, ErrorClass, JobSource, JobStatus, Orientation, PageRange,
    PaperSize, PrintJob, PrintSettings,
};

#[test]
fn e2e_default_config_is_valid() {
    let config = AppConfig::default();
    assert!(config.server_port > 0);
    assert!(config.print_timeout_secs > 0);
    assert!(config.query_timeout_secs > 0);
    assert!(config.print_timeout_secs >= config.query_timeout_secs);
}

#[test]
fn e2e_config_with_custom_settings() {
    let config = AppConfig {
        default_paper_size: PaperSize::Letter,
        auto_start_server: true,
        server_port: 9631,
        server_require_tls: false,
        auto_accept_network_jobs: true,
        audit_enabled: false,
        encryption_enabled: false,
        print_timeout_secs: 120,
        query_timeout_secs: 30,
        easy_mode: false,
    };

    assert_eq!(config.default_paper_size, PaperSize::Letter);
    assert!(config.auto_start_server);
    assert_eq!(config.server_port, 9631);
    assert!(!config.server_require_tls);
}

#[test]
fn e2e_print_job_creation_and_update() {
    let mut job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "example.pdf".to_string(),
        "abc123def456".to_string(),
    );

    // Verify initial state
    assert_eq!(job.status, JobStatus::Pending);
    assert_eq!(job.retry_count, 0);
    assert_eq!(job.max_retries, 5);
    assert_eq!(job.document_name, "example.pdf");

    // Update status
    job.status = JobStatus::Processing;
    assert_eq!(job.status, JobStatus::Processing);

    // Record an error
    job.status = JobStatus::Failed;
    job.error_message = Some("Connection timeout".to_string());
    job.error_class = Some(ErrorClass::Transient);
    job.error_history.push("Connection timeout".to_string());

    assert_eq!(job.error_history.len(), 1);
    assert_eq!(job.error_class, Some(ErrorClass::Transient));
}

#[test]
fn e2e_print_settings_workflow() {
    let mut settings = PrintSettings::default();

    // Verify defaults
    assert_eq!(settings.copies, 1);
    assert_eq!(settings.paper_size, PaperSize::A4);
    assert_eq!(settings.duplex, DuplexMode::Simplex);
    assert_eq!(settings.orientation, Orientation::Portrait);
    assert!(settings.color);
    assert!(settings.scale_to_fit);

    // Customize
    settings.copies = 3;
    settings.paper_size = PaperSize::Letter;
    settings.duplex = DuplexMode::LongEdge;
    settings.orientation = Orientation::Landscape;
    settings.color = false;
    settings.page_range = Some(PageRange { start: 1, end: 10 });

    assert_eq!(settings.copies, 3);
    assert_eq!(settings.paper_size, PaperSize::Letter);
    assert_eq!(settings.duplex, DuplexMode::LongEdge);
    assert_eq!(settings.orientation, Orientation::Landscape);
    assert!(!settings.color);
    assert_eq!(settings.page_range, Some(PageRange { start: 1, end: 10 }));
}

#[test]
fn e2e_document_type_detection() {
    let types = vec![
        ("document.pdf", Some(DocumentType::Pdf)),
        ("photo.jpg", Some(DocumentType::Jpeg)),
        ("image.png", Some(DocumentType::Png)),
        ("scan.tiff", Some(DocumentType::Tiff)),
        ("letter.txt", Some(DocumentType::PlainText)),
        ("script.ps", Some(DocumentType::PostScript)),
        ("printer.pcl", Some(DocumentType::Pcl)),
        ("unknown.xyz", None),
        ("DOCUMENT.PDF", Some(DocumentType::Pdf)),
        ("Photo.JPG", Some(DocumentType::Jpeg)),
    ];

    for (filename, expected) in types {
        let ext = filename.split('.').last().unwrap_or("");
        let detected = DocumentType::from_extension(ext);
        assert_eq!(
            detected, expected,
            "Failed to detect type for: {}",
            filename
        );
    }
}

#[test]
fn e2e_native_delegate_detection() {
    let native_types = vec!["docx", "doc", "xlsx", "xls", "pptx", "ppt", "odt", "ods"];

    for ext in native_types {
        let detected = DocumentType::from_extension(ext);
        assert_eq!(
            detected,
            Some(DocumentType::NativeDelegate),
            "Failed to detect NativeDelegate for: {}",
            ext
        );
    }
}

#[test]
fn e2e_paper_size_ipp_mapping() {
    let sizes = vec![
        (PaperSize::A4, "iso_a4_210x297mm"),
        (PaperSize::A3, "iso_a3_297x420mm"),
        (PaperSize::A5, "iso_a5_148x210mm"),
        (PaperSize::Letter, "na_letter_8.5x11in"),
        (PaperSize::Legal, "na_legal_8.5x14in"),
        (PaperSize::Tabloid, "na_ledger_11x17in"),
    ];

    for (size, expected_keyword) in sizes {
        let keyword = size.ipp_media_keyword();
        assert_eq!(keyword, expected_keyword);
    }

    // Custom size
    let custom = PaperSize::Custom {
        width_mm: 210,
        height_mm: 297,
    };
    assert_eq!(custom.ipp_media_keyword(), "custom");
}

#[test]
fn e2e_duplex_mode_ipp_mapping() {
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
fn e2e_orientation_ipp_enum() {
    assert_eq!(Orientation::Portrait.ipp_enum_value(), 3);
    assert_eq!(Orientation::Landscape.ipp_enum_value(), 4);
    assert_eq!(Orientation::ReversePortrait.ipp_enum_value(), 5);
    assert_eq!(Orientation::ReverseLandscape.ipp_enum_value(), 6);
}

#[test]
fn e2e_mime_type_for_all_document_types() {
    let doc_types = vec![
        (DocumentType::Pdf, "application/pdf"),
        (DocumentType::Jpeg, "image/jpeg"),
        (DocumentType::Png, "image/png"),
        (DocumentType::Tiff, "image/tiff"),
        (DocumentType::PlainText, "text/plain"),
        (DocumentType::PostScript, "application/postscript"),
        (DocumentType::Pcl, "application/vnd.hp-pcl"),
        (DocumentType::PwgRaster, "image/pwg-raster"),
        (DocumentType::NativeDelegate, "application/octet-stream"),
    ];

    for (doc_type, expected_mime) in doc_types {
        let mime = doc_type.mime_type();
        assert_eq!(mime, expected_mime);
    }
}

#[test]
fn e2e_print_job_error_tracking() {
    let mut job = PrintJob::new(
        JobSource::Network {
            remote_addr: "192.168.1.100".parse().expect("valid IP"),
        },
        DocumentType::Pdf,
        "network-doc.pdf".to_string(),
        "network_hash_123".to_string(),
    );

    // Simulate retry cycle
    job.status = JobStatus::Processing;
    job.status = JobStatus::Failed;
    job.error_message = Some("Printer offline".to_string());
    job.error_class = Some(ErrorClass::Transient);
    job.error_history.push("Printer offline".to_string());
    job.retry_count = 1;

    job.status = JobStatus::RetryPending;

    job.status = JobStatus::Processing;
    job.status = JobStatus::Completed;

    assert_eq!(job.retry_count, 1);
    assert_eq!(job.error_history.len(), 1);
    assert_eq!(job.status, JobStatus::Completed);
}

#[test]
fn e2e_print_job_serialization_workflow() {
    let mut job = PrintJob::new(
        JobSource::Scan,
        DocumentType::Tiff,
        "scanned_document.tiff".to_string(),
        "scan_hash_xyz".to_string(),
    );

    job.settings.copies = 5;
    job.settings.duplex = DuplexMode::LongEdge;
    job.total_bytes = 50_000;
    job.bytes_sent = 25_000;

    // Serialize
    let json = serde_json::to_string(&job).expect("serialize failed");
    assert!(!json.is_empty());

    // Deserialize
    let restored: PrintJob = serde_json::from_str(&json).expect("deserialize failed");

    assert_eq!(restored.id, job.id);
    assert_eq!(restored.settings.copies, 5);
    assert_eq!(restored.settings.duplex, DuplexMode::LongEdge);
    assert_eq!(restored.total_bytes, 50_000);
    assert_eq!(restored.bytes_sent, 25_000);
}

#[test]
fn e2e_job_source_variants() {
    use std::net::IpAddr;

    let local_job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "local.pdf".to_string(),
        "local_hash".to_string(),
    );
    assert!(matches!(local_job.source, JobSource::Local));

    let scan_job = PrintJob::new(
        JobSource::Scan,
        DocumentType::Tiff,
        "scan.tiff".to_string(),
        "scan_hash".to_string(),
    );
    assert!(matches!(scan_job.source, JobSource::Scan));

    let text_job = PrintJob::new(
        JobSource::TextEditor,
        DocumentType::PlainText,
        "notes.txt".to_string(),
        "text_hash".to_string(),
    );
    assert!(matches!(text_job.source, JobSource::TextEditor));

    let ip: IpAddr = "192.168.1.50".parse().expect("valid IP");
    let network_job = PrintJob::new(
        JobSource::Network { remote_addr: ip },
        DocumentType::Pdf,
        "remote.pdf".to_string(),
        "remote_hash".to_string(),
    );
    assert!(matches!(network_job.source, JobSource::Network { .. }));
}

#[test]
fn e2e_job_status_transitions() {
    let mut job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "test.pdf".to_string(),
        "hash".to_string(),
    );

    // Valid transition: Pending -> Processing
    assert_eq!(job.status, JobStatus::Pending);
    job.status = JobStatus::Processing;
    assert_eq!(job.status, JobStatus::Processing);

    // Valid transition: Processing -> Completed
    job.status = JobStatus::Completed;
    assert_eq!(job.status, JobStatus::Completed);

    // Create another for failure path
    let mut job2 = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "test2.pdf".to_string(),
        "hash2".to_string(),
    );

    job2.status = JobStatus::Processing;
    job2.status = JobStatus::Failed;
    assert_eq!(job2.status, JobStatus::Failed);

    job2.status = JobStatus::RetryPending;
    assert_eq!(job2.status, JobStatus::RetryPending);

    // Create another for held state
    let mut job3 = PrintJob::new(
        JobSource::Network {
            remote_addr: "192.168.1.1".parse().unwrap(),
        },
        DocumentType::Pdf,
        "held.pdf".to_string(),
        "hash3".to_string(),
    );

    job3.status = JobStatus::Held;
    assert_eq!(job3.status, JobStatus::Held);
}

#[test]
fn e2e_custom_paper_size() {
    let custom = PaperSize::Custom {
        width_mm: 100,
        height_mm: 150,
    };

    let (w, h) = custom.dimensions_mm();
    assert_eq!(w, 100);
    assert_eq!(h, 150);
    assert_eq!(custom.ipp_media_keyword(), "custom");
}

#[test]
fn e2e_job_id_string_representation() {
    let job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "test.pdf".to_string(),
        "hash".to_string(),
    );

    let id_string = job.id.to_string();
    assert!(!id_string.is_empty());
    // UUID string should be 36 characters (8-4-4-4-12 with hyphens)
    assert_eq!(id_string.len(), 36);
}
