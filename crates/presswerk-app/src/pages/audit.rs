// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Audit page — view the append-only audit trail backed by SQLite.
//
// Form-provenance entries (issue #118, ruling D189) carry a JSON determination
// in `details`; those are rendered as a one-line human summary.

use dioxus::prelude::*;

use presswerk_core::provenance::{FfpRecord, FormProvenanceLegacy};
use presswerk_core::types::FormProvenance;
use presswerk_security::audit::AuditEntry;

use crate::services::app_services::AppServices;

#[component]
pub fn Audit() -> Element {
    let svc = use_context::<AppServices>();
    let mut entries = use_signal(Vec::<AuditEntry>::new);
    let mut total_count = use_signal(|| 0u64);

    // Load entries on mount and periodically refresh
    let svc_load = svc.clone();
    let _loader = use_resource(move || {
        let svc = svc_load.clone();
        async move {
            loop {
                if let Ok(recent) = svc.recent_audit_entries(100) {
                    entries.set(recent);
                }
                if let Ok(count) = svc.audit_count() {
                    total_count.set(count);
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    });

    rsx! {
        div {
            div { style: "display: flex; justify-content: space-between; align-items: center;",
                h1 { "Audit Trail" }
                {
                    let count = *total_count.read();
                    rsx! {
                        span { style: "color: #666; font-size: 14px;", "{count} entries" }
                    }
                }
            }
            p { style: "color: #666;",
                "Every operation is logged with a timestamp, action type, document hash, and result."
            }

            if entries.read().is_empty() {
                p { style: "text-align: center; color: #aaa; margin: 48px 0;",
                    "No audit entries yet."
                }
            } else {
                div { style: "margin-top: 16px;",
                    for entry in entries.read().iter() {
                        {
                            let success_icon = if entry.success { "\u{2705}" } else { "\u{274C}" };
                            let hash_short = if entry.document_hash.len() > 12 {
                                format!("{}...", &entry.document_hash[..12])
                            } else {
                                entry.document_hash.clone()
                            };
                            let details_text =
                                summarise_details(&entry.action, entry.details.as_deref());

                            rsx! {
                                div { style: "padding: 10px; margin: 4px 0; border: 1px solid #f0f0f0; border-radius: 6px; font-size: 14px;",
                                    div { style: "display: flex; justify-content: space-between; align-items: center;",
                                        span {
                                            "{success_icon} "
                                            strong { "{entry.action}" }
                                        }
                                        span { style: "color: #999; font-size: 12px;", "{entry.timestamp}" }
                                    }
                                    p { style: "color: #888; font-size: 12px; margin: 2px 0 0; font-family: monospace;",
                                        "{hash_short}"
                                    }
                                    if !details_text.is_empty() {
                                        p { style: "color: #666; font-size: 12px; margin: 2px 0 0;",
                                            "{details_text}"
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

/// Render an audit entry's `details` for a human.
///
/// Form-provenance entries store a JSON determination; everything else is
/// already human-readable text.
fn summarise_details(action: &str, details: Option<&str>) -> String {
    let Some(details) = details else {
        return String::new();
    };
    if action == "form_provenance" {
        if let Ok(provenance) = serde_json::from_str::<FfpRecord>(details) {
            return provenance.summary();
        }
        if let Ok(legacy) = serde_json::from_str::<FormProvenanceLegacy>(details) {
            let rec: FfpRecord = legacy.into();
            return rec.summary();
        }
        // Also try legacy alias via FormProvenance (which is now FfpRecord, but old JSON may still parse via alias fallback)
        if let Ok(provenance) = serde_json::from_str::<FormProvenance>(details) {
            return provenance.summary();
        }
    }
    details.to_string()
}
