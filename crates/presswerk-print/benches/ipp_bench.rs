// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Criterion benchmarks for IPP request parsing, response building, and
// document content hashing in the presswerk-print crate.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use sha2::{Digest, Sha256};

use presswerk_print::ipp_server::{
    IPP_VERSION_MAJOR, IPP_VERSION_MINOR, IppResponseBuilder, OP_GET_PRINTER_ATTRIBUTES,
    OP_PRINT_JOB, STATUS_OK, TAG_END_OF_ATTRIBUTES, TAG_OPERATION_ATTRIBUTES,
    TAG_PRINTER_ATTRIBUTES, VALUE_TAG_CHARSET, VALUE_TAG_NAME, VALUE_TAG_NATURAL_LANGUAGE,
    parse_ipp_request,
};

// ---------------------------------------------------------------------------
// Helper: build a minimal IPP request (mirrors the test helper in ipp_server.rs)
// ---------------------------------------------------------------------------

/// Construct a binary IPP request identical to the test helper used in
/// `ipp_server::tests::build_test_ipp_request`.
fn build_test_ipp_request(
    operation_id: u16,
    request_id: u32,
    attributes: &[(u8, &str, &[u8])],
    document_data: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::new();
    // version 1.1
    buf.push(IPP_VERSION_MAJOR);
    buf.push(IPP_VERSION_MINOR);
    // operation-id
    buf.extend_from_slice(&operation_id.to_be_bytes());
    // request-id
    buf.extend_from_slice(&request_id.to_be_bytes());
    // operation attributes group
    buf.push(TAG_OPERATION_ATTRIBUTES);
    // Required: attributes-charset
    write_attr(&mut buf, VALUE_TAG_CHARSET, "attributes-charset", b"utf-8");
    // Required: attributes-natural-language
    write_attr(
        &mut buf,
        VALUE_TAG_NATURAL_LANGUAGE,
        "attributes-natural-language",
        b"en",
    );
    // Additional attributes
    for &(tag, name, value) in attributes {
        write_attr(&mut buf, tag, name, value);
    }
    // end-of-attributes
    buf.push(TAG_END_OF_ATTRIBUTES);
    // document data
    buf.extend_from_slice(document_data);
    buf
}

/// Write a single IPP attribute into a byte buffer.
fn write_attr(buf: &mut Vec<u8>, value_tag: u8, name: &str, value: &[u8]) {
    buf.push(value_tag);
    buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
    buf.extend_from_slice(name.as_bytes());
    buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
    buf.extend_from_slice(value);
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// Benchmark parsing a minimal IPP Get-Printer-Attributes request.
fn bench_parse_ipp_request(c: &mut Criterion) {
    let data = build_test_ipp_request(OP_GET_PRINTER_ATTRIBUTES, 42, &[], &[]);

    c.bench_function("parse_ipp_request (minimal)", |b| {
        b.iter(|| {
            let result = parse_ipp_request(black_box(&data));
            assert!(result.is_ok());
        });
    });

    // Also benchmark with a Print-Job that has extra attributes and a small
    // document payload, which exercises the document-data extraction path.
    let attrs = vec![(VALUE_TAG_NAME, "job-name", b"Benchmark Print Job" as &[u8])];
    let doc = vec![0xABu8; 4096]; // 4 KiB fake document
    let data_with_doc = build_test_ipp_request(OP_PRINT_JOB, 100, &attrs, &doc);

    c.bench_function("parse_ipp_request (4 KiB document)", |b| {
        b.iter(|| {
            let result = parse_ipp_request(black_box(&data_with_doc));
            assert!(result.is_ok());
        });
    });
}

/// Benchmark building an IPP response with operation and printer attributes.
fn bench_build_ipp_response(c: &mut Criterion) {
    c.bench_function("build_ipp_response (printer attrs)", |b| {
        b.iter(|| {
            let mut builder = IppResponseBuilder::new(black_box(STATUS_OK), black_box(1));
            builder.begin_group(TAG_OPERATION_ATTRIBUTES);
            builder.charset("attributes-charset", "utf-8");
            builder.natural_language("attributes-natural-language", "en");
            builder.begin_group(TAG_PRINTER_ATTRIBUTES);
            builder.name_attr("printer-name", "Presswerk Virtual Printer");
            builder.keyword("printer-state", "idle");
            builder.uri("printer-uri-supported", "ipp://localhost:631/ipp/print");
            builder.keyword("document-format-supported", "application/pdf");
            builder.keyword_additional("image/jpeg");
            builder.keyword_additional("image/png");
            builder.integer("printer-state", 3);
            builder.boolean("printer-is-accepting-jobs", true);
            let response = builder.build();
            black_box(response);
        });
    });
}

/// Benchmark SHA-256 hashing of a 1 MiB document (the content hash path
/// used when receiving print jobs).
fn bench_content_hash(c: &mut Criterion) {
    let data = vec![0x42u8; 1024 * 1024]; // 1 MiB

    c.bench_function("content_hash_sha256 (1 MiB)", |b| {
        b.iter(|| {
            let mut hasher = Sha256::new();
            hasher.update(black_box(&data));
            let result = hasher.finalize();
            black_box(result);
        });
    });
}

criterion_group!(
    benches,
    bench_parse_ipp_request,
    bench_build_ipp_response,
    bench_content_hash,
);
criterion_main!(benches);
