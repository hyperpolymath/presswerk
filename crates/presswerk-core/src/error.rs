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
