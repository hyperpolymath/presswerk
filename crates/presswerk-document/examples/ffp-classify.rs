// SPDX-License-Identifier: MPL-2.0
// Thin CLI for FFP conformance runner: `FFP_DETECTOR=... ffp-classify`.
// Usage: ffp-classify <path-to-pdf>  → prints canonical line to stdout.

use std::env;
use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: {} <pdf-path>", args[0]);
        process::exit(2);
    }
    let path = &args[1];
    let bytes = fs::read(path).unwrap_or_else(|e| {
        // Unreadable file is still a vector → classification unreadable
        eprintln!("read {}: {}", path, e);
        Vec::new()
    });
    // Use FFP v1.0 detector
    let record = presswerk_document::provenance::classify_pdf(&bytes);
    println!("{}", record.canonical_line());
    if std::env::var("FFP_PROBE_RECORD").as_deref() == Ok("1") {
        println!("{}", record.audit_json());
    }
}
