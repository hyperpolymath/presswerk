// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Encrypted storage — age (X25519 / scrypt) for encrypting and decrypting
// byte buffers.  Uses passphrase-based encryption via `age::scrypt` so that
// the user only needs to remember a single passphrase rather than managing
// raw key files.

use std::io::{Read, Write};

use age::secrecy::SecretString;
use presswerk_core::error::PresswerkError;
use tracing::{debug, instrument};

/// Passphrase-based encrypted storage backed by the `age` crate.
///
/// Each encrypt/decrypt call is stateless — the passphrase is held only for
/// the lifetime of the `EncryptedStorage` value so that callers can drop it
/// promptly after use.
pub struct EncryptedStorage {
    /// The user-supplied passphrase wrapped in a `SecretString` so that it
    /// is zeroised on drop.
    passphrase: SecretString,
}

impl EncryptedStorage {
    /// Create a new storage handle with the given passphrase.
    ///
    /// The passphrase is kept in memory (inside a `SecretString`) until this
    /// struct is dropped.
    pub fn new(passphrase: impl Into<String>) -> Self {
        Self {
            passphrase: SecretString::from(passphrase.into()),
        }
    }

    /// Encrypt `plaintext` and return the ciphertext as a `Vec<u8>`.
    ///
    /// The output is a complete age file (header + encrypted payload) that
    /// can be written directly to disk.
    #[instrument(skip_all, fields(plaintext_len = plaintext.len()))]
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, PresswerkError> {
        let encryptor = age::Encryptor::with_user_passphrase(self.passphrase.clone());
        let mut ciphertext = Vec::new();

        let mut writer = encryptor
            .wrap_output(&mut ciphertext)
            .map_err(|e| PresswerkError::Encryption(e.to_string()))?;

        writer
            .write_all(plaintext)
            .map_err(|e| PresswerkError::Encryption(e.to_string()))?;

        writer
            .finish()
            .map_err(|e| PresswerkError::Encryption(e.to_string()))?;

        debug!(ciphertext_len = ciphertext.len(), "encryption complete");
        Ok(ciphertext)
    }

    /// Decrypt `ciphertext` (a complete age file) and return the original
    /// plaintext bytes.
    #[instrument(skip_all, fields(ciphertext_len = ciphertext.len()))]
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, PresswerkError> {
        let decryptor = age::Decryptor::new(ciphertext)
            .map_err(|e| PresswerkError::Decryption(e.to_string()))?;

        let identity = age::scrypt::Identity::new(self.passphrase.clone());

        let mut reader = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .map_err(|e| PresswerkError::Decryption(e.to_string()))?;

        let mut plaintext = Vec::new();
        reader
            .read_to_end(&mut plaintext)
            .map_err(|e| PresswerkError::Decryption(e.to_string()))?;

        debug!(plaintext_len = plaintext.len(), "decryption complete");
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let storage = EncryptedStorage::new("correct-horse-battery-staple");
        let plaintext = b"Presswerk print job #42";

        let ciphertext = storage.encrypt(plaintext).expect("encrypt failed");
        assert_ne!(
            &ciphertext[..],
            plaintext,
            "ciphertext must differ from plaintext"
        );

        let decrypted = storage.decrypt(&ciphertext).expect("decrypt failed");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let storage_a = EncryptedStorage::new("passphrase-alpha");
        let storage_b = EncryptedStorage::new("passphrase-beta");

        let ciphertext = storage_a.encrypt(b"secret").expect("encrypt failed");
        let result = storage_b.decrypt(&ciphertext);

        assert!(
            result.is_err(),
            "decryption with wrong passphrase must fail"
        );
    }

    #[test]
    fn empty_plaintext() {
        let storage = EncryptedStorage::new("empty-test");
        let ciphertext = storage.encrypt(b"").expect("encrypt failed");
        let decrypted = storage.decrypt(&ciphertext).expect("decrypt failed");
        assert!(decrypted.is_empty());
    }
}
