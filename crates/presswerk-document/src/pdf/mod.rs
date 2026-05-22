// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// PDF module — reading, merging, splitting, rotating, and creating PDFs.

pub mod reader;
pub mod writer;

pub use reader::PdfReader;
pub use writer::PdfWriter;
