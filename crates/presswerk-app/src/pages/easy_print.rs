// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Easy Mode — the default Print Doctor interface.
//
// This IS the app for most users. Three steps: Choose file → (printer
// auto-selected) → PRINT. Giant touch targets, large text, auto-defaults.
//
// The "advanced" Presswerk interface is accessible via Settings → Advanced Mode.

use dioxus::prelude::*;

use presswerk_core::types::{DocumentType, PrintSettings};

use crate::services::app_services::AppServices;
use crate::state::AppState;

#[component]
pub fn EasyPrint() -> Element {
    let state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();
    let mut file_name = use_signal(|| Option::<String>::None);
    let mut file_bytes = use_signal(|| Option::<Vec<u8>>::None);
    let mut file_type = use_signal(|| DocumentType::Pdf);
    let mut printing = use_signal(|| false);
    let mut done = use_signal(|| false);
    let mut error_msg = use_signal(|| Option::<String>::None);

    // Auto-select the only printer, or the last-used printer
    let auto_printer = {
        let printers = &state.read().printers;
        if printers.len() == 1 {
            Some(printers[0].uri.clone())
        } else {
            state.read().selected_printer.clone()
        }
    };

    rsx! {
        div { style: "display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 80vh; padding: 24px;",

            // Title
            h1 { style: "font-size: 32px; text-align: center; margin-bottom: 8px;",
                "Print Doctor"
            }
            p { style: "color: #666; font-size: 18px; text-align: center; margin-bottom: 32px;",
                "It just works."
            }

            if *done.read() {
                // Success screen
                div { style: "text-align: center;",
                    p { style: "font-size: 72px; margin: 0;", "\u{2705}" }
                    p { style: "font-size: 24px; font-weight: bold; color: #155724; margin-top: 16px;",
                        "Done! Your document is printing."
                    }
                    button {
                        style: "margin-top: 32px; padding: 20px 48px; border-radius: 16px; border: none; background: #007aff; color: white; font-size: 22px; font-weight: bold;",
                        onclick: move |_| {
                            done.set(false);
                            file_name.set(None);
                            file_bytes.set(None);
                            error_msg.set(None);
                        },
                        "Print Another"
                    }
                }
            } else if error_msg.read().is_some() {
                // Error screen
                div { style: "text-align: center; max-width: 400px;",
                    p { style: "font-size: 48px; margin: 0;", "\u{274C}" }
                    p { style: "font-size: 20px; color: #721c24; margin-top: 16px;",
                        "{error_msg.read().as_deref().unwrap_or(\"\")}"
                    }
                    div { style: "display: flex; flex-direction: column; gap: 12px; margin-top: 24px;",
                        button {
                            style: "padding: 20px; border-radius: 16px; border: none; background: #007aff; color: white; font-size: 20px; font-weight: bold;",
                            onclick: move |_| {
                                error_msg.set(None);
                            },
                            "Try Again"
                        }
                        Link {
                            to: crate::Route::Doctor {},
                            style: "padding: 16px; border-radius: 16px; border: 2px solid #007aff; color: #007aff; background: white; font-size: 18px; font-weight: bold; text-decoration: none; text-align: center;",
                            "Get Help"
                        }
                    }
                }
            } else if file_name.read().is_some() {
                // File selected — show PRINT button
                div { style: "text-align: center; width: 100%; max-width: 400px;",
                    p { style: "font-size: 18px; color: #333; margin-bottom: 8px;",
                        "Ready to print:"
                    }
                    p { style: "font-size: 22px; font-weight: bold; color: #007aff; margin-bottom: 16px;",
                        "{file_name.read().as_deref().unwrap_or(\"\")}"
                    }

                    // Show selected printer
                    if let Some(ref uri) = auto_printer {
                        p { style: "font-size: 14px; color: #888; margin-bottom: 24px;",
                            "Sending to: {uri}"
                        }
                    }

                    button {
                        style: "width: 100%; padding: 24px; border-radius: 20px; border: none; background: #007aff; color: white; font-size: 28px; font-weight: bold; min-height: 80px;",
                        disabled: auto_printer.is_none() || *printing.read(),
                        onclick: {
                            let svc = svc.clone();
                            let printer_uri = auto_printer.clone();
                            move |_| {
                                let doc_bytes = file_bytes.read().clone();
                                let doc_name = file_name.read().clone();
                                let doc_type = *file_type.read();

                                if let (Some(bytes), Some(name), Some(uri)) = (doc_bytes, doc_name, printer_uri.clone()) {
                                    printing.set(true);
                                    let svc = svc.clone();
                                    let settings = PrintSettings::default();

                                    spawn(async move {
                                        match svc.print_document(bytes, name, doc_type, uri, settings).await {
                                            Ok(_) => {
                                                done.set(true);
                                            }
                                            Err(e) => {
                                                let human = presswerk_core::human_errors::humanize_error(&e);
                                                error_msg.set(Some(format!("{} {}", human.message, human.suggestion)));
                                            }
                                        }
                                        printing.set(false);
                                    });
                                }
                            }
                        },
                        if *printing.read() { "Printing..." } else { "PRINT" }
                    }

                    if auto_printer.is_none() {
                        p { style: "color: #ff9500; font-size: 16px; margin-top: 16px;",
                            "No printer found. "
                            Link {
                                to: crate::Route::Doctor {},
                                style: "color: #007aff; text-decoration: underline;",
                                "Find your printer"
                            }
                        }
                    }

                    // More options link
                    button {
                        style: "margin-top: 16px; padding: 8px 16px; border: none; background: none; color: #888; font-size: 14px; text-decoration: underline;",
                        onclick: move |_| {
                            file_name.set(None);
                            file_bytes.set(None);
                        },
                        "Choose a different file"
                    }
                }
            } else {
                // File picker — big button
                div { style: "text-align: center; width: 100%; max-width: 400px;",
                    button {
                        style: "width: 100%; padding: 32px; border-radius: 20px; border: 3px dashed #007aff; background: #f0f7ff; color: #007aff; font-size: 24px; font-weight: bold; min-height: 120px;",
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
                                            file_bytes.set(Some(bytes));
                                            file_name.set(Some(name));
                                        }
                                        Err(e) => {
                                            error_msg.set(Some(format!("Could not read that file. {e}")));
                                        }
                                    }
                                }
                            }
                            #[cfg(any(target_os = "ios", target_os = "android"))]
                            {
                                error_msg.set(Some("File picker coming soon on mobile.".into()));
                            }
                        },
                        "Choose File to Print"
                    }

                    // Printer status
                    div { style: "margin-top: 24px;",
                        if state.read().printers.is_empty() {
                            p { style: "color: #888; font-size: 16px;",
                                "Looking for printers..."
                            }
                        } else {
                            p { style: "color: #666; font-size: 16px;",
                                "{state.read().printers.len()} printer(s) found"
                            }
                        }
                    }
                }
            }
        }
    }
}
