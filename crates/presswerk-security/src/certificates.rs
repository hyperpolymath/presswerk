// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// TLS certificate generation — ECDSA P-256 key pair for Presswerk's embedded
// print server mode.
//
// # Design note
//
// `ring` provides key generation and signing primitives but does **not**
// include an X.509 certificate builder.  This module generates the ECDSA P-256
// key pair (PKCS#8 DER) and exposes the raw material.  A full self-signed
// X.509 certificate requires an additional crate such as `rcgen` or a manual
// DER/ASN.1 encoder; that integration belongs in presswerk-print where TLS is
// actually configured.  The key pair produced here can be fed directly into
// `rcgen::Certificate::from_params()` or `rustls::PrivateKey`.

use presswerk_core::error::PresswerkError;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use tracing::{debug, instrument};

/// An ECDSA P-256 key pair suitable for TLS server authentication.
///
/// The private key is stored as a PKCS#8 v1 DER document.  The public key is
/// the uncompressed SEC1 encoding (0x04 || x || y, 65 bytes).
pub struct SelfSignedCert {
    /// PKCS#8 v1 DER-encoded private key (includes the public key).
    pkcs8_der: Vec<u8>,
    /// Uncompressed SEC1 public key bytes.
    public_key_der: Vec<u8>,
}

impl SelfSignedCert {
    /// Generate a fresh ECDSA P-256 key pair using the OS CSPRNG.
    ///
    /// This does **not** produce an X.509 certificate — only the raw key
    /// material.  See the module-level docs for how to turn this into a
    /// self-signed cert with `rcgen`.
    #[instrument]
    pub fn generate() -> Result<Self, PresswerkError> {
        let rng = SystemRandom::new();

        let pkcs8_document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
            .map_err(|e| PresswerkError::Certificate(format!("key generation failed: {e}")))?;

        let pkcs8_der = pkcs8_document.as_ref().to_vec();

        // Re-parse so we can extract the public key.
        let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &pkcs8_der, &rng)
            .map_err(|e| PresswerkError::Certificate(format!("key parsing failed: {e}")))?;

        let public_key_der = key_pair.public_key().as_ref().to_vec();

        debug!(
            pkcs8_len = pkcs8_der.len(),
            pubkey_len = public_key_der.len(),
            "ECDSA P-256 key pair generated"
        );

        Ok(Self {
            pkcs8_der,
            public_key_der,
        })
    }

    /// The PKCS#8 v1 DER-encoded private key.
    ///
    /// Pass this to `rustls::pki_types::PrivateKeyDer::Pkcs8` or to `rcgen`
    /// for certificate generation.
    pub fn private_key_pkcs8_der(&self) -> &[u8] {
        &self.pkcs8_der
    }

    /// The uncompressed SEC1 public key (65 bytes for P-256).
    pub fn public_key_der(&self) -> &[u8] {
        &self.public_key_der
    }

    /// Sign `message` with the private key (ECDSA P-256 + SHA-256, ASN.1
    /// DER-encoded signature).
    ///
    /// Useful for signing certificate requests or verifying that the key
    /// pair works end-to-end.
    pub fn sign(&self, message: &[u8]) -> Result<Vec<u8>, PresswerkError> {
        let rng = SystemRandom::new();

        let key_pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &self.pkcs8_der, &rng)
                .map_err(|e| PresswerkError::Certificate(format!("key load failed: {e}")))?;

        let sig = key_pair
            .sign(&rng, message)
            .map_err(|e| PresswerkError::Certificate(format!("signing failed: {e}")))?;

        Ok(sig.as_ref().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};

    #[test]
    fn generate_key_pair() {
        let cert = SelfSignedCert::generate().expect("key generation failed");

        // PKCS#8 for P-256 is typically ~138 bytes.
        assert!(
            cert.private_key_pkcs8_der().len() > 100,
            "PKCS#8 DER looks too short"
        );

        // Uncompressed P-256 public key: 1 (0x04) + 32 + 32 = 65 bytes.
        assert_eq!(cert.public_key_der().len(), 65);
        assert_eq!(cert.public_key_der()[0], 0x04, "must be uncompressed point");
    }

    #[test]
    fn sign_and_verify() {
        let cert = SelfSignedCert::generate().expect("key generation failed");
        let message = b"Presswerk TLS handshake test";

        let signature = cert.sign(message).expect("signing failed");

        // Verify with ring's public-key-only verifier.
        let public_key = UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, cert.public_key_der());

        public_key
            .verify(message, &signature)
            .expect("signature verification failed");
    }

    #[test]
    fn different_keys_each_time() {
        let a = SelfSignedCert::generate().expect("gen a");
        let b = SelfSignedCert::generate().expect("gen b");
        assert_ne!(
            a.private_key_pkcs8_der(),
            b.private_key_pkcs8_der(),
            "two generations must produce different keys"
        );
    }
}
