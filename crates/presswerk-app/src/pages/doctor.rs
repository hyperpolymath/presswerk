// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Print Doctor — guided diagnostic wizard.
//
// This is the core identity feature of Print Doctor. Walks the user through
// every step of the printing pipeline, tests each one, and provides honest
// actionable guidance — including "you need to buy X" when that's the real
// answer.

use dioxus::prelude::*;

use presswerk_print::diagnostics;

use crate::state::AppState;

/// Diagnostic wizard states.
#[derive(Debug, Clone, PartialEq)]
enum WizardState {
    /// Not yet started.
    Intro,
    /// Running the diagnostic steps.
    Running { current_step: usize },
    /// All steps completed.
    Complete,
}

#[component]
pub fn Doctor() -> Element {
    let state = use_context::<Signal<AppState>>();
    let mut wizard = use_signal(|| WizardState::Intro);
    let mut report = use_signal(|| Option::<diagnostics::DiagnosticReport>::None);

    rsx! {
        div { style: "max-width: 600px; margin: 0 auto;",
            h1 { style: "text-align: center; font-size: 28px;",
                "Print Doctor"
            }
            p { style: "text-align: center; color: #666; margin-bottom: 24px;",
                "Let's figure out what's going on with your printer."
            }

            match &*wizard.read() {
                WizardState::Intro => rsx! {
                    div { style: "text-align: center; padding: 32px 0;",
                        p { style: "font-size: 18px; margin-bottom: 24px;",
                            "We'll check everything step by step:"
                        }
                        div { style: "text-align: left; max-width: 300px; margin: 0 auto;",
                            StepPreview { num: 1, label: "Network connection" }
                            StepPreview { num: 2, label: "Finding printers" }
                            StepPreview { num: 3, label: "Reaching the printer" }
                            StepPreview { num: 4, label: "Printer language" }
                            StepPreview { num: 5, label: "Printer readiness" }
                            StepPreview { num: 6, label: "Test print" }
                        }
                        button {
                            style: "margin-top: 32px; padding: 16px 48px; border-radius: 12px; border: none; background: #007aff; color: white; font-size: 20px; font-weight: bold;",
                            onclick: {
                                let selected = state.read().selected_printer.clone();
                                move |_| {
                                    wizard.set(WizardState::Running { current_step: 0 });
                                    let selected = selected.clone();
                                    spawn(async move {
                                        let result = diagnostics::run_diagnostics(
                                            None, None,
                                            selected.as_deref(),
                                        ).await;
                                        report.set(Some(result));
                                        wizard.set(WizardState::Complete);
                                    });
                                }
                            },
                            "Start Diagnosis"
                        }
                    }
                },

                WizardState::Running { current_step } => rsx! {
                    div { style: "text-align: center; padding: 48px 0;",
                        // Spinner
                        div { style: "font-size: 48px; margin-bottom: 16px; animation: spin 1s linear infinite;",
                            "\u{1F50D}"
                        }
                        p { style: "font-size: 20px; color: #007aff;",
                            "Checking... step {current_step + 1} of 6"
                        }
                        p { style: "color: #666; font-size: 16px; margin-top: 8px;",
                            "This may take a moment."
                        }
                    }
                },

                WizardState::Complete => {
                    if let Some(ref rpt) = *report.read() {
                        let summary_bg = if rpt.failed_step.is_none() { "#d4edda" } else { "#f8d7da" };
                        let summary_fg = if rpt.failed_step.is_none() { "#155724" } else { "#721c24" };
                        rsx! {
                            // Summary card
                            div {
                                style: "padding: 24px; border-radius: 16px; margin-bottom: 24px; background: {summary_bg};",
                                p { style: "font-size: 20px; font-weight: bold; color: {summary_fg}; margin: 0;",
                                    "{rpt.summary}"
                                }
                            }

                            // Step results
                            for (i, step) in rpt.steps.iter().enumerate() {
                                {
                                    let icon = if step.passed { "\u{2705}" } else { "\u{274C}" };
                                    let border = if step.passed { "#d4edda" } else { "#f8d7da" };
                                    rsx! {
                                        div {
                                            style: "padding: 16px; margin: 8px 0; border: 2px solid {border}; border-radius: 12px;",
                                            div { style: "display: flex; align-items: center; gap: 12px;",
                                                span { style: "font-size: 24px;", "{icon}" }
                                                div {
                                                    strong { style: "font-size: 16px;",
                                                        "Step {i + 1}: {step.name}"
                                                    }
                                                    p { style: "color: #666; font-size: 14px; margin: 4px 0 0 0;",
                                                        "{step.detail}"
                                                    }
                                                }
                                            }
                                            if !step.passed {
                                                if let Some(ref fix) = step.fix {
                                                    div { style: "margin-top: 12px; padding: 12px; background: #fff3cd; border-radius: 8px;",
                                                        strong { style: "color: #856404; font-size: 14px;",
                                                            "What to do: "
                                                        }
                                                        span { style: "color: #856404; font-size: 14px;",
                                                            "{fix}"
                                                        }
                                                    }
                                                }
                                                if let Some(ref esc) = step.escalation {
                                                    details { style: "margin-top: 8px; font-size: 14px; color: #666;",
                                                        summary { style: "cursor: pointer; color: #007aff;",
                                                            "What does this mean?"
                                                        }
                                                        p { style: "margin-top: 8px;", "{esc}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Action buttons
                            div { style: "display: flex; gap: 12px; margin-top: 24px;",
                                button {
                                    style: "flex: 1; padding: 14px; border-radius: 12px; border: 1px solid #007aff; color: #007aff; background: white; font-size: 16px; font-weight: bold;",
                                    onclick: move |_| {
                                        wizard.set(WizardState::Intro);
                                        report.set(None);
                                    },
                                    "Run Again"
                                }
                                button {
                                    style: "flex: 1; padding: 14px; border-radius: 12px; border: none; background: #007aff; color: white; font-size: 16px; font-weight: bold;",
                                    onclick: {
                                        let rpt = rpt.clone();
                                        move |_| {
                                            let summary = diagnostics::generate_help_summary(&rpt);
                                            // Copy to clipboard via JS interop or share sheet
                                            tracing::info!(summary = %summary, "help summary generated");
                                            // For now, log it — platform sharing in v0.3
                                        }
                                    },
                                    "I Need Help"
                                }
                            }
                        }
                    } else {
                        rsx! {
                            p { "Loading results..." }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn StepPreview(num: u8, label: &'static str) -> Element {
    rsx! {
        div { style: "display: flex; align-items: center; gap: 12px; padding: 8px 0;",
            span { style: "width: 28px; height: 28px; border-radius: 50%; background: #e0e0e0; display: flex; align-items: center; justify-content: center; font-size: 14px; font-weight: bold; color: #666;",
                "{num}"
            }
            span { style: "font-size: 16px; color: #333;", "{label}" }
        }
    }
}
