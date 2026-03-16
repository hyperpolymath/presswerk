// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Settings page — persistent app configuration.

use dioxus::prelude::*;

use presswerk_core::types::PaperSize;

use crate::services::app_services::AppServices;
use crate::state::AppState;

#[component]
pub fn Settings() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();
    let mut save_msg = use_signal(|| Option::<String>::None);

    rsx! {
        div {
            h1 { "Settings" }

            section { style: "margin: 16px 0;",
                h3 { "Print Server" }
                // Server port
                div { style: "display: flex; justify-content: space-between; align-items: center; padding: 12px 0; border-bottom: 1px solid #f0f0f0;",
                    span { "Server port" }
                    input {
                        r#type: "number",
                        style: "width: 80px; padding: 4px 8px; border: 1px solid #ccc; border-radius: 4px; text-align: right;",
                        value: "{state.read().config.server_port}",
                        onchange: move |evt| {
                            if let Ok(port) = evt.value().parse::<u16>()
                                && port > 0
                            {
                                state.write().config.server_port = port;
                            }
                        },
                    }
                }
                SettingRow {
                    label: "Auto-start server on launch",
                    checked: state.read().config.auto_start_server,
                    on_toggle: move |v: bool| { state.write().config.auto_start_server = v; },
                }
                SettingRow {
                    label: "Require TLS",
                    checked: state.read().config.server_require_tls,
                    on_toggle: move |v: bool| { state.write().config.server_require_tls = v; },
                }
                SettingRow {
                    label: "Auto-accept network jobs",
                    checked: state.read().config.auto_accept_network_jobs,
                    on_toggle: move |v: bool| { state.write().config.auto_accept_network_jobs = v; },
                }
            }

            section { style: "margin: 16px 0;",
                h3 { "Printing" }
                // Default paper size
                div { style: "display: flex; justify-content: space-between; align-items: center; padding: 12px 0; border-bottom: 1px solid #f0f0f0;",
                    span { "Default paper size" }
                    select {
                        style: "padding: 4px 8px; border: 1px solid #ccc; border-radius: 4px;",
                        value: paper_size_label(&state.read().config.default_paper_size),
                        onchange: move |evt| {
                            if let Some(ps) = paper_size_from_label(&evt.value()) {
                                state.write().config.default_paper_size = ps;
                            }
                        },
                        option { value: "A4", "A4" }
                        option { value: "A3", "A3" }
                        option { value: "A5", "A5" }
                        option { value: "Letter", "Letter" }
                        option { value: "Legal", "Legal" }
                        option { value: "Tabloid", "Tabloid" }
                    }
                }
            }

            section { style: "margin: 16px 0;",
                h3 { "Security" }
                SettingRow {
                    label: "Encrypt local storage",
                    checked: state.read().config.encryption_enabled,
                    on_toggle: move |v: bool| { state.write().config.encryption_enabled = v; },
                }
                SettingRow {
                    label: "Enable audit trail",
                    checked: state.read().config.audit_enabled,
                    on_toggle: move |v: bool| { state.write().config.audit_enabled = v; },
                }
            }

            // Save button
            button {
                style: "width: 100%; padding: 12px; border-radius: 8px; border: none; background: #007aff; color: white; font-size: 16px; margin-top: 8px;",
                onclick: {
                    let svc = svc.clone();
                    move |_| {
                        let config = state.read().config.clone();
                        match svc.save_config(&config) {
                            Ok(()) => {
                                tracing::info!("settings saved");
                                save_msg.set(Some("Settings saved.".into()));
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "failed to save settings");
                                save_msg.set(Some(format!("Save failed: {e}")));
                            }
                        }
                    }
                },
                "Save Settings"
            }
            if let Some(ref msg) = *save_msg.read() {
                p { style: "color: #34c759; font-size: 14px; text-align: center; margin-top: 8px;",
                    "{msg}"
                }
            }

            section { style: "margin: 24px 0;",
                h3 { "About" }
                p { style: "color: #666; font-size: 14px;",
                    "Presswerk v0.1.0"
                    br {}
                    "High-Assurance Local Print Router/Server"
                    br {}
                    "PMPL-1.0-or-later"
                }
            }
        }
    }
}

#[component]
fn SettingRow(label: &'static str, checked: bool, on_toggle: EventHandler<bool>) -> Element {
    rsx! {
        div { style: "display: flex; justify-content: space-between; align-items: center; padding: 12px 0; border-bottom: 1px solid #f0f0f0;",
            span { "{label}" }
            input {
                r#type: "checkbox",
                checked: checked,
                onchange: move |evt| {
                    on_toggle.call(evt.checked());
                },
            }
        }
    }
}

fn paper_size_label(ps: &PaperSize) -> &'static str {
    match ps {
        PaperSize::A4 => "A4",
        PaperSize::A3 => "A3",
        PaperSize::A5 => "A5",
        PaperSize::Letter => "Letter",
        PaperSize::Legal => "Legal",
        PaperSize::Tabloid => "Tabloid",
        PaperSize::Custom { .. } => "Custom",
    }
}

fn paper_size_from_label(label: &str) -> Option<PaperSize> {
    match label {
        "A4" => Some(PaperSize::A4),
        "A3" => Some(PaperSize::A3),
        "A5" => Some(PaperSize::A5),
        "Letter" => Some(PaperSize::Letter),
        "Legal" => Some(PaperSize::Legal),
        "Tabloid" => Some(PaperSize::Tabloid),
        _ => None,
    }
}
