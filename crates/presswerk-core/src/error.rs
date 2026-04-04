// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Unified error types for Presswerk.

use thiserror::Error;

/// Top-level error type for all Presswerk operations.
#[derive(Debug, Error)]
pub enum PresswerkError {
    // -- Print errors --
    #[error("printer discovery failed: {0}")]
    Discovery(String),

    #[error("IPP request failed: {0}")]
    IppRequest(String),

    #[error("print server error: {0}")]
    PrintServer(String),

    #[error("no printer selected")]
    NoPrinterSelected,

    // -- Document errors --
    #[error("unsupported document type: {0}")]
    UnsupportedDocument(String),

    #[error("PDF operation failed: {0}")]
    PdfError(String),

    #[error("image processing failed: {0}")]
    ImageError(String),

    #[error("OCR failed: {0}")]
    OcrError(String),

    // -- Security errors --
    #[error("encryption failed: {0}")]
    Encryption(String),

    #[error("decryption failed: {0}")]
    Decryption(String),

    #[error("integrity check failed: expected {expected}, got {actual}")]
    IntegrityMismatch { expected: String, actual: String },

    #[error("certificate generation failed: {0}")]
    Certificate(String),

    // -- Storage / persistence --
    #[error("database error: {0}")]
    Database(String),

    #[error("file I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    // -- Platform bridge --
    #[error("platform bridge error: {0}")]
    Bridge(String),

    #[error("feature not available on this platform")]
    PlatformUnavailable,
}

/// Alias used throughout the codebase.
pub type Result<T> = std::result::Result<T, PresswerkError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_error() {
        let err = PresswerkError::Discovery("No printers found".to_string());
        let msg = err.to_string();
        assert!(msg.contains("discovery failed"));
    }

    #[test]
    fn test_ipp_request_error() {
        let err = PresswerkError::IppRequest("Timeout".to_string());
        let msg = err.to_string();
        assert!(msg.contains("IPP request failed"));
    }

    #[test]
    fn test_no_printer_error() {
        let err = PresswerkError::NoPrinterSelected;
        let msg = err.to_string();
        assert_eq!(msg, "no printer selected");
    }

    #[test]
    fn test_unsupported_document_error() {
        let err = PresswerkError::UnsupportedDocument("TIFF".to_string());
        let msg = err.to_string();
        assert!(msg.contains("unsupported document type"));
    }

    #[test]
    fn test_integrity_mismatch() {
        let err = PresswerkError::IntegrityMismatch {
            expected: "abc123".to_string(),
            actual: "def456".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("integrity check failed"));
        assert!(msg.contains("abc123"));
        assert!(msg.contains("def456"));
    }

    #[test]
    fn test_encryption_error() {
        let err = PresswerkError::Encryption("Key derivation failed".to_string());
        let msg = err.to_string();
        assert!(msg.contains("encryption failed"));
    }

    #[test]
    fn test_decryption_error() {
        let err = PresswerkError::Decryption("Invalid ciphertext".to_string());
        let msg = err.to_string();
        assert!(msg.contains("decryption failed"));
    }

    #[test]
    fn test_io_error_from() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: PresswerkError = io_err.into();
        let msg = err.to_string();
        assert!(msg.contains("file not found"));
    }

    #[test]
    fn test_json_error_from() {
        let json_str = r#"{"invalid json"#;
        let result: std::result::Result<serde_json::Value, _> = serde_json::from_str(json_str);
        if let Err(json_err) = result {
            let err: PresswerkError = json_err.into();
            let msg = err.to_string();
            assert!(msg.contains("error"));
        }
    }

    #[test]
    fn test_platform_unavailable() {
        let err = PresswerkError::PlatformUnavailable;
        let msg = err.to_string();
        assert_eq!(msg, "feature not available on this platform");
    }

    #[test]
    fn test_result_alias() {
        let result: Result<i32> = Err(PresswerkError::NoPrinterSelected);
        assert!(result.is_err());

        let ok_result: Result<i32> = Ok(42);
        assert!(ok_result.is_ok());
    }

    #[test]
    fn test_error_debug_display() {
        let err = PresswerkError::Discovery("Test discovery failed".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Discovery"));
    }
}
