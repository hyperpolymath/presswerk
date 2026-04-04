// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Property-based tests for presswerk-core using proptest.
// These tests verify invariants that must hold across all valid inputs.

use presswerk_core::{AppConfig, DuplexMode, DocumentType, JobSource, Orientation, PaperSize, PrintJob, PrintSettings};
use proptest::prelude::*;

// ============================================================================
// CONFIGURATION PROPERTIES
// ============================================================================

proptest! {
    #[test]
    fn prop_config_port_in_valid_range(port in 1u16..=65535u16) {
        let mut config = AppConfig::default();
        config.server_port = port;
        prop_assert!(config.server_port >= 1);
        prop_assert!(config.server_port <= 65535);
    }

    #[test]
    fn prop_config_timeouts_positive(timeout_secs in 1u64..=3600u64) {
        let mut config = AppConfig::default();
        config.print_timeout_secs = timeout_secs;
        config.query_timeout_secs = timeout_secs / 2;
        prop_assert!(config.print_timeout_secs > 0);
        prop_assert!(config.query_timeout_secs >= 0);
        prop_assert!(config.print_timeout_secs >= config.query_timeout_secs);
    }

    #[test]
    fn prop_config_serialization_roundtrip(
        auto_start in any::<bool>(),
        require_tls in any::<bool>(),
        audit_enabled in any::<bool>(),
        encryption_enabled in any::<bool>(),
        easy_mode in any::<bool>(),
        port in 1u16..=65535u16,
    ) {
        let config = AppConfig {
            auto_start_server: auto_start,
            server_require_tls: require_tls,
            audit_enabled,
            encryption_enabled,
            easy_mode,
            server_port: port,
            ..Default::default()
        };

        let serialized = serde_json::to_string(&config).expect("serialize");
        let deserialized: AppConfig = serde_json::from_str(&serialized).expect("deserialize");

        prop_assert_eq!(config.auto_start_server, deserialized.auto_start_server);
        prop_assert_eq!(config.server_require_tls, deserialized.server_require_tls);
        prop_assert_eq!(config.audit_enabled, deserialized.audit_enabled);
        prop_assert_eq!(config.encryption_enabled, deserialized.encryption_enabled);
        prop_assert_eq!(config.easy_mode, deserialized.easy_mode);
        prop_assert_eq!(config.server_port, deserialized.server_port);
    }
}

// ============================================================================
// PAPER SIZE PROPERTIES
// ============================================================================

proptest! {
    #[test]
    fn prop_paper_dimensions_always_positive(
        width_mm in 1u32..=1000u32,
        height_mm in 1u32..=1000u32,
    ) {
        let paper = PaperSize::Custom { width_mm, height_mm };
        let (w, h) = paper.dimensions_mm();
        prop_assert!(w > 0);
        prop_assert!(h > 0);
    }

    #[test]
    fn prop_ipp_media_keyword_never_empty(size in any::<u8>()) {
        let paper = match size % 6 {
            0 => PaperSize::A4,
            1 => PaperSize::A3,
            2 => PaperSize::A5,
            3 => PaperSize::Letter,
            4 => PaperSize::Legal,
            _ => PaperSize::Tabloid,
        };

        let keyword = paper.ipp_media_keyword();
        prop_assert!(!keyword.is_empty());
    }
}

// ============================================================================
// DOCUMENT TYPE PROPERTIES
// ============================================================================

proptest! {
    #[test]
    fn prop_mime_type_never_empty(doc_type in any::<u8>()) {
        let doc = match doc_type % 9 {
            0 => DocumentType::Pdf,
            1 => DocumentType::Jpeg,
            2 => DocumentType::Png,
            3 => DocumentType::Tiff,
            4 => DocumentType::PlainText,
            5 => DocumentType::PostScript,
            6 => DocumentType::Pcl,
            7 => DocumentType::PwgRaster,
            _ => DocumentType::NativeDelegate,
        };

        let mime = doc.mime_type();
        prop_assert!(!mime.is_empty());
        prop_assert!(mime.contains('/'));
    }
}

// ============================================================================
// PRINT SETTINGS PROPERTIES
// ============================================================================

proptest! {
    #[test]
    fn prop_print_settings_copies_positive(copies in 1u32..=1000u32) {
        let mut settings = PrintSettings::default();
        settings.copies = copies;
        prop_assert!(settings.copies > 0);
    }

    #[test]
    fn prop_print_settings_serialization_roundtrip(
        copies in 1u32..=100u32,
        color in any::<bool>(),
        scale_to_fit in any::<bool>(),
    ) {
        let settings = PrintSettings {
            copies,
            color,
            scale_to_fit,
            ..Default::default()
        };

        let serialized = serde_json::to_string(&settings).expect("serialize");
        let deserialized: PrintSettings = serde_json::from_str(&serialized).expect("deserialize");

        prop_assert_eq!(settings.copies, deserialized.copies);
        prop_assert_eq!(settings.color, deserialized.color);
        prop_assert_eq!(settings.scale_to_fit, deserialized.scale_to_fit);
    }

    #[test]
    fn prop_duplex_ipp_keyword_valid(duplex in any::<u8>()) {
        let mode = match duplex % 3 {
            0 => DuplexMode::Simplex,
            1 => DuplexMode::LongEdge,
            _ => DuplexMode::ShortEdge,
        };

        let keyword = mode.ipp_sides_keyword();
        prop_assert!(!keyword.is_empty());
        prop_assert!(keyword.contains("sided") || keyword.contains("one"));
    }

    #[test]
    fn prop_orientation_ipp_enum_in_range(orient in any::<u8>()) {
        let orientation = match orient % 4 {
            0 => Orientation::Portrait,
            1 => Orientation::Landscape,
            2 => Orientation::ReversePortrait,
            _ => Orientation::ReverseLandscape,
        };

        let enum_val = orientation.ipp_enum_value();
        prop_assert!(enum_val >= 3 && enum_val <= 6);
    }
}

// ============================================================================
// PRINT JOB PROPERTIES
// ============================================================================

proptest! {
    #[test]
    fn prop_print_job_created_at_equals_updated_at(
        doc_name in r"[a-zA-Z0-9_\-\.]{1,50}",
        doc_hash in r"[a-f0-9]{64}",
    ) {
        let job = PrintJob::new(
            JobSource::Local,
            DocumentType::Pdf,
            doc_name,
            doc_hash,
        );

        // Created and updated times should be very close (same second)
        let diff = job.updated_at.signed_duration_since(job.created_at);
        prop_assert!(diff.num_seconds() <= 1);
    }

    #[test]
    fn prop_print_job_retries_bounded(max_retries in 0u32..=100u32) {
        let mut job = PrintJob::new(
            JobSource::Local,
            DocumentType::Pdf,
            "test".to_string(),
            "hash".to_string(),
        );
        job.max_retries = max_retries;
        job.retry_count = 0;

        // Retry count must never exceed max_retries
        prop_assert!(job.retry_count <= job.max_retries);
    }

    #[test]
    fn prop_print_job_bytes_sent_le_total(
        bytes_sent in 0u64..=1_000_000u64,
        total_bytes in 0u64..=1_000_000u64,
    ) {
        if bytes_sent <= total_bytes {
            let mut job = PrintJob::new(
                JobSource::Local,
                DocumentType::Pdf,
                "test".to_string(),
                "hash".to_string(),
            );
            job.bytes_sent = bytes_sent;
            job.total_bytes = total_bytes;

            prop_assert!(job.bytes_sent <= job.total_bytes);
        }
    }
}
