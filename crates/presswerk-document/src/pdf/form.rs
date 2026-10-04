// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Deprecated wrapper — the FFP v1.0 detector lives in `crate::provenance`.
// This module re-exports it so `crate::pdf::form::classify_*` keeps compiling.

pub use crate::provenance::{classify_document, classify_for_print, classify_pdf, MAX_INSPECT_BYTES};
pub use crate::provenance::MAX_INSPECT_BYTES as DETECTOR_MAX_BYTES;
