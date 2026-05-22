// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Easy Mode — simplified job status page.
//
// Shows print jobs with simple status: printing... / done! / problem.
// Large text, clear colours, no technical details unless expanded.

use dioxus::prelude::*;

use presswerk_core::types::JobStatus;

use crate::services::app_services::AppServices;
use crate::state::AppState;

#[component]
pub fn EasyJobs() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();

    // Auto-refresh
    let svc_refresh = svc.clone();
    let _refresher = use_resource(move || {
        let svc = svc_refresh.clone();
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                if let Ok(jobs) = svc.all_jobs() {
                    state.write().jobs = jobs;
                }
            }
        }
    });

    rsx! {
        div { style: "padding: 8px;",
            h1 { style: "font-size: 28px; text-align: center;", "My Jobs" }

            if state.read().jobs.is_empty() {
                div { style: "text-align: center; padding: 48px 0;",
                    p { style: "font-size: 18px; color: #888;",
                        "No print jobs yet."
                    }
                    p { style: "font-size: 16px; color: #aaa; margin-top: 8px;",
                        "Print something and it will appear here."
                    }
                }
            } else {
                for job in state.read().jobs.iter() {
                    {
                        let _job_id = job.id;
                        let status = job.status;
                        let name = job.document_name.clone();
                        let (icon, label, bg, fg) = easy_status(status);
                        let has_error = job.error_message.is_some();
                        let error_msg = job.error_message.clone();

                        rsx! {
                            div {
                                style: "padding: 20px; margin: 12px 0; border-radius: 16px; background: {bg}; min-height: 80px;",

                                div { style: "display: flex; align-items: center; gap: 16px;",
                                    span { style: "font-size: 36px;", "{icon}" }
                                    div { style: "flex: 1;",
                                        p { style: "font-size: 20px; font-weight: bold; color: #333; margin: 0;",
                                            "{name}"
                                        }
                                        p { style: "font-size: 18px; color: {fg}; margin: 4px 0 0 0;",
                                            "{label}"
                                        }
                                    }
                                }

                                // Error detail (expandable)
                                if has_error {
                                    details { style: "margin-top: 12px;",
                                        summary { style: "font-size: 16px; color: #007aff; cursor: pointer;",
                                            "What went wrong?"
                                        }
                                        if let Some(ref msg) = error_msg {
                                            {
                                                let human = presswerk_core::human_errors::humanize_error(
                                                    &presswerk_core::error::PresswerkError::IppRequest(msg.clone()),
                                                );
                                                rsx! {
                                                    div { style: "margin-top: 8px; padding: 12px; background: white; border-radius: 8px;",
                                                        p { style: "font-size: 16px; color: #333; margin: 0;",
                                                            "{human.message}"
                                                        }
                                                        p { style: "font-size: 14px; color: #666; margin: 8px 0 0 0;",
                                                            "{human.suggestion}"
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
            }
        }
    }
}

/// Map job status to simple Easy Mode presentation.
/// Returns (icon, label, background_color, text_color).
fn easy_status(status: JobStatus) -> (&'static str, &'static str, &'static str, &'static str) {
    match status {
        JobStatus::Pending | JobStatus::Held => ("\u{23F3}", "Waiting...", "#f5f5f5", "#888"),
        JobStatus::Processing => ("\u{1F4E8}", "Printing...", "#fff3cd", "#856404"),
        JobStatus::RetryPending => ("\u{1F504}", "Trying again...", "#fff3cd", "#856404"),
        JobStatus::Completed => ("\u{2705}", "Done!", "#d4edda", "#155724"),
        JobStatus::Failed => ("\u{274C}", "Problem", "#f8d7da", "#721c24"),
        JobStatus::Cancelled => ("\u{1F6AB}", "Cancelled", "#e2e3e5", "#383d41"),
    }
}
