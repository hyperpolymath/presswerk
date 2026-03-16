// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Criterion benchmarks for document processing in the presswerk-document crate.
// Currently benchmarks the perspective correction pipeline on a small synthetic
// test image.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use image::{DynamicImage, GrayImage, Luma};

use presswerk_core::PaperSize;
use presswerk_document::ScanEnhancer;

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// Benchmark perspective correction on a 100x100 synthetic grayscale image.
///
/// Creates a small image with a white rectangle on a dark background (the
/// same pattern used in the `ScanEnhancer` unit tests) and runs the full
/// perspective correction pipeline. On such a small image the Hough
/// transform may not find a clean quadrilateral, exercising the early-exit
/// fallback path -- which is still the realistic hot path for images that
/// lack clear document borders.
fn bench_perspective_correction(c: &mut Criterion) {
    // Build a 100x100 synthetic image: dark background with a white
    // rectangle from (15, 15) to (85, 85).
    let (width, height) = (100u32, 100u32);
    let mut img = GrayImage::from_pixel(width, height, Luma([30u8]));
    for y in 15..85 {
        for x in 15..85 {
            img.put_pixel(x, y, Luma([240u8]));
        }
    }
    let dynamic = DynamicImage::ImageLuma8(img);

    c.bench_function("perspective_correction (100x100)", |b| {
        b.iter(|| {
            let enhancer = ScanEnhancer::from_dynamic(black_box(dynamic.clone()), PaperSize::A4);
            let result = enhancer.correct_perspective();
            black_box(result.into_dynamic());
        });
    });
}

criterion_group!(benches, bench_perspective_correction);
criterion_main!(benches);
