// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

||| presswerk-security — Cryptographic foundation for high-assurance printing.
|||
||| This crate provides the secure storage and identity primitives required by 
||| the Presswerk router. It handles local data encryption, TLS certificate 
||| generation for secure mDNS/IPP communication, and tamper-evident audit logs.
|||
||| HIGH-ASSURANCE: All operations in this crate are designed to satisfy the
||| formal specifications defined in `src/abi/Encryption.idr`.

pub mod audit;
pub mod certificates;
pub mod integrity;
pub mod storage;

// PUBLIC API: Re-export core security primitives
pub use audit::AuditLog;
pub use certificates::SelfSignedCert;
pub use integrity::{hash_bytes, verify_hash};
pub use storage::EncryptedStorage;
