// SPDX-License-Identifier: MPL-2.0
// Binary target for FFP conformance: same as examples/ffp-classify.rs
// so `cargo build --bin ffp-classify` produces `target/debug/ffp-classify`.

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
        eprintln!("read {}: {}", path, e);
        Vec::new()
    });
    let record = presswerk_document::provenance::classify_pdf(&bytes);
    println!("{}", record.canonical_line());
    if std::env::var("FFP_PROBE_RECORD").as_deref() == Ok("1") {
        println!("{}", record.audit_json());
    }
}
