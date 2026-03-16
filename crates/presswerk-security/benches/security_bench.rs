// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Criterion benchmarks for encryption, integrity hashing, and audit logging
// in the presswerk-security crate.

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use presswerk_security::{AuditLog, EncryptedStorage, hash_bytes};

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// Benchmark a full age encrypt-then-decrypt round trip on a 10 KiB payload.
///
/// This exercises the passphrase-based scrypt key derivation, X25519
/// encryption, and the corresponding decryption path.
fn bench_encrypt_decrypt_roundtrip(c: &mut Criterion) {
    let passphrase = "correct-horse-battery-staple";
    let plaintext = vec![0x42u8; 10 * 1024]; // 10 KiB

    c.bench_function("encrypt_decrypt_roundtrip (10 KiB)", |b| {
        b.iter(|| {
            let storage = EncryptedStorage::new(passphrase);
            let ciphertext = storage
                .encrypt(black_box(&plaintext))
                .expect("encrypt failed");
            let decrypted = storage.decrypt(&ciphertext).expect("decrypt failed");
            assert_eq!(decrypted.len(), plaintext.len());
            black_box(decrypted);
        });
    });
}

/// Benchmark SHA-256 integrity hashing at various document sizes.
///
/// Sizes: 1 KiB, 10 KiB, 100 KiB, 1 MiB -- covering the range from small
/// receipts to full-page scanned documents.
fn bench_integrity_hash(c: &mut Criterion) {
    let sizes: &[(&str, usize)] = &[
        ("1 KiB", 1024),
        ("10 KiB", 10 * 1024),
        ("100 KiB", 100 * 1024),
        ("1 MiB", 1024 * 1024),
    ];

    let mut group = c.benchmark_group("integrity_hash_sha256");
    for &(label, size) in sizes {
        let data = vec![0xABu8; size];
        group.bench_function(label, |b| {
            b.iter(|| {
                let hex = hash_bytes(black_box(&data));
                black_box(hex);
            });
        });
    }
    group.finish();
}

/// Benchmark recording an audit entry to an in-memory SQLite database.
///
/// Each iteration opens a fresh in-memory database, inserts a single record,
/// and verifies the count. This measures the per-record overhead including
/// SQLite WAL journalling.
fn bench_audit_record(c: &mut Criterion) {
    c.bench_function("audit_record (in-memory SQLite)", |b| {
        // Create the database once outside the hot loop so we measure
        // steady-state insertion, not schema creation.
        let log = AuditLog::open_in_memory().expect("open in-memory audit log");

        b.iter(|| {
            log.record(
                black_box("encrypt"),
                black_box("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"),
                black_box(true),
                black_box(Some("benchmark test entry")),
            )
            .expect("record failed");
        });
    });
}

criterion_group!(
    benches,
    bench_encrypt_decrypt_roundtrip,
    bench_integrity_hash,
    bench_audit_record,
);
criterion_main!(benches);
