// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

#![forbid(unsafe_code)]
// presswerk-document — High-assurance document and image processing pipeline.
//
// This crate implements the "Print Doctor" logic, responsible for transforming
// raw user input (scans, photos, text) into standard-compliant PDF documents
// suitable for high-quality printing.
//
// CORE CAPABILITIES:
// 1. PDF Engineering: Direct manipulation of PDF structures using `lopdf`.
// 2. Visual Enhancement: Binarization and denoising for scanned documents.
// 3. Format Conversion: Stable conversion between Image and PDF formats.
// 4. Verified Metadata: Embedding proof-of-authenticity into document headers.

pub mod convert;
pub mod image;
pub mod pdf;
pub mod scan;

// CONVENIENCE: Primary interfaces for document transformation.
pub use image::processor::ImageProcessor;
pub use pdf::reader::PdfReader;
pub use pdf::writer::PdfWriter;
pub use scan::enhance::ScanEnhancer;

// OPTIONAL: OCR integration using `ocrs` (enabled via the "ocr" feature gate).
#[cfg(feature = "ocr")]
pub use scan::ocr::OcrEngine;
