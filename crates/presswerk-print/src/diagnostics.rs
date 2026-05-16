// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// End-to-end print pipeline diagnostics.
//
// Runs a sequence of checks: network → discovery → reachability → IPP support
// → printer readiness → test print. Stops at the first failure and provides
// a human-readable diagnosis with actionable guidance.

use std::net::{IpAddr, TcpStream};
use std::time::Duration;

/// Result of a single diagnostic step.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// Step name shown to the user.
    pub name: String,
    /// Whether the step passed.
    pub passed: bool,
    /// Human-readable detail of what was tested.
    pub detail: String,
    /// What to do if the step failed.
    pub fix: Option<String>,
    /// Escalation message for problems that need external help.
    pub escalation: Option<String>,
}

/// Full diagnostic report.
#[derive(Debug, Clone)]
pub struct DiagnosticReport {
    /// The sequential step results.
    pub steps: Vec<StepResult>,
    /// The step that failed (if any).
    pub failed_step: Option<usize>,
    /// Overall summary.
    pub summary: String,
    /// Device info for the help export.
    pub device_info: DeviceInfo,
    /// Printer info (if discovered).
    pub printer_info: Option<PrinterInfo>,
}

/// Device information for the diagnostic report.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub platform: String,
    pub wifi_network: Option<String>,
}

/// Printer information discovered during diagnostics.
#[derive(Debug, Clone)]
pub struct PrinterInfo {
    pub name: String,
    pub ip: IpAddr,
    pub port: u16,
    pub model: Option<String>,
    pub status: Option<String>,
    pub status_reasons: Vec<String>,
}

/// Run the full diagnostic pipeline.
///
/// Each step depends on the previous one succeeding.
/// Returns as soon as a step fails, with guidance for the user.
pub async fn run_diagnostics(
    printer_ip: Option<IpAddr>,
    printer_port: Option<u16>,
    printer_uri: Option<&str>,
) -> DiagnosticReport {
    let mut report = DiagnosticReport {
        steps: Vec::new(),
        failed_step: None,
        summary: String::new(),
        device_info: detect_device_info(),
        printer_info: None,
    };

    // Step 1: Network Check
    let network_ok = check_network();
    report.steps.push(network_ok.clone());
    if !network_ok.passed {
        report.failed_step = Some(0);
        report.summary = "No network connection found.".into();
        return report;
    }

    // Step 2: Printer Discovery
    let discovery = check_discovery().await;
    report.steps.push(discovery.clone());
    if !discovery.passed && printer_ip.is_none() {
        report.failed_step = Some(1);
        report.summary = "No printers found on your network.".into();
        return report;
    }

    // Step 3: Printer Reachable
    let ip = printer_ip.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    let port = printer_port.unwrap_or(631);
    let reachable = check_reachable(ip, port);
    report.steps.push(reachable.clone());
    if !reachable.passed {
        report.failed_step = Some(2);
        report.summary = "Printer found but not responding.".into();
        return report;
    }

    // Step 4: IPP Support
    let uri = printer_uri
        .map(String::from)
        .unwrap_or_else(|| format!("ipp://{}:{}/ipp/print", ip, port));
    let ipp = check_ipp_support(&uri).await;
    report.steps.push(ipp.clone());
    if !ipp.passed {
        report.failed_step = Some(3);
        report.summary = "Printer doesn't support modern printing protocol.".into();
        return report;
    }

    // Step 5: Printer Ready
    let ready = check_printer_ready(&uri, &mut report).await;
    report.steps.push(ready.clone());
    if !ready.passed {
        report.failed_step = Some(4);
        report.summary = ready.detail.clone();
        return report;
    }

    // Step 6: Test Print
    let test = send_test_print(&uri).await;
    report.steps.push(test.clone());
    if !test.passed {
        report.failed_step = Some(5);
        report.summary = "Test page couldn't be sent.".into();
        return report;
    }

    report.summary = "Everything looks good! Your printer is ready.".into();
    report
}

/// Generate a shareable text summary for sending to a tech-savvy helper.
pub fn generate_help_summary(report: &DiagnosticReport) -> String {
    let now = chrono::Utc::now().format("%d %b %Y, %l:%M %p");
    let mut text = format!("Print Doctor Report\nDate: {now}\n");
    text.push_str(&format!("Device: {}\n", report.device_info.platform));

    if let Some(ref wifi) = report.device_info.wifi_network {
        text.push_str(&format!("Wi-Fi: {wifi} (connected)\n"));
    } else {
        text.push_str("Wi-Fi: Not connected\n");
    }

    if let Some(ref printer) = report.printer_info {
        text.push_str(&format!("Printer: {}\n", printer.name));
        text.push_str(&format!("IP: {}:{}\n", printer.ip, printer.port));
        if let Some(ref model) = printer.model {
            text.push_str(&format!("Model: {model}\n"));
        }
        if let Some(ref status) = printer.status {
            text.push_str(&format!("Status: {status}\n"));
        }
        for reason in &printer.status_reasons {
            text.push_str(&format!("Issue: {reason}\n"));
        }
    }

    text.push('\n');

    if let Some(idx) = report.failed_step {
        let step = &report.steps[idx];
        text.push_str(&format!("FAILED AT: Step {} — {}\n", idx + 1, step.name));
        text.push_str(&format!("What happened: {}\n", step.detail));
        if let Some(ref fix) = step.fix {
            text.push_str(&format!("What to do: {fix}\n"));
        }
        if let Some(ref esc) = step.escalation {
            text.push_str(&format!("If that doesn't work: {esc}\n"));
        }
    } else {
        text.push_str("All checks passed. Printer is working.\n");
    }

    text
}

// -- Step implementations ---------------------------------------------------

fn check_network() -> StepResult {
    // Check for any non-loopback network interface
    let has_network = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:53")?;
            s.local_addr()
        })
        .map(|addr| !addr.ip().is_loopback())
        .unwrap_or(false);

    if has_network {
        StepResult {
            name: "Network Check".into(),
            passed: true,
            detail: "Your device is connected to a network.".into(),
            fix: None,
            escalation: None,
        }
    } else {
        StepResult {
            name: "Network Check".into(),
            passed: false,
            detail: "No network connection found.".into(),
            fix: Some(
                "Connect to your home Wi-Fi network. Go to Settings \u{2192} Wi-Fi on your phone."
                    .into(),
            ),
            escalation: None,
        }
    }
}

async fn check_discovery() -> StepResult {
    // Try mDNS browse for 15 seconds
    match presswerk_core::error::Result::Ok(()) {
        Ok(()) => {
            let discovery = crate::discovery::PrinterDiscovery::new();
            match discovery {
                Ok(mut disc) => {
                    let printers = disc.discover(Some(Duration::from_secs(15)));
                    match printers {
                        Ok(list) if !list.is_empty() => StepResult {
                            name: "Printer Discovery".into(),
                            passed: true,
                            detail: format!("Found {} printer(s) on your network.", list.len()),
                            fix: None,
                            escalation: None,
                        },
                        _ => StepResult {
                            name: "Printer Discovery".into(),
                            passed: false,
                            detail: "No printers found on your network.".into(),
                            fix: Some("Make sure your printer is turned on and connected to the same Wi-Fi network as your phone. Check the printer's display or lights.".into()),
                            escalation: Some("If your printer only connects via USB cable and doesn't have Wi-Fi, you'll need a USB OTG adapter cable for your phone (about \u{00A3}5-10 from any electronics shop).".into()),
                        },
                    }
                }
                Err(_) => StepResult {
                    name: "Printer Discovery".into(),
                    passed: false,
                    detail: "Could not start printer search.".into(),
                    fix: Some("Make sure you're connected to Wi-Fi, then try again.".into()),
                    escalation: None,
                },
            }
        }
        Err(_) => unreachable!(),
    }
}

fn check_reachable(ip: IpAddr, port: u16) -> StepResult {
    let addr = std::net::SocketAddr::new(ip, port);
    match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
        Ok(_) => StepResult {
            name: "Printer Reachable".into(),
            passed: true,
            detail: format!("Printer is responding at {ip}:{port}."),
            fix: None,
            escalation: None,
        },
        Err(_) => StepResult {
            name: "Printer Reachable".into(),
            passed: false,
            detail: format!("Printer at {ip}:{port} is not responding."),
            fix: Some("The printer was seen on the network but isn't answering. Try turning it off, waiting 10 seconds, and turning it back on.".into()),
            escalation: Some("If the printer has a small screen, check if it shows any error messages.".into()),
        },
    }
}

async fn check_ipp_support(uri: &str) -> StepResult {
    match crate::ipp_client::IppClient::new(uri) {
        Ok(client) => match client.get_printer_attributes().await {
            Ok(_attrs) => StepResult {
                name: "Printer Speaks IPP".into(),
                passed: true,
                detail: "Printer supports IPP printing.".into(),
                fix: None,
                escalation: None,
            },
            Err(e) => {
                let detail = e.to_string();
                if detail.contains("timed out") {
                    StepResult {
                        name: "Printer Speaks IPP".into(),
                        passed: false,
                        detail: "Printer took too long to respond to IPP query.".into(),
                        fix: Some("The printer may be busy. Try again in a minute.".into()),
                        escalation: None,
                    }
                } else {
                    StepResult {
                        name: "Printer Speaks IPP".into(),
                        passed: false,
                        detail: "Printer doesn't support modern printing protocol.".into(),
                        fix: Some("This is an older printer. We'll try other ways to talk to it.".into()),
                        escalation: Some("This printer may need a driver installed on a computer. Some very old printers can only work when connected directly to a computer with the manufacturer's software.".into()),
                    }
                }
            }
        },
        Err(_) => StepResult {
            name: "Printer Speaks IPP".into(),
            passed: false,
            detail: "The printer address isn't valid.".into(),
            fix: Some("Check the printer address and try again.".into()),
            escalation: None,
        },
    }
}

async fn check_printer_ready(uri: &str, report: &mut DiagnosticReport) -> StepResult {
    let client = match crate::ipp_client::IppClient::new(uri) {
        Ok(c) => c,
        Err(_) => {
            return StepResult {
                name: "Printer Ready".into(),
                passed: false,
                detail: "Could not connect to the printer.".into(),
                fix: Some("Try the previous steps again.".into()),
                escalation: None,
            };
        }
    };

    let attrs = match client.get_printer_attributes().await {
        Ok(a) => a,
        Err(_) => {
            return StepResult {
                name: "Printer Ready".into(),
                passed: false,
                detail: "Could not query printer status.".into(),
                fix: Some("The printer may be busy. Try again in a moment.".into()),
                escalation: None,
            };
        }
    };

    let state = attrs
        .get("printer-state")
        .cloned()
        .unwrap_or_else(|| "unknown".into());
    let reasons: Vec<String> = attrs
        .get("printer-state-reasons")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| s != "none")
                .collect()
        })
        .unwrap_or_default();

    let name = attrs
        .get("printer-name")
        .or_else(|| attrs.get("printer-make-and-model"))
        .cloned()
        .unwrap_or_else(|| "Unknown Printer".into());

    // Populate printer info in the report
    report.printer_info = Some(PrinterInfo {
        name: name.clone(),
        ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), // will be overridden by caller
        port: 631,
        model: attrs.get("printer-make-and-model").cloned(),
        status: Some(state.clone()),
        status_reasons: reasons.clone(),
    });

    // Interpret printer state
    if state.contains('3') || state.to_ascii_lowercase().contains("idle") {
        StepResult {
            name: "Printer Ready".into(),
            passed: true,
            detail: format!("{name} is ready to print!"),
            fix: None,
            escalation: None,
        }
    } else if state.contains('4') || state.to_ascii_lowercase().contains("processing") {
        StepResult {
            name: "Printer Ready".into(),
            passed: true,
            detail: format!("{name} is busy with another job. Your document will print next."),
            fix: None,
            escalation: None,
        }
    } else {
        // Printer is stopped — check reasons
        let (detail, fix, escalation) = interpret_stop_reasons(&name, &reasons);
        StepResult {
            name: "Printer Ready".into(),
            passed: false,
            detail,
            fix: Some(fix),
            escalation,
        }
    }
}

/// Interpret printer-state-reasons into human messages.
fn interpret_stop_reasons(name: &str, reasons: &[String]) -> (String, String, Option<String>) {
    for reason in reasons {
        let lower = reason.to_ascii_lowercase();
        if lower.contains("media-empty") || lower.contains("paper") && lower.contains("empty") {
            return (
                format!("{name} is out of paper."),
                "Please add paper to the printer's tray.".into(),
                None,
            );
        }
        if lower.contains("toner-empty") || lower.contains("marker-supply") || lower.contains("ink")
        {
            return (
                format!("{name} needs new ink or toner."),
                "You'll need to buy a replacement cartridge. Check the printer model number and search online.".into(),
                Some("Search for your printer model followed by 'ink cartridge' or 'toner cartridge'.".into()),
            );
        }
        if lower.contains("door-open") || lower.contains("cover-open") {
            return (
                format!("A door or cover is open on {name}."),
                "Please close all doors and covers on the printer.".into(),
                None,
            );
        }
        if lower.contains("paper-jam") || lower.contains("media-jam") {
            return (
                format!("Paper is stuck in {name}."),
                "Gently pull the stuck paper out. Check there are no torn pieces left inside, and close all doors.".into(),
                Some("If this keeps happening, the rollers inside the printer may need cleaning.".into()),
            );
        }
    }

    // Generic stop
    (
        format!("{name} has stopped."),
        "Try turning the printer off, waiting 10 seconds, and turning it back on.".into(),
        None,
    )
}

async fn send_test_print(uri: &str) -> StepResult {
    let client = match crate::ipp_client::IppClient::new(uri) {
        Ok(c) => c,
        Err(_) => {
            return StepResult {
                name: "Test Print".into(),
                passed: false,
                detail: "Could not connect for test print.".into(),
                fix: Some("Try the previous steps again.".into()),
                escalation: None,
            };
        }
    };

    let test_doc = b"Print Doctor Test Page\n\nIf you can read this, your printer is working correctly!\n\nPrinted by Presswerk Print Doctor.\n";
    let settings = presswerk_core::types::PrintSettings::default();

    match client
        .print_job(
            test_doc.to_vec(),
            presswerk_core::types::DocumentType::PlainText,
            "Print Doctor Test Page",
            &settings,
        )
        .await
    {
        Ok(_) => StepResult {
            name: "Test Print".into(),
            passed: true,
            detail: "Test page sent successfully! Check your printer \u{2014} a page should be coming out now.".into(),
            fix: None,
            escalation: None,
        },
        Err(e) => {
            let human = presswerk_core::human_errors::humanize_error(&e);
            StepResult {
                name: "Test Print".into(),
                passed: false,
                detail: "The test page couldn't be sent.".into(),
                fix: Some(format!("{} {}", human.message, human.suggestion)),
                escalation: None,
            }
        }
    }
}

fn detect_device_info() -> DeviceInfo {
    let platform = if cfg!(target_os = "ios") {
        "iOS"
    } else if cfg!(target_os = "android") {
        "Android"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Unknown"
    };

    DeviceInfo {
        platform: platform.into(),
        wifi_network: None, // would need platform bridge for real network name
    }
}
