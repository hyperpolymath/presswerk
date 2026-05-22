// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Home page — printer discovery list and quick actions.

use dioxus::prelude::*;

use crate::Route;
use crate::services::app_services::AppServices;
use crate::state::AppState;

#[component]
pub fn Home() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();

    // Periodically refresh the printer list from the discovery engine
    let svc_poll = svc.clone();
    let _poller = use_resource(move || {
        let svc = svc_poll.clone();
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let printers = svc.discovered_printers();
                let scanning = svc.is_discovering();
                state.write().printers = printers;
                state.write().scanning = scanning;
            }
        }
    });

    rsx! {
        div {
            h1 { "Presswerk" }
            p { style: "color: #666;", "High-assurance local print router" }

            // Quick actions
            div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin: 24px 0;",
                QuickAction { to: Route::Print {}, label: "Print a File", icon: "\u{1F5A8}" }
                QuickAction { to: Route::Scan {}, label: "Scan Document", icon: "\u{1F4F7}" }
                QuickAction { to: Route::TextEditor {}, label: "New Text", icon: "\u{1F4DD}" }
                QuickAction { to: Route::Server {}, label: "Print Server", icon: "\u{1F4E1}" }
            }

            // Discovered printers
            div { style: "display: flex; justify-content: space-between; align-items: center;",
                h2 { "Printers" }
                if state.read().scanning {
                    span { style: "color: #007aff; font-size: 14px;", "Scanning..." }
                }
            }

            if state.read().printers.is_empty() {
                p { style: "color: #888;", "No printers found on the network." }
                button {
                    style: "padding: 8px 16px; border-radius: 8px; border: 1px solid #ccc; background: white;",
                    onclick: {
                        let svc = svc.clone();
                        move |_| {
                            tracing::info!("Starting printer discovery");
                            state.write().scanning = true;
                            if let Err(e) = svc.start_discovery() {
                                tracing::error!(error = %e, "discovery start failed");
                                state.write().status_message = Some(format!("Discovery failed: {e}"));
                            }
                        }
                    },
                    "Scan for Printers"
                }
            } else {
                {
                    let count = state.read().printers.len();
                    rsx! {
                        p { style: "color: #666; font-size: 14px; margin-bottom: 8px;",
                            "{count} printer(s) found"
                        }
                    }
                }
                for printer in state.read().printers.iter() {
                    {
                        let uri = printer.uri.clone();
                        let is_selected = state.read().selected_printer.as_deref() == Some(&uri);
                        let border = if is_selected { "2px solid #007aff" } else { "1px solid #e0e0e0" };
                        rsx! {
                            div {
                                style: "padding: 12px; margin: 8px 0; border: {border}; border-radius: 8px; cursor: pointer;",
                                onclick: move |_| {
                                    state.write().selected_printer = Some(uri.clone());
                                    tracing::info!(uri = %uri, "printer selected");
                                },
                                strong { "{printer.name}" }
                                p { style: "color: #666; font-size: 14px; margin: 4px 0;",
                                    "{printer.ip}:{printer.port}"
                                    if let Some(ref model) = printer.make_and_model {
                                        " — {model}"
                                    }
                                }
                                div { style: "font-size: 12px; color: #888;",
                                    if printer.supports_color { span { "Color " } }
                                    if printer.supports_duplex { span { "Duplex " } }
                                    if printer.supports_tls { span { "TLS " } }
                                }
                            }
                        }
                    }
                }
            }

            // Add printer manually
            div { style: "margin-top: 16px; text-align: center;",
                Link {
                    to: Route::AddPrinter {},
                    style: "color: #007aff; font-size: 14px; text-decoration: underline;",
                    "Add Printer Manually"
                }
            }

            // Print Doctor link
            div { style: "margin-top: 8px; text-align: center;",
                Link {
                    to: Route::Doctor {},
                    style: "color: #007aff; font-size: 14px; text-decoration: underline;",
                    "Having trouble? Run Print Doctor"
                }
            }

            // Status message
            if let Some(ref msg) = state.read().status_message {
                p { style: "color: #ff9500; font-size: 14px; margin-top: 12px;", "{msg}" }
            }
        }
    }
}

#[component]
fn QuickAction(to: Route, label: &'static str, icon: &'static str) -> Element {
    rsx! {
        Link { to: to,
            style: "display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 24px 16px; border: 1px solid #e0e0e0; border-radius: 12px; text-decoration: none; color: #333; background: white;",
            span { style: "font-size: 32px; margin-bottom: 8px;", "{icon}" }
            span { style: "font-size: 14px;", "{label}" }
        }
    }
}
