// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Server page — toggle IPP print server, view status, incoming jobs.

use dioxus::prelude::*;

use presswerk_core::types::ServerStatus;

use crate::services::app_services::AppServices;
use crate::state::AppState;

#[component]
pub fn Server() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();
    let status = state.read().server_status;

    // Periodically refresh incoming jobs while server is running
    {
        let svc = svc.clone();
        use_resource(move || {
            let svc = svc.clone();
            async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if state.read().server_status == ServerStatus::Running {
                        if let Ok(jobs) = svc.all_jobs() {
                            state.write().jobs = jobs;
                        }
                        // Also sync server status
                        let live_status = svc.ipp_server_status();
                        if state.read().server_status != live_status {
                            state.write().server_status = live_status;
                        }
                    }
                }
            }
        });
    }

    rsx! {
        div {
            h1 { "Print Server" }
            p { style: "color: #666;",
                "Turn your device into a network printer. Other devices on the same network can discover and print to this device."
            }

            // Status indicator
            div { style: "display: flex; align-items: center; gap: 12px; margin: 24px 0; padding: 16px; border-radius: 12px; border: 1px solid #e0e0e0;",
                div { style: "width: 16px; height: 16px; border-radius: 50%; background: {status_color(status)};", }
                div {
                    strong { "{status_label(status)}" }
                    if status == ServerStatus::Running {
                        {
                            let port = state.read().config.server_port;
                            let tls_text = if state.read().config.server_require_tls { "TLS enabled" } else { "TLS disabled" };
                            rsx! {
                                p { style: "margin: 4px 0 0; color: #666; font-size: 14px;",
                                    "Port {port} • {tls_text}"
                                }
                            }
                        }
                    }
                }
            }

            // Toggle button
            button {
                style: "width: 100%; padding: 16px; border-radius: 12px; border: none; font-size: 18px; font-weight: bold; color: white; background: {toggle_color(status)};",
                disabled: status == ServerStatus::Starting,
                onclick: {
                    let svc = svc.clone();
                    move |_| {
                        let current = state.read().server_status;
                        let svc = svc.clone();
                        match current {
                            ServerStatus::Stopped | ServerStatus::Error => {
                                tracing::info!("Starting IPP server");
                                state.write().server_status = ServerStatus::Starting;
                                spawn(async move {
                                    match svc.start_ipp_server().await {
                                        Ok(new_status) => {
                                            state.write().server_status = new_status;
                                            tracing::info!("IPP server started");
                                        }
                                        Err(e) => {
                                            tracing::error!(error = %e, "Failed to start IPP server");
                                            state.write().server_status = ServerStatus::Error;
                                        }
                                    }
                                });
                            }
                            ServerStatus::Running => {
                                spawn(async move {
                                    match svc.stop_ipp_server().await {
                                        Ok(new_status) => {
                                            state.write().server_status = new_status;
                                            tracing::info!("IPP server stopped");
                                        }
                                        Err(e) => {
                                            tracing::error!(error = %e, "Failed to stop IPP server");
                                        }
                                    }
                                });
                            }
                            ServerStatus::Starting => {} // Button disabled during transition
                        }
                    }
                },
                match status {
                    ServerStatus::Stopped | ServerStatus::Error => "Start Server",
                    ServerStatus::Starting => "Starting...",
                    ServerStatus::Running => "Stop Server",
                }
            }

            // Network-received jobs
            if status == ServerStatus::Running {
                h2 { style: "margin-top: 24px;", "Incoming Jobs" }
                {
                    let network_jobs: Vec<_> = state.read().jobs.iter()
                        .filter(|j| matches!(j.source, presswerk_core::types::JobSource::Network { .. }))
                        .cloned()
                        .collect();

                    if network_jobs.is_empty() {
                        rsx! {
                            p { style: "color: #888;", "No incoming jobs yet. Waiting for connections..." }
                        }
                    } else {
                        rsx! {
                            for job in network_jobs.iter() {
                                div { style: "padding: 10px; margin: 6px 0; border: 1px solid #e0e0e0; border-radius: 8px;",
                                    strong { "{job.document_name}" }
                                    {
                                        let ts = job.created_at.format("%H:%M:%S").to_string();
                                        rsx! {
                                            span { style: "float: right; color: #888; font-size: 12px;", "{ts}" }
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
}

fn status_color(s: ServerStatus) -> &'static str {
    match s {
        ServerStatus::Stopped => "#ccc",
        ServerStatus::Starting => "#ff9500",
        ServerStatus::Running => "#34c759",
        ServerStatus::Error => "#ff3b30",
    }
}

fn status_label(s: ServerStatus) -> &'static str {
    match s {
        ServerStatus::Stopped => "Stopped",
        ServerStatus::Starting => "Starting...",
        ServerStatus::Running => "Running",
        ServerStatus::Error => "Error",
    }
}

fn toggle_color(s: ServerStatus) -> &'static str {
    match s {
        ServerStatus::Stopped | ServerStatus::Error => "#34c759",
        _ => "#ff3b30",
    }
}
