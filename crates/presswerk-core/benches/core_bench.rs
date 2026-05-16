// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Benchmarks for presswerk-core critical operations.
// Run with: cargo bench -p presswerk-core

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use presswerk_core::{AppConfig, DocumentType, JobSource, PrintJob, PrintSettings};

fn bench_config_creation(c: &mut Criterion) {
    c.bench_function("config_default", |b| {
        b.iter(|| {
            let _config = AppConfig::default();
        })
    });

    c.bench_function("config_custom", |b| {
        b.iter(|| {
            let _config = AppConfig {
                server_port: black_box(9631),
                auto_start_server: black_box(true),
                print_timeout_secs: black_box(120),
                ..Default::default()
            };
        })
    });
}

fn bench_config_serialization(c: &mut Criterion) {
    let config = AppConfig::default();

    c.bench_function("config_serialize_json", |b| {
        b.iter(|| {
            let _json = serde_json::to_string(&black_box(&config)).expect("serialize");
        })
    });

    let json = serde_json::to_string(&config).expect("serialize");

    c.bench_function("config_deserialize_json", |b| {
        b.iter(|| {
            let _config: AppConfig = serde_json::from_str(&black_box(&json)).expect("deserialize");
        })
    });
}

fn bench_print_job_creation(c: &mut Criterion) {
    c.bench_function("job_new_local", |b| {
        b.iter(|| {
            let _job = PrintJob::new(
                black_box(JobSource::Local),
                black_box(DocumentType::Pdf),
                black_box("test.pdf".to_string()),
                black_box("hash123".to_string()),
            );
        })
    });

    c.bench_function("job_new_network", |b| {
        let ip = "192.168.1.1".parse().expect("valid IP");
        b.iter(|| {
            let _job = PrintJob::new(
                black_box(JobSource::Network { remote_addr: ip }),
                black_box(DocumentType::Pdf),
                black_box("test.pdf".to_string()),
                black_box("hash123".to_string()),
            );
        })
    });

    c.bench_function("job_new_scan", |b| {
        b.iter(|| {
            let _job = PrintJob::new(
                black_box(JobSource::Scan),
                black_box(DocumentType::Tiff),
                black_box("scan.tiff".to_string()),
                black_box("hash456".to_string()),
            );
        })
    });
}

fn bench_job_serialization(c: &mut Criterion) {
    let job = PrintJob::new(
        JobSource::Local,
        DocumentType::Pdf,
        "test.pdf".to_string(),
        "hash123".to_string(),
    );

    c.bench_function("job_serialize_json", |b| {
        b.iter(|| {
            let _json = serde_json::to_string(&black_box(&job)).expect("serialize");
        })
    });

    let json = serde_json::to_string(&job).expect("serialize");

    c.bench_function("job_deserialize_json", |b| {
        b.iter(|| {
            let _job: PrintJob = serde_json::from_str(&black_box(&json)).expect("deserialize");
        })
    });
}

fn bench_print_settings(c: &mut Criterion) {
    c.bench_function("settings_default", |b| {
        b.iter(|| {
            let _settings = PrintSettings::default();
        })
    });

    c.bench_function("settings_custom", |b| {
        b.iter(|| {
            let mut _settings = PrintSettings::default();
            _settings.copies = black_box(5);
            _settings.color = black_box(false);
            _settings.scale_to_fit = black_box(false);
        })
    });

    let settings = PrintSettings::default();

    c.bench_function("settings_serialize_json", |b| {
        b.iter(|| {
            let _json = serde_json::to_string(&black_box(&settings)).expect("serialize");
        })
    });

    let json = serde_json::to_string(&settings).expect("serialize");

    c.bench_function("settings_deserialize_json", |b| {
        b.iter(|| {
            let _settings: PrintSettings =
                serde_json::from_str(&black_box(&json)).expect("deserialize");
        })
    });
}

fn bench_document_type_detection(c: &mut Criterion) {
    c.bench_function("doctype_from_pdf", |b| {
        b.iter(|| {
            let _dt = DocumentType::from_extension(black_box("pdf"));
        })
    });

    c.bench_function("doctype_from_jpg", |b| {
        b.iter(|| {
            let _dt = DocumentType::from_extension(black_box("jpg"));
        })
    });

    c.bench_function("doctype_from_unknown", |b| {
        b.iter(|| {
            let _dt = DocumentType::from_extension(black_box("xyz"));
        })
    });

    c.bench_function("doctype_mime_type", |b| {
        let dt = DocumentType::Pdf;
        b.iter(|| {
            let _mime = dt.mime_type();
        })
    });
}

fn bench_job_id_operations(c: &mut Criterion) {
    use presswerk_core::JobId;

    c.bench_function("job_id_new", |b| {
        b.iter(|| {
            let _id = JobId::new();
        })
    });

    let id = JobId::new();

    c.bench_function("job_id_display", |b| {
        b.iter(|| {
            let _s = black_box(&id).to_string();
        })
    });

    c.bench_function("job_id_clone", |b| {
        b.iter(|| {
            let _id = black_box(&id).clone();
        })
    });
}

fn bench_paper_dimensions(c: &mut Criterion) {
    use presswerk_core::PaperSize;

    c.bench_function("paper_a4_dimensions", |b| {
        let paper = PaperSize::A4;
        b.iter(|| {
            let (_w, _h) = paper.dimensions_mm();
        })
    });

    c.bench_function("paper_custom_dimensions", |b| {
        let paper = PaperSize::Custom {
            width_mm: 210,
            height_mm: 297,
        };
        b.iter(|| {
            let (_w, _h) = black_box(&paper).dimensions_mm();
        })
    });

    c.bench_function("paper_ipp_keyword_a4", |b| {
        let paper = PaperSize::A4;
        b.iter(|| {
            let _keyword = paper.ipp_media_keyword();
        })
    });
}

criterion_group!(
    benches,
    bench_config_creation,
    bench_config_serialization,
    bench_print_job_creation,
    bench_job_serialization,
    bench_print_settings,
    bench_document_type_detection,
    bench_job_id_operations,
    bench_paper_dimensions,
);

criterion_main!(benches);
