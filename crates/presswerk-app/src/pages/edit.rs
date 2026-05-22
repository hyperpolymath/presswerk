// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Edit page — PDF editor with page thumbnails and toolbar.

use dioxus::prelude::*;

use presswerk_document::pdf::reader::PdfReader;

use crate::services::app_services::AppServices;
use crate::state::AppState;

#[component]
pub fn Edit() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();
    let mut page_count = use_signal(|| 0u32);
    let mut selected_page = use_signal(|| Option::<u32>::None);
    let mut pdf_bytes = use_signal(|| Option::<Vec<u8>>::None);
    let mut status_msg = use_signal(|| Option::<String>::None);

    rsx! {
        div {
            h1 { "Edit" }
            p { style: "color: #666;", "Open a PDF to edit pages." }

            // Open file
            button {
                style: "width: 100%; padding: 12px; border-radius: 8px; border: 1px solid #007aff; color: #007aff; background: white; font-size: 16px; margin: 16px 0;",
                onclick: move |_| {
                    #[cfg(not(any(target_os = "ios", target_os = "android")))]
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PDF", &["pdf"])
                            .pick_file()
                        {
                            match std::fs::read(&path) {
                                Ok(bytes) => {
                                    match PdfReader::from_bytes(&bytes) {
                                        Ok(reader) => {
                                            let count = reader.page_count() as u32;
                                            page_count.set(count);
                                            pdf_bytes.set(Some(bytes.clone()));
                                            selected_page.set(None);
                                            let name = path.file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                                .unwrap_or_else(|| "document.pdf".into());
                                            state.write().current_document = Some(bytes);
                                            state.write().current_document_name = Some(name.clone());
                                            status_msg.set(Some(format!("Opened {name} ({count} pages)")));
                                            tracing::info!(file = %name, pages = count, "PDF opened for editing");
                                        }
                                        Err(e) => {
                                            status_msg.set(Some(format!("Invalid PDF: {e}")));
                                        }
                                    }
                                }
                                Err(e) => {
                                    status_msg.set(Some(format!("Error: {e}")));
                                }
                            }
                        }
                    }
                    #[cfg(any(target_os = "ios", target_os = "android"))]
                    {
                        status_msg.set(Some("File picker not yet wired on mobile".into()));
                    }
                },
                "Open PDF"
            }

            if *page_count.read() > 0 {
                // Toolbar
                div { style: "display: flex; gap: 8px; flex-wrap: wrap; margin: 12px 0;",
                    ToolButton {
                        label: "Rotate",
                        icon: "\u{1F504}",
                        disabled: selected_page.read().is_none(),
                        onclick: {
                            let svc = svc.clone();
                            move |_| {
                                let current_bytes = pdf_bytes.read().clone();
                                let current_page = *selected_page.read();
                                if let (Some(bytes), Some(page_num)) = (current_bytes, current_page) {
                                    match PdfReader::from_bytes(&bytes) {
                                        Ok(reader) => {
                                            match reader.rotate_page(page_num, 90) {
                                                Ok(new_bytes) => {
                                                    state.write().current_document = Some(new_bytes.clone());
                                                    pdf_bytes.set(Some(new_bytes));
                                                    svc.audit("pdf_rotate", "editor", true, Some(&format!("page {page_num}")));
                                                    status_msg.set(Some(format!("Page {page_num} rotated 90\u{00B0}")));
                                                }
                                                Err(e) => status_msg.set(Some(format!("Rotate failed: {e}"))),
                                            }
                                        }
                                        Err(e) => status_msg.set(Some(format!("PDF error: {e}"))),
                                    }
                                }
                            }
                        },
                    }
                    ToolButton {
                        label: "Extract",
                        icon: "\u{1F4C4}",
                        disabled: selected_page.read().is_none(),
                        onclick: {
                            let svc = svc.clone();
                            move |_| {
                                let current_bytes = pdf_bytes.read().clone();
                                let current_page = *selected_page.read();
                                if let (Some(bytes), Some(page_num)) = (current_bytes, current_page) {
                                    match PdfReader::from_bytes(&bytes) {
                                        Ok(reader) => {
                                            match reader.extract_page(page_num) {
                                                Ok(extracted) => {
                                                    match svc.store_document(&extracted) {
                                                        Ok(hash) => {
                                                            svc.audit("pdf_extract", &hash, true, Some(&format!("page {page_num}")));
                                                            status_msg.set(Some(format!("Page {page_num} extracted ({} KB)", extracted.len() / 1024)));
                                                        }
                                                        Err(e) => status_msg.set(Some(format!("Save failed: {e}"))),
                                                    }
                                                }
                                                Err(e) => status_msg.set(Some(format!("Extract failed: {e}"))),
                                            }
                                        }
                                        Err(e) => status_msg.set(Some(format!("PDF error: {e}"))),
                                    }
                                }
                            }
                        },
                    }
                    ToolButton {
                        label: "Split",
                        icon: "\u{2194}",
                        disabled: selected_page.read().is_none() || *page_count.read() < 2,
                        onclick: {
                            let svc = svc.clone();
                            move |_| {
                                let current_bytes = pdf_bytes.read().clone();
                                let current_page = *selected_page.read();
                                if let (Some(bytes), Some(page_num)) = (current_bytes, current_page) {
                                    match PdfReader::from_bytes(&bytes) {
                                        Ok(reader) => {
                                            match reader.split(page_num) {
                                                Ok((part_a, part_b)) => {
                                                    let _ = svc.store_document(&part_a);
                                                    let _ = svc.store_document(&part_b);
                                                    svc.audit("pdf_split", "editor", true, Some(&format!("after page {page_num}")));
                                                    status_msg.set(Some(format!("Split after page {page_num}")));
                                                }
                                                Err(e) => status_msg.set(Some(format!("Split failed: {e}"))),
                                            }
                                        }
                                        Err(e) => status_msg.set(Some(format!("PDF error: {e}"))),
                                    }
                                }
                            }
                        },
                    }
                }

                // Page thumbnails
                {
                    let count = *page_count.read();
                    rsx! {
                        h3 { "{count} pages" }
                    }
                }
                div { style: "display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px;",
                    for i in 0..*page_count.read() {
                        {
                            let page_num = i + 1;
                            let is_selected = *selected_page.read() == Some(page_num);
                            let border = if is_selected { "2px solid #007aff" } else { "1px solid #ccc" };
                            rsx! {
                                div {
                                    style: "aspect-ratio: 0.707; border: {border}; border-radius: 4px; display: flex; align-items: center; justify-content: center; background: white; font-size: 14px; color: #666; cursor: pointer;",
                                    onclick: move |_| {
                                        selected_page.set(Some(page_num));
                                    },
                                    "Page {page_num}"
                                }
                            }
                        }
                    }
                }

                // Save
                button {
                    style: "width: 100%; padding: 12px; border-radius: 8px; border: none; background: #34c759; color: white; font-size: 16px; margin-top: 16px;",
                    onclick: {
                        let svc = svc.clone();
                        move |_| {
                            if let Some(ref bytes) = *pdf_bytes.read() {
                                match svc.store_document(bytes) {
                                    Ok(hash) => {
                                        svc.audit("pdf_save", &hash, true, None);
                                        status_msg.set(Some(format!("PDF saved ({} KB)", bytes.len() / 1024)));
                                    }
                                    Err(e) => {
                                        status_msg.set(Some(format!("Save failed: {e}")));
                                    }
                                }
                            }
                        }
                    },
                    "Save PDF"
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

#[component]
fn ToolButton(
    label: &'static str,
    icon: &'static str,
    disabled: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let opacity = if disabled { "0.5" } else { "1" };
    rsx! {
        button {
            style: "padding: 8px 12px; border-radius: 8px; border: 1px solid #ccc; background: white; font-size: 14px; opacity: {opacity};",
            disabled: disabled,
            onclick: move |evt| onclick.call(evt),
            "{icon} {label}"
        }
    }
}
