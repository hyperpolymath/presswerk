// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Manual printer entry page.
//
// When mDNS doesn't find a printer, the user can enter the IP address
// and port manually. The app validates by probing with Get-Printer-Attributes.

use dioxus::prelude::*;

use crate::services::app_services::AppServices;
use crate::state::AppState;

#[component]
pub fn AddPrinter() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let svc = use_context::<AppServices>();
    let mut ip_input = use_signal(String::new);
    let mut port_input = use_signal(|| "631".to_string());
    let mut status_msg = use_signal(|| Option::<String>::None);
    let mut checking = use_signal(|| false);

    rsx! {
        div { style: "max-width: 500px; margin: 0 auto;",
            h1 { "Add Printer Manually" }
            p { style: "color: #666; margin-bottom: 24px;",
                "If your printer wasn't found automatically, enter its IP address below."
            }

            div { style: "margin-bottom: 16px;",
                label { style: "display: block; font-size: 16px; font-weight: bold; margin-bottom: 8px;",
                    "Printer IP Address"
                }
                input {
                    r#type: "text",
                    placeholder: "192.168.1.100",
                    value: "{ip_input}",
                    style: "width: 100%; padding: 14px; font-size: 18px; border: 2px solid #ccc; border-radius: 12px; box-sizing: border-box;",
                    oninput: move |evt| ip_input.set(evt.value().to_string()),
                }
            }

            div { style: "margin-bottom: 24px;",
                label { style: "display: block; font-size: 16px; font-weight: bold; margin-bottom: 8px;",
                    "Port (usually 631)"
                }
                input {
                    r#type: "number",
                    value: "{port_input}",
                    min: "1",
                    max: "65535",
                    style: "width: 100%; padding: 14px; font-size: 18px; border: 2px solid #ccc; border-radius: 12px; box-sizing: border-box;",
                    oninput: move |evt| port_input.set(evt.value().to_string()),
                }
            }

            button {
                style: "width: 100%; padding: 16px; border-radius: 12px; border: none; background: #007aff; color: white; font-size: 18px; font-weight: bold;",
                disabled: ip_input.read().trim().is_empty() || *checking.read(),
                onclick: {
                    let svc = svc.clone();
                    move |_| {
                        let ip_str = ip_input.read().trim().to_string();
                        let port_str = port_input.read().trim().to_string();
                        let port: u16 = port_str.parse().unwrap_or(631);

                        checking.set(true);
                        status_msg.set(Some("Checking printer...".into()));

                        let _svc = svc.clone();
                        spawn(async move {
                            // Try IPPS first (most secure), fall back to IPP
                            let ipps_uri = format!("ipps://{ip_str}:{port}/ipp/print");
                            let ipp_uri = format!("ipp://{ip_str}:{port}/ipp/print");

                            let (working_uri, using_tls) =
                                match probe_printer(&ipps_uri).await {
                                    Ok(_) => (ipps_uri, true),
                                    Err(_) => match probe_printer(&ipp_uri).await {
                                        Ok(_) => (ipp_uri, false),
                                        Err(e) => {
                                            status_msg.set(Some(format!(
                                                "Could not reach a printer at {ip_str}:{port}. \
                                                 Check the address and make sure the printer is on. ({})",
                                                e
                                            )));
                                            checking.set(false);
                                            return;
                                        }
                                    },
                                };

                            if !using_tls {
                                tracing::warn!(
                                    "printer at {ip_str}:{port} only supports plain IPP (no TLS)"
                                );
                            }

                            // Parse IP
                            let ip: std::net::IpAddr = match ip_str.parse() {
                                Ok(ip) => ip,
                                Err(_) => {
                                    status_msg.set(Some("That doesn't look like a valid IP address. Try something like 192.168.1.100.".into()));
                                    checking.set(false);
                                    return;
                                }
                            };

                            let printer = presswerk_core::types::DiscoveredPrinter {
                                name: format!("Manual: {ip_str}"),
                                uri: working_uri,
                                ip,
                                port,
                                supports_color: false, // unknown until queried
                                supports_duplex: false,
                                supports_tls: using_tls,
                                paper_sizes: Vec::new(),
                                make_and_model: None,
                                location: None,
                                last_seen: chrono::Utc::now(),
                                stale: false,
                                manually_added: true,
                            };

                            // Add to state
                            let name = printer.name.clone();
                            let uri = printer.uri.clone();
                            state.write().printers.push(printer);
                            state.write().selected_printer = Some(uri);

                            status_msg.set(Some(format!(
                                "Added {name}! {}",
                                if using_tls {
                                    "Connected securely with TLS."
                                } else {
                                    "Connected (no TLS available)."
                                }
                            )));
                            checking.set(false);
                        });
                    }
                },
                if *checking.read() { "Checking..." } else { "Add Printer" }
            }

            if let Some(ref msg) = *status_msg.read() {
                p { style: "margin-top: 16px; padding: 16px; border-radius: 12px; background: #f0f0f0; color: #333; font-size: 15px; text-align: center;",
                    "{msg}"
                }
            }

            // Help text
            div { style: "margin-top: 32px; padding: 16px; background: #f8f9fa; border-radius: 12px;",
                h3 { style: "font-size: 16px; margin: 0 0 8px 0;", "How to find your printer's IP address:" }
                ul { style: "color: #666; font-size: 14px; padding-left: 20px; margin: 0;",
                    li { "Check your printer's display or settings menu" }
                    li { "Print a \"network configuration\" page from the printer itself" }
                    li { "Check your router's admin page for connected devices" }
                    li { "Look for a sticker on the printer showing network info" }
                }
            }
        }
    }
}

/// Probe a printer by attempting Get-Printer-Attributes.
async fn probe_printer(uri: &str) -> Result<(), String> {
    let client = presswerk_print::ipp_client::IppClient::new(uri).map_err(|e| e.to_string())?;
    client
        .get_printer_attributes()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
