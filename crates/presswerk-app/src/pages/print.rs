// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Print page — pick a file, select printer, configure settings, print.
// Settings are now fully wired: copies, color, duplex, paper size, orientation
// are all sent as IPP attributes to the printer.

use dioxus::prelude::*;

use presswerk_core::types::{DocumentType, DuplexMode, Orientation, PaperSize, PrintSettings};

use crate::services::app_services::AppServices;
use crate::state::AppState;

/// Print progress stages shown to the user.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum PrintStage {
    Idle,
    Preparing,
    CheckingPrinter,
    Sending,
    Confirming,
    Complete,
    Failed,
    Retrying,
}

impl PrintStage {
    fn message(&self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Preparing => "Preparing your document...",
            Self::CheckingPrinter => "Checking the printer is ready...",
            Self::Sending => "Sending to printer...",
            Self::Confirming => "Confirming with the printer...",
            Self::Complete => "Done! Your document is printing.",
            Self::Failed => "Something went wrong.",
            Self::Retrying => "Trying again...",
        }
    }

    fn color(&self) -> &'static str {
        match self {
            Self::Complete => "#155724",
            Self::Failed => "#721c24",
            Self::Retrying => "#856404",
            _ => "#007aff",
        }
    }

    fn bg(&self) -> &'static str {
        match self {
            Self::Complete => "#d4edda",
            Self::Failed => "#f8d7da",
            Self::Retrying => "#fff3cd",
            _ => "#e7f3ff",
        }
    }
}

#[component]
pub fn Print() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();
    let mut file_name = use_signal(|| Option::<String>::None);
    let mut file_bytes = use_signal(|| Option::<Vec<u8>>::None);
    let mut file_type = use_signal(|| DocumentType::Pdf);
    let mut printing = use_signal(|| false);
    let mut print_result = use_signal(|| Option::<String>::None);
    let mut stage = use_signal(|| PrintStage::Idle);

    // Print settings — bound to the UI inputs
    let mut copies = use_signal(|| 1u32);
    let mut color = use_signal(|| true);
    let mut duplex = use_signal(|| DuplexMode::Simplex);
    let mut paper_size = use_signal(|| PaperSize::A4);
    let mut orientation = use_signal(|| Orientation::Portrait);

    rsx! {
        div {
            h1 { "Print" }

            // File selection
            section { style: "margin: 16px 0;",
                h3 { "1. Select Document" }
                if let Some(ref name) = *file_name.read() {
                    div { style: "display: flex; align-items: center; gap: 8px;",
                        p { "Selected: {name}" }
                        button {
                            style: "padding: 4px 12px; border-radius: 4px; border: 1px solid #ccc; background: white; font-size: 12px;",
                            onclick: move |_| {
                                file_name.set(None);
                                file_bytes.set(None);
                                print_result.set(None);
                                stage.set(PrintStage::Idle);
                            },
                            "Clear"
                        }
                    }
                } else {
                    button {
                        style: "padding: 12px 24px; border-radius: 8px; border: 1px solid #007aff; color: #007aff; background: white; font-size: 16px;",
                        onclick: move |_| {
                            #[cfg(not(any(target_os = "ios", target_os = "android")))]
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Documents", &["pdf", "jpg", "jpeg", "png", "tiff", "tif", "txt"])
                                    .pick_file()
                                {
                                    let name = path.file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "unknown".into());
                                    let ext = path.extension()
                                        .map(|e| e.to_string_lossy().to_string())
                                        .unwrap_or_default();

                                    if let Some(dt) = DocumentType::from_extension(&ext) {
                                        file_type.set(dt);
                                    }

                                    match std::fs::read(&path) {
                                        Ok(bytes) => {
                                            tracing::info!(file = %name, bytes = bytes.len(), "file loaded");
                                            file_bytes.set(Some(bytes));
                                            file_name.set(Some(name));
                                        }
                                        Err(e) => {
                                            tracing::error!(error = %e, "failed to read file");
                                            print_result.set(Some(format!("Could not read that file. {e}")));
                                        }
                                    }
                                }
                            }
                            #[cfg(any(target_os = "ios", target_os = "android"))]
                            {
                                tracing::info!("file picker: use native bridge on mobile");
                                print_result.set(Some("File picker not yet wired on mobile".into()));
                            }
                        },
                        "Choose File"
                    }
                }
            }

            // Printer selection
            section { style: "margin: 16px 0;",
                h3 { "2. Select Printer" }
                if state.read().printers.is_empty() {
                    p { style: "color: #888;",
                        "No printers found. "
                        Link { to: crate::Route::Home {}, "Go to Home to scan." }
                    }
                } else {
                    select {
                        style: "width: 100%; padding: 8px; font-size: 16px; border-radius: 8px; border: 1px solid #ccc;",
                        onchange: move |evt| {
                            let val = evt.value().to_string();
                            if !val.is_empty() {
                                state.write().selected_printer = Some(val);
                            }
                        },
                        option { value: "", "Select a printer..." }
                        for printer in state.read().printers.iter() {
                            option { value: "{printer.uri}", "{printer.name}" }
                        }
                    }
                }
            }

            // Print settings — fully wired
            section { style: "margin: 16px 0;",
                h3 { "3. Settings" }
                div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 8px; align-items: center;",
                    label { "Copies:" }
                    input {
                        r#type: "number",
                        value: "{copies}",
                        min: "1",
                        max: "99",
                        style: "padding: 4px; border: 1px solid #ccc; border-radius: 4px;",
                        onchange: move |evt| {
                            if let Ok(n) = evt.value().parse::<u32>() {
                                copies.set(n.clamp(1, 99));
                            }
                        },
                    }

                    label { "Color:" }
                    input {
                        r#type: "checkbox",
                        checked: *color.read(),
                        onchange: move |evt| {
                            color.set(evt.checked());
                        },
                    }

                    label { "Duplex:" }
                    select {
                        style: "padding: 4px; border: 1px solid #ccc; border-radius: 4px;",
                        onchange: move |evt| {
                            let val = evt.value().to_string();
                            duplex.set(match val.as_str() {
                                "long-edge" => DuplexMode::LongEdge,
                                "short-edge" => DuplexMode::ShortEdge,
                                _ => DuplexMode::Simplex,
                            });
                        },
                        option { value: "simplex", "One-sided" }
                        option { value: "long-edge", "Two-sided (long edge)" }
                        option { value: "short-edge", "Two-sided (short edge)" }
                    }

                    label { "Paper:" }
                    select {
                        style: "padding: 4px; border: 1px solid #ccc; border-radius: 4px;",
                        onchange: move |evt| {
                            let val = evt.value().to_string();
                            paper_size.set(match val.as_str() {
                                "A3" => PaperSize::A3,
                                "A5" => PaperSize::A5,
                                "Letter" => PaperSize::Letter,
                                "Legal" => PaperSize::Legal,
                                "Tabloid" => PaperSize::Tabloid,
                                _ => PaperSize::A4,
                            });
                        },
                        option { value: "A4", "A4" }
                        option { value: "A3", "A3" }
                        option { value: "A5", "A5" }
                        option { value: "Letter", "Letter" }
                        option { value: "Legal", "Legal" }
                        option { value: "Tabloid", "Tabloid" }
                    }

                    label { "Orientation:" }
                    select {
                        style: "padding: 4px; border: 1px solid #ccc; border-radius: 4px;",
                        onchange: move |evt| {
                            let val = evt.value().to_string();
                            orientation.set(match val.as_str() {
                                "landscape" => Orientation::Landscape,
                                _ => Orientation::Portrait,
                            });
                        },
                        option { value: "portrait", "Portrait" }
                        option { value: "landscape", "Landscape" }
                    }
                }
            }

            // Print button
            button {
                style: "width: 100%; padding: 16px; border-radius: 12px; border: none; background: #007aff; color: white; font-size: 18px; font-weight: bold; margin-top: 16px;",
                disabled: file_bytes.read().is_none() || state.read().selected_printer.is_none() || *printing.read(),
                onclick: {
                    let svc = svc.clone();
                    move |_| {
                        let doc_bytes = file_bytes.read().clone();
                        let doc_name = file_name.read().clone();
                        let printer_uri = state.read().selected_printer.clone();
                        let doc_type = *file_type.read();

                        let settings = PrintSettings {
                            copies: *copies.read(),
                            paper_size: *paper_size.read(),
                            duplex: *duplex.read(),
                            orientation: *orientation.read(),
                            color: *color.read(),
                            page_range: None,
                            scale_to_fit: true,
                        };

                        if let (Some(bytes), Some(name), Some(uri)) = (doc_bytes, doc_name, printer_uri) {
                            printing.set(true);
                            stage.set(PrintStage::Preparing);
                            print_result.set(None);
                            let svc = svc.clone();

                            spawn(async move {
                                stage.set(PrintStage::Sending);
                                match svc.print_document(bytes, name, doc_type, uri, settings).await {
                                    Ok(job_id) => {
                                        tracing::info!(job_id = %job_id, "print job submitted");
                                        stage.set(PrintStage::Complete);
                                        print_result.set(Some(format!("Job submitted: {job_id}")));
                                        if let Ok(jobs) = svc.all_jobs() {
                                            state.write().jobs = jobs;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!(error = %e, "print failed");
                                        stage.set(PrintStage::Failed);
                                        print_result.set(Some(
                                            presswerk_core::human_errors::humanize_error(&e).message,
                                        ));
                                    }
                                }
                                printing.set(false);
                            });
                        }
                    }
                },
                if *printing.read() { "Printing..." } else { "Print" }
            }

            // Progress feedback
            if *stage.read() != PrintStage::Idle {
                {
                    let current_stage = *stage.read();
                    rsx! {
                        div {
                            style: "margin-top: 16px; padding: 16px; border-radius: 12px; background: {current_stage.bg()}; text-align: center;",
                            p { style: "color: {current_stage.color()}; font-size: 16px; font-weight: bold; margin: 0;",
                                "{current_stage.message()}"
                            }
                            if let Some(ref msg) = *print_result.read() {
                                p { style: "color: #666; font-size: 14px; margin-top: 8px;",
                                    "{msg}"
                                }
                            }
                            if current_stage == PrintStage::Failed {
                                div { style: "margin-top: 12px;",
                                    Link {
                                        to: crate::Route::Doctor {},
                                        style: "color: #007aff; text-decoration: underline; font-size: 14px;",
                                        "Having trouble? Run Print Doctor"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
