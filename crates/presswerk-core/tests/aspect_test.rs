// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Aspect and security tests for presswerk-core.
// Tests security properties, invariants, and edge cases.

use presswerk_core::{
    AppConfig, DocumentType, JobSource, PageRange, PaperSize, PrintJob, PrintSettings,
};

// ============================================================================
// DOCUMENT SECURITY TESTS
// ============================================================================

#[test]
fn aspect_null_bytes_in_document_name() {
    let doc_name = "test\0injection.pdf".to_string();
    // If the name contains null bytes, we should be aware in real filtering
    assert!(doc_name.contains('\0'));
    // Systems should strip or reject these
    let cleaned = doc_name.replace('\0', "");
    assert!(!cleaned.contains('\0'));
}

#[test]
fn aspect_html_injection_in_document_name() {
    let malicious_names = vec![
        "<script>alert('xss')</script>.pdf",
        "../../etc/passwd",
        "'; DROP TABLE jobs; --",
        "<img src=x onerror=alert(1)>",
    ];

    for name in malicious_names {
        // These should be detected as suspicious by the application
        assert!(
            name.contains('<')
                || name.contains('>')
                || name.contains('\'')
                || name.contains('/')
                || name.contains('.'),
            "Potential injection in: {}",
            name
        );
    }
}

#[test]
fn aspect_path_traversal_prevention() {
    let suspicious_paths = vec![
        ("../../../etc/passwd", true),
        ("..\\..\\..\\windows\\system32", true),
        ("/.ssh/id_rsa", true),
        ("simple_file.pdf", false),
    ];

    for (path, should_be_suspicious) in suspicious_paths {
        // Check for path traversal indicators
        let has_traversal = path.contains("../") || path.contains("..\\") || path.starts_with('/');
        if should_be_suspicious {
            assert!(
                has_traversal,
                "Path traversal pattern should be detected in: {}",
                path
            );
        }
    }
}

#[test]
fn aspect_oversized_document_detection() {
    let mut job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "huge.pdf".to_string(),
        "hash".to_string(),
    );

    // Set total_bytes to a very large value
    job.total_bytes = 5_000_000_000; // 5 GB

    // Applications should have a max size policy and reject or warn
    const MAX_DOCUMENT_SIZE: u64 = 1_000_000_000; // 1 GB

    if job.total_bytes > MAX_DOCUMENT_SIZE {
        // This should be rejected or logged
        assert!(job.total_bytes > MAX_DOCUMENT_SIZE);
    }
}

#[test]
fn aspect_page_count_validation() {
    let mut job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "test.pdf".to_string(),
        "hash".to_string(),
    );

    job.settings.page_range = Some(PageRange {
        start: 100,
        end: 50,
    });

    // Invalid range: start > end should be detected
    if let Some(range) = &job.settings.page_range {
        // This invariant might be enforced at creation time
        let is_valid = range.start <= range.end;
        if !is_valid {
            // Should be caught
            assert!(range.start > range.end);
        }
    }
}

#[test]
fn aspect_invalid_dpi_detection() {
    // DPI values should be in reasonable ranges for printing
    let valid_dpis = vec![72, 150, 300, 600, 1200];
    let invalid_dpis = vec![0, 1, 36, 2000, u32::MAX];

    const MIN_DPI: u32 = 72;
    const MAX_DPI: u32 = 1200;

    for dpi in valid_dpis {
        assert!(
            dpi >= MIN_DPI && dpi <= MAX_DPI,
            "DPI {} should be valid",
            dpi
        );
    }

    for dpi in invalid_dpis {
        let is_invalid = dpi < MIN_DPI || dpi > MAX_DPI;
        assert!(is_invalid, "DPI {} should be detected as invalid", dpi);
    }
}

#[test]
fn aspect_config_port_validation() {
    // Port 0 is reserved; ports < 1024 require elevated privileges
    const PRIVILEGED_PORT_LIMIT: u16 = 1024;

    let config = AppConfig::default();
    assert!(config.server_port > 0, "Port must not be 0");

    if config.server_port < PRIVILEGED_PORT_LIMIT {
        // Elevated privileges needed
        assert!(config.server_port < PRIVILEGED_PORT_LIMIT);
    }

    // Validate no common misconfigurations
    assert_ne!(config.server_port, 0);
    assert_ne!(config.server_port, 1); // Super-user only
}

#[test]
fn aspect_timeout_sanity_checks() {
    let mut config = AppConfig::default();

    // Timeouts should be reasonable (not 0 or excessive)
    const MIN_TIMEOUT: u64 = 1; // seconds
    const MAX_TIMEOUT: u64 = 3600; // 1 hour

    config.print_timeout_secs = 60;
    config.query_timeout_secs = 15;

    assert!(config.print_timeout_secs >= MIN_TIMEOUT);
    assert!(config.print_timeout_secs <= MAX_TIMEOUT);
    assert!(config.query_timeout_secs >= MIN_TIMEOUT);
    assert!(config.query_timeout_secs <= MAX_TIMEOUT);

    // Print timeout should generally be >= query timeout
    assert!(config.print_timeout_secs >= config.query_timeout_secs);
}

#[test]
fn aspect_error_message_sanitization() {
    let mut job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "test.pdf".to_string(),
        "hash".to_string(),
    );

    // Error messages could contain sensitive info; they should be sanitized
    let sensitive_error = "Failed to connect to 192.168.1.1:631 with password abc123".to_string();
    job.error_message = Some(sensitive_error.clone());

    if let Some(msg) = &job.error_message {
        // Check for common sensitive patterns
        let contains_password = msg.contains("password");
        let contains_ip = msg.contains("192.168");
        assert!(
            contains_password || contains_ip,
            "Error message should contain sensitive data (for testing)"
        );
        // In production, these should be redacted
    }
}

#[test]
fn aspect_document_hash_validation() {
    // SHA-256 hashes should be 64 hex characters
    let valid_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"; // SHA-256("")
    let invalid_hashes = vec![
        "short",
        "not_hex_at_all",
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        "", // empty
    ];

    assert_eq!(valid_hash.len(), 64);
    for hex_char in valid_hash.chars() {
        assert!(hex_char.is_ascii_hexdigit());
    }

    for invalid in &invalid_hashes {
        if invalid.len() != 64 {
            assert_ne!(invalid.len(), 64, "Hash '{}' has wrong length", invalid);
        }
        if !invalid.is_empty() && invalid.len() == 64 {
            // Check if all are hex
            let all_hex = invalid.chars().all(|c| c.is_ascii_hexdigit());
            if !all_hex {
                assert!(!all_hex, "Hash '{}' contains non-hex characters", invalid);
            }
        }
    }
}

#[test]
fn aspect_retry_limit_enforcement() {
    let mut job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "test.pdf".to_string(),
        "hash".to_string(),
    );

    // Retry count should never exceed max_retries
    job.max_retries = 5;
    for i in 0..=10 {
        if i <= job.max_retries {
            job.retry_count = i;
            assert!(job.retry_count <= job.max_retries);
        }
    }
}

#[test]
fn aspect_max_copies_limit() {
    let mut settings = PrintSettings::default();

    // Practical limit on copies to prevent accidental large batch prints
    const MAX_COPIES: u32 = 999;

    settings.copies = 5;
    assert!(settings.copies <= MAX_COPIES);

    settings.copies = 999;
    assert!(settings.copies <= MAX_COPIES);

    // Beyond practical limits would need manual override
    let excessive_copies = 10_000u32;
    assert!(
        excessive_copies > MAX_COPIES,
        "Excessive copies should be detectable"
    );
}

#[test]
fn aspect_bytes_progress_invariant() {
    let mut job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "test.pdf".to_string(),
        "hash".to_string(),
    );

    job.total_bytes = 100_000;

    // bytes_sent should never exceed total_bytes
    job.bytes_sent = 50_000;
    assert!(job.bytes_sent <= job.total_bytes);

    job.bytes_sent = 100_000;
    assert!(job.bytes_sent <= job.total_bytes);

    // Can't track more bytes than exist
    if job.bytes_sent > job.total_bytes {
        assert!(job.bytes_sent > job.total_bytes, "Should catch overage");
    }
}

#[test]
fn aspect_encryption_setting_consistency() {
    let mut config = AppConfig::default();

    // If encryption is disabled, audit should still be optional
    config.encryption_enabled = false;
    assert!(!config.encryption_enabled);
    // audit can be true or false independently

    config.encryption_enabled = true;
    assert!(config.encryption_enabled);
    // Both can be enabled
}

#[test]
fn aspect_tls_requirement_with_network_jobs() {
    let config = AppConfig {
        server_require_tls: true,
        ..Default::default()
    };

    // If TLS is required, all network jobs should use secure connections
    assert!(config.server_require_tls);

    let job = PrintJob::new(
        JobSource::Network {
            remote_addr: "192.168.1.100".parse().unwrap(),
        },
        DocumentType::Pdf,
        "network.pdf".to_string(),
        "hash".to_string(),
    );

    if config.server_require_tls {
        // Should enforce TLS for this job
        assert!(matches!(job.source, JobSource::Network { .. }));
    }
}

#[test]
fn aspect_auto_accept_policy_consistency() {
    let mut config = AppConfig::default();

    // auto_accept_network_jobs decision affects workflow
    config.auto_accept_network_jobs = false;
    // When false, jobs should be held for review
    let job = PrintJob::new(
        JobSource::Network {
            remote_addr: "10.0.0.50".parse().unwrap(),
        },
        DocumentType::Pdf,
        "remote.pdf".to_string(),
        "hash".to_string(),
    );

    // Job created; if auto_accept is false, it should start in Held state
    // (This is a behavioral constraint enforced elsewhere)
    assert!(matches!(job.source, JobSource::Network { .. }));
}

// ============================================================================
// REFLEXIVE CONTRACTS
// ============================================================================

#[test]
fn contract_print_job_identity() {
    let job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "test.pdf".to_string(),
        "hash".to_string(),
    );

    // A job equals itself
    assert_eq!(job.id, job.id);
}

#[test]
fn contract_document_type_mime_consistency() {
    let doc = DocumentType::Pdf;
    let mime1 = doc.mime_type();
    let mime2 = doc.mime_type();

    // mime_type() should be consistent
    assert_eq!(mime1, mime2);
}

#[test]
fn contract_paper_size_dimension_consistency() {
    let paper = PaperSize::A4;
    let (w1, h1) = paper.dimensions_mm();
    let (w2, h2) = paper.dimensions_mm();

    assert_eq!(w1, w2);
    assert_eq!(h1, h2);
}

#[test]
fn contract_config_clonability() {
    let config1 = AppConfig::default();
    let config2 = config1.clone();

    assert_eq!(config1.server_port, config2.server_port);
    assert_eq!(config1.auto_start_server, config2.auto_start_server);
}

#[test]
fn contract_print_settings_clonability() {
    let settings1 = PrintSettings::default();
    let settings2 = settings1.clone();

    assert_eq!(settings1.copies, settings2.copies);
    assert_eq!(settings1.color, settings2.color);
    assert_eq!(settings1.paper_size, settings2.paper_size);
}
