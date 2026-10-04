// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Jobs page — view all print jobs, their status, cancel/retry, release, and
// delete.  Jobs whose document was classified as a machine-filled form carry a
// provenance badge (issue #118, ruling D189).

use dioxus::prelude::*;

use presswerk_core::types::JobStatus;

use crate::services::app_services::AppServices;
use crate::state::AppState;

#[component]
pub fn Jobs() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();

    // Refresh job list from the database
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
        div {
            div { style: "display: flex; justify-content: space-between; align-items: center;",
                h1 { "Jobs" }
                button {
                    style: "padding: 6px 12px; border-radius: 6px; border: 1px solid #ccc; background: white; font-size: 13px;",
                    onclick: {
                        let svc = svc.clone();
                        move |_| {
                            if let Ok(jobs) = svc.all_jobs() {
                                state.write().jobs = jobs;
                            }
                        }
                    },
                    "Refresh"
                }
            }

            if state.read().jobs.is_empty() {
                p { style: "text-align: center; color: #aaa; margin: 48px 0;",
                    "No print jobs yet."
                }
            } else {
                for job in state.read().jobs.iter() {
                    {
                        let job_id = job.id;
                        let job_status = job.status;
                        let ts = job.created_at.format("%Y-%m-%d %H:%M").to_string();
                        let can_cancel = matches!(job_status, JobStatus::Pending | JobStatus::Held);
                        let can_release = matches!(job_status, JobStatus::Held);
                        let is_terminal = matches!(job_status, JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled);
                        let provenance_note = job.form_provenance.summary();
                        let machine_filled = job.form_provenance.is_machine_filled();

                        rsx! {
                            div { style: "padding: 12px; margin: 8px 0; border: 1px solid #e0e0e0; border-radius: 8px;",
                                div { style: "display: flex; justify-content: space-between; align-items: center;",
                                    strong { "{job.document_name}" }
                                    span { style: "font-size: 12px; padding: 4px 8px; border-radius: 4px; background: {status_bg(job_status)}; color: {status_fg(job_status)};",
                                        "{status_text(job_status)}"
                                    }
                                }
                                p { style: "color: #666; font-size: 14px; margin: 4px 0;", "{ts}" }
                                if machine_filled {
                                    p { style: "color: #8a5300; font-size: 12px; margin: 4px 0; background: #fff4e5; border: 1px solid #ffd8a8; border-radius: 4px; padding: 2px 6px;",
                                        "Machine-filled: {provenance_note}"
                                    }
                                }
                                if let Some(ref uri) = job.printer_uri {
                                    p { style: "color: #999; font-size: 12px;", "{uri}" }
                                }
                                if let Some(ref err) = job.error_message {
                                    p { style: "color: #ff3b30; font-size: 13px;", "{err}" }
                                }
                                // Action buttons
                                div { style: "display: flex; gap: 8px; margin-top: 8px;",
                                    if can_cancel {
                                        button {
                                            style: "padding: 4px 12px; border-radius: 4px; border: 1px solid #ff3b30; color: #ff3b30; background: white; font-size: 12px;",
                                            onclick: {
                                                let svc = svc.clone();
                                                move |_| {
                                                    if let Err(e) = svc.cancel_job(&job_id) {
                                                        tracing::error!(error = %e, "cancel failed");
                                                    }
                                                    if let Ok(jobs) = svc.all_jobs() {
                                                        state.write().jobs = jobs;
                                                    }
                                                }
                                            },
                                            "Cancel"
                                        }
                                    }
                                    if can_release {
                                        button {
                                            style: "padding: 4px 12px; border-radius: 4px; border: 1px solid #007aff; color: #007aff; background: white; font-size: 12px;",
                                            onclick: {
                                                let svc = svc.clone();
                                                move |_| {
                                                    let svc = svc.clone();
                                                    spawn(async move {
                                                        if let Err(e) = svc.release_job(&job_id).await {
                                                            tracing::error!(error = %e, "release failed");
                                                        }
                                                        if let Ok(jobs) = svc.all_jobs() {
                                                            state.write().jobs = jobs;
                                                        }
                                                    });
                                                }
                                            },
                                            "Release"
                                        }
                                    }
                                    if is_terminal {
                                        button {
                                            style: "padding: 4px 12px; border-radius: 4px; border: 1px solid #ccc; color: #666; background: white; font-size: 12px;",
                                            onclick: {
                                                let svc = svc.clone();
                                                move |_| {
                                                    if let Err(e) = svc.delete_job(&job_id) {
                                                        tracing::error!(error = %e, "delete failed");
                                                    }
                                                    if let Ok(jobs) = svc.all_jobs() {
                                                        state.write().jobs = jobs;
                                                    }
                                                }
                                            },
                                            "Delete"
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

fn status_bg(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Pending | JobStatus::Held => "#f0f0f0",
        JobStatus::Processing | JobStatus::RetryPending => "#fff3cd",
        JobStatus::Completed => "#d4edda",
        JobStatus::Failed => "#f8d7da",
        JobStatus::Cancelled => "#e2e3e5",
    }
}

fn status_fg(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Pending | JobStatus::Held => "#333",
        JobStatus::Processing | JobStatus::RetryPending => "#856404",
        JobStatus::Completed => "#155724",
        JobStatus::Failed => "#721c24",
        JobStatus::Cancelled => "#383d41",
    }
}

fn status_text(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Pending => "Pending",
        JobStatus::Processing => "Printing...",
        JobStatus::RetryPending => "Retrying...",
        JobStatus::Completed => "Done",
        JobStatus::Failed => "Failed",
        JobStatus::Cancelled => "Cancelled",
        JobStatus::Held => "Held",
    }
}
