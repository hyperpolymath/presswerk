// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Presswerk — Core types and error definitions shared across all crates.

#![forbid(unsafe_code)]
pub mod config;
pub mod error;
pub mod human_errors;
pub mod types;

pub use config::AppConfig;
pub use error::PresswerkError;
pub use types::*;
