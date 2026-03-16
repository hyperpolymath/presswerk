// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Scan page — capture images, enhance, export as PDF.
//
// On desktop, the "capture" button opens a file dialog for image selection.
// On mobile, it will use the native camera bridge (presswerk-bridge).

use dioxus::prelude::*;

use presswerk_core::types::PaperSize;
use presswerk_document::scan::enhance::ScanEnhancer;

use crate::services::app_services::AppServices;

#[component]
pub fn Scan() -> Element {
    let svc = use_context::<AppServices>();
    let mut scanned_pages = use_signal(Vec::<Vec<u8>>::new);
    let mut status_msg = use_signal(|| Option::<String>::None);
    let mut processing = use_signal(|| false);

    rsx! {
        div {
            h1 { "Scan" }
            p { style: "color: #666;", "Capture documents with your camera or load images." }

            // Capture / load button
            button {
                style: "width: 100%; padding: 16px; border-radius: 12px; border: 2px dashed #007aff; color: #007aff; background: white; font-size: 16px; margin: 16px 0;",
                disabled: *processing.read(),
                onclick: move |_| {
                    #[cfg(not(any(target_os = "ios", target_os = "android")))]
                    {
                        // Desktop: open file dialog for images
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "tiff", "tif", "bmp"])
                            .pick_file()
                        {
                            match std::fs::read(&path) {
                                Ok(bytes) => {
                                    tracing::info!(path = %path.display(), bytes = bytes.len(), "image loaded for scanning");
                                    scanned_pages.write().push(bytes);
                                    status_msg.set(Some("Page added.".into()));
                                }
                                Err(e) => {
                                    status_msg.set(Some(format!("Error: {e}")));
                                }
                            }
                        }
                    }
                    #[cfg(any(target_os = "ios", target_os = "android"))]
                    {
                        tracing::info!("camera capture: requires native bridge");
                        status_msg.set(Some("Camera not yet wired on mobile".into()));
                    }
                },
                "\u{1F4F7} Capture Page"
            }

            // Scanned pages
            if scanned_pages.read().is_empty() {
                p { style: "text-align: center; color: #aaa; margin: 48px 0;",
                    "No pages scanned yet."
                }
            } else {
                {
                    let count = scanned_pages.read().len();
                    rsx! {
                        h3 { "{count} page(s) scanned" }
                    }
                }
                div { style: "display: flex; gap: 8px; overflow-x: auto; padding: 8px 0;",
                    for (i, page) in scanned_pages.read().iter().enumerate() {
                        {
                            let size_kb = page.len() / 1024;
                            rsx! {
                                div { style: "min-width: 80px; height: 110px; border: 1px solid #ccc; border-radius: 4px; display: flex; flex-direction: column; align-items: center; justify-content: center; background: #f0f0f0; font-size: 12px;",
                                    span { "P{i + 1}" }
                                    span { style: "color: #888;", "{size_kb}KB" }
                                }
                            }
                        }
                    }
                }
            }

            // Actions
            div { style: "display: flex; gap: 8px; margin-top: 16px;",
                button {
                    style: "flex: 1; padding: 12px; border-radius: 8px; border: 1px solid #ccc; background: white;",
                    disabled: scanned_pages.read().is_empty() || *processing.read(),
                    onclick: move |_| {
                        processing.set(true);
                        status_msg.set(Some("Enhancing...".into()));

                        let pages = scanned_pages.read().clone();
                        let mut enhanced = Vec::new();
                        let mut had_errors = false;

                        for page_bytes in &pages {
                            match ScanEnhancer::from_bytes(page_bytes, PaperSize::A4) {
                                Ok(enhancer) => {
                                    match enhancer.enhance_and_convert() {
                                        Ok(pdf_bytes) => {
                                            enhanced.push(pdf_bytes);
                                        }
                                        Err(_) => {
                                            enhanced.push(page_bytes.clone());
                                            had_errors = true;
                                        }
                                    }
                                }
                                Err(_) => {
                                    enhanced.push(page_bytes.clone());
                                    had_errors = true;
                                }
                            }
                        }

                        scanned_pages.set(enhanced);
                        processing.set(false);
                        if had_errors {
                            status_msg.set(Some("Some pages could not be enhanced.".into()));
                        } else {
                            status_msg.set(Some("All pages enhanced.".into()));
                        }
                    },
                    "Enhance"
                }
                button {
                    style: "flex: 1; padding: 12px; border-radius: 8px; border: none; background: #007aff; color: white;",
                    disabled: scanned_pages.read().is_empty() || *processing.read(),
                    onclick: {
                        let svc = svc.clone();
                        move |_| {
                            processing.set(true);
                            status_msg.set(Some("Converting to PDF...".into()));

                            let pages = scanned_pages.read().clone();
                            // Combine all scanned pages into one PDF
                            // For multi-page, convert each to PDF and merge
                            let combined_result: std::result::Result<Vec<u8>, _> = if pages.len() == 1 {
                                ScanEnhancer::from_bytes(&pages[0], PaperSize::A4)
                                    .and_then(|e| e.enhance_and_convert())
                            } else {
                                // Convert first page, then merge rest
                                // For MVP, just use the first page
                                ScanEnhancer::from_bytes(&pages[0], PaperSize::A4)
                                    .and_then(|e| e.enhance_and_convert())
                            };
                            match combined_result {
                                Ok(pdf_bytes) => {
                                    match svc.store_document(&pdf_bytes) {
                                        Ok(hash) => {
                                            svc.audit("scan_export_pdf", &hash, true, None);
                                            tracing::info!(hash = %hash, bytes = pdf_bytes.len(), "scan exported as PDF");
                                            status_msg.set(Some(format!("PDF exported ({} KB)", pdf_bytes.len() / 1024)));
                                        }
                                        Err(e) => {
                                            status_msg.set(Some(format!("Save failed: {e}")));
                                        }
                                    }
                                }
                                Err(e) => {
                                    status_msg.set(Some(format!("PDF conversion failed: {e}")));
                                }
                            }
                            processing.set(false);
                        }
                    },
                    "Export PDF"
                }
            }

            // Clear button
            if !scanned_pages.read().is_empty() {
                button {
                    style: "width: 100%; padding: 8px; border-radius: 8px; border: 1px solid #ff3b30; color: #ff3b30; background: white; font-size: 14px; margin-top: 8px;",
                    onclick: move |_| {
                        scanned_pages.set(Vec::new());
                        status_msg.set(None);
                    },
                    "Clear All Pages"
                }
            }

            // Status
            if let Some(ref msg) = *status_msg.read() {
                p { style: "margin-top: 12px; color: #666; font-size: 14px; text-align: center;",
                    "{msg}"
                }
            }
        }
    }
}
