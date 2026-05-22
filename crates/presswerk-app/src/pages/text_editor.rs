// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Text editor page — create plain text documents, export as PDF, or print.

use dioxus::prelude::*;

use presswerk_core::types::DocumentType;
use presswerk_document::PdfWriter;

use crate::services::app_services::AppServices;
use crate::state::AppState;

#[component]
pub fn TextEditor() -> Element {
    let state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();
    let mut text = use_signal(String::new);
    let mut status_msg = use_signal(|| Option::<String>::None);

    rsx! {
        div { style: "display: flex; flex-direction: column; height: 100%;",
            h1 { "Text Editor" }

            textarea {
                style: "flex: 1; min-height: 300px; padding: 12px; font-family: monospace; font-size: 14px; border: 1px solid #ccc; border-radius: 8px; resize: none;",
                placeholder: "Type or paste text here...",
                value: "{text}",
                oninput: move |evt| text.set(evt.value().to_string()),
            }

            div { style: "display: flex; gap: 8px; margin-top: 12px;",
                button {
                    style: "flex: 1; padding: 12px; border-radius: 8px; border: 1px solid #007aff; color: #007aff; background: white;",
                    disabled: text.read().is_empty(),
                    onclick: {
                        let svc = svc.clone();
                        move |_| {
                            let content = text.read().clone();
                            let mut writer = PdfWriter::a4();
                            writer.set_title("Text Document");
                            match writer.create_from_text(&content) {
                                Ok(pdf_bytes) => {
                                    match svc.store_document(&pdf_bytes) {
                                        Ok(hash) => {
                                            svc.audit("text_export_pdf", &hash, true, None);
                                            tracing::info!(hash = %hash, bytes = pdf_bytes.len(), "text exported as PDF");
                                            status_msg.set(Some(format!("PDF exported ({} KB)", pdf_bytes.len() / 1024)));
                                        }
                                        Err(e) => {
                                            status_msg.set(Some(format!("Save failed: {e}")));
                                        }
                                    }
                                }
                                Err(e) => {
                                    status_msg.set(Some(format!("PDF creation failed: {e}")));
                                }
                            }
                        }
                    },
                    "Export PDF"
                }
                button {
                    style: "flex: 1; padding: 12px; border-radius: 8px; border: none; background: #007aff; color: white;",
                    disabled: text.read().is_empty() || state.read().selected_printer.is_none(),
                    onclick: {
                        let svc = svc.clone();
                        move |_| {
                            let content = text.read().clone();
                            let printer_uri = state.read().selected_printer.clone();

                            if let Some(uri) = printer_uri {
                                let text_bytes = content.into_bytes();
                                let svc = svc.clone();
                                spawn(async move {
                                    match svc.print_document(
                                        text_bytes,
                                        "Text Document.txt".into(),
                                        DocumentType::PlainText,
                                        uri,
                                        presswerk_core::types::PrintSettings::default(),
                                    ).await {
                                        Ok(job_id) => {
                                            status_msg.set(Some(format!("Print job submitted: {job_id}")));
                                        }
                                        Err(e) => {
                                            status_msg.set(Some(format!("Print failed: {e}")));
                                        }
                                    }
                                });
                            } else {
                                status_msg.set(Some("Select a printer first (go to Home page)".into()));
                            }
                        }
                    },
                    "Print"
                }
            }

            // Status
            if let Some(ref msg) = *status_msg.read() {
                p { style: "margin-top: 8px; color: #666; font-size: 14px; text-align: center;",
                    "{msg}"
                }
            }
        }
    }
}
