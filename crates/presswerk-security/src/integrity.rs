// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Document integrity — SHA-256 hashing for tamper detection.

use presswerk_core::error::PresswerkError;
use sha2::{Digest, Sha256};

/// Compute the SHA-256 hash of `data` and return it as a lowercase hex string.
///
/// Used throughout Presswerk to fingerprint documents before and after
/// encryption, audit logging, and print submission.
pub fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(result)
}

/// Verify that `data` matches the expected SHA-256 hex digest.
///
/// Returns `Ok(())` when the hash matches, or
/// `Err(PresswerkError::IntegrityMismatch)` with the expected and actual
/// values when it does not.
pub fn verify_hash(data: &[u8], expected_hex: &str) -> Result<(), PresswerkError> {
    let actual = hash_bytes(data);
    if actual == expected_hex {
        Ok(())
    } else {
        Err(PresswerkError::IntegrityMismatch {
            expected: expected_hex.to_owned(),
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-256 of the empty byte slice (well-known constant).
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn hash_empty_input() {
        assert_eq!(hash_bytes(b""), EMPTY_SHA256);
    }

    #[test]
    fn hash_known_value() {
        // SHA-256("hello") — verified against coreutils sha256sum.
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert_eq!(hash_bytes(b"hello"), expected);
    }

    #[test]
    fn verify_matching_hash() {
        let data = b"presswerk";
        let hex = hash_bytes(data);
        assert!(verify_hash(data, &hex).is_ok());
    }

    #[test]
    fn verify_mismatched_hash() {
        let result = verify_hash(b"a", "0000");
        assert!(result.is_err());
        match result.unwrap_err() {
            PresswerkError::IntegrityMismatch { expected, actual } => {
                assert_eq!(expected, "0000");
                assert_eq!(actual, hash_bytes(b"a"));
            }
            other => panic!("unexpected error variant: {other}"),
        }
    }
}
