// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// PDF module — reading, merging, splitting, rotating, and creating PDFs.

pub mod reader;
pub mod writer;

pub use reader::PdfReader;
pub use writer::PdfWriter;
