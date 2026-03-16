// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// mDNS service discovery for IPP and IPPS printers on the local network.
//
// We browse for `_ipp._tcp.local.` (plain IPP, port 631) and
// `_ipps._tcp.local.` (TLS-secured IPP) using the `mdns-sd` crate.  Resolved
// services are converted into `DiscoveredPrinter` values that the rest of the
// application can consume.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tracing::{debug, info, warn};

use presswerk_core::error::{PresswerkError, Result};
use presswerk_core::types::DiscoveredPrinter;

/// mDNS service type for plain IPP.
const IPP_SERVICE: &str = "_ipp._tcp.local.";

/// mDNS service type for TLS-secured IPP.
const IPPS_SERVICE: &str = "_ipps._tcp.local.";

/// Default browse duration before the initial snapshot is returned.
/// Increased from 5s to 15s to catch slow printers.
const DEFAULT_BROWSE_TIMEOUT: Duration = Duration::from_secs(15);

/// Printer discovery engine using mDNS-SD.
///
/// Wraps an `mdns-sd` `ServiceDaemon` that continuously browses for IPP and
/// IPPS services.  Discovered printers are accumulated in a thread-safe map
/// keyed by their full service name so that duplicate events are deduplicated
/// automatically.
pub struct PrinterDiscovery {
    /// The underlying mDNS daemon handle.
    daemon: ServiceDaemon,
    /// Thread-safe map of discovered printers keyed by mDNS full-name.
    printers: Arc<Mutex<HashMap<String, DiscoveredPrinter>>>,
    /// Whether we are currently browsing.
    browsing: bool,
}

impl PrinterDiscovery {
    /// Create a new discovery engine.
    ///
    /// This spawns the mDNS daemon thread but does **not** start browsing.
    /// Call [`start`] to begin service discovery.
    pub fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new()
            .map_err(|e| PresswerkError::Discovery(format!("failed to start mDNS daemon: {e}")))?;
        Ok(Self {
            daemon,
            printers: Arc::new(Mutex::new(HashMap::new())),
            browsing: false,
        })
    }

    /// Start browsing for IPP and IPPS printers.
    ///
    /// Returns immediately.  Discovered printers are accumulated internally and
    /// can be retrieved with [`printers`].  Background `flume` receiver threads
    /// are spawned for each service type.
    pub fn start(&mut self) -> Result<()> {
        if self.browsing {
            debug!("printer discovery already running");
            return Ok(());
        }

        let ipp_receiver = self
            .daemon
            .browse(IPP_SERVICE)
            .map_err(|e| PresswerkError::Discovery(format!("browse {IPP_SERVICE}: {e}")))?;

        let ipps_receiver = self
            .daemon
            .browse(IPPS_SERVICE)
            .map_err(|e| PresswerkError::Discovery(format!("browse {IPPS_SERVICE}: {e}")))?;

        // Spawn a background thread per service type to drain the receiver
        // channel and update the shared printer map.
        Self::spawn_listener(IPP_SERVICE, false, ipp_receiver, Arc::clone(&self.printers));
        Self::spawn_listener(
            IPPS_SERVICE,
            true,
            ipps_receiver,
            Arc::clone(&self.printers),
        );

        self.browsing = true;
        info!("mDNS printer discovery started");
        Ok(())
    }

    /// Stop browsing for printers.
    pub fn stop(&mut self) -> Result<()> {
        if !self.browsing {
            return Ok(());
        }

        self.daemon
            .stop_browse(IPP_SERVICE)
            .map_err(|e| PresswerkError::Discovery(format!("stop browse {IPP_SERVICE}: {e}")))?;
        self.daemon
            .stop_browse(IPPS_SERVICE)
            .map_err(|e| PresswerkError::Discovery(format!("stop browse {IPPS_SERVICE}: {e}")))?;

        self.browsing = false;
        info!("mDNS printer discovery stopped");
        Ok(())
    }

    /// Shut down the mDNS daemon entirely.
    ///
    /// After calling this the `PrinterDiscovery` instance cannot be reused.
    pub fn shutdown(self) -> Result<()> {
        let _status_rx = self
            .daemon
            .shutdown()
            .map_err(|e| PresswerkError::Discovery(format!("daemon shutdown: {e}")))?;
        info!("mDNS daemon shut down");
        Ok(())
    }

    /// Return a snapshot of all currently discovered printers.
    pub fn printers(&self) -> Vec<DiscoveredPrinter> {
        self.printers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// Browse the network for printers, wait up to `timeout` for initial
    /// results, then return whatever has been found.
    ///
    /// This is a convenience wrapper combining [`start`], a sleep, and
    /// [`printers`].  Discovery continues running in the background
    /// after this call returns.
    pub fn discover(&mut self, timeout: Option<Duration>) -> Result<Vec<DiscoveredPrinter>> {
        self.start()?;
        std::thread::sleep(timeout.unwrap_or(DEFAULT_BROWSE_TIMEOUT));
        Ok(self.printers())
    }

    /// Whether the discovery engine is currently browsing.
    pub fn is_browsing(&self) -> bool {
        self.browsing
    }

    // -- internal helpers ---------------------------------------------------

    /// Spawn a thread that drains the `flume::Receiver<ServiceEvent>` produced
    /// by `ServiceDaemon::browse` and populates the shared printer map.
    fn spawn_listener(
        service_type: &'static str,
        tls: bool,
        receiver: mdns_sd::Receiver<ServiceEvent>,
        printers: Arc<Mutex<HashMap<String, DiscoveredPrinter>>>,
    ) {
        std::thread::Builder::new()
            .name(format!("mdns-{service_type}"))
            .spawn(move || {
                // Block on the receiver until the channel is closed (which
                // happens when the daemon is shut down or browsing is stopped).
                while let Ok(event) = receiver.recv() {
                    match event {
                        ServiceEvent::SearchStarted(stype) => {
                            debug!(service_type = %stype, "mDNS search started");
                        }
                        ServiceEvent::ServiceFound(stype, fullname) => {
                            debug!(service_type = %stype, name = %fullname, "service found");
                        }
                        ServiceEvent::ServiceResolved(info) => {
                            let fullname = info.get_fullname().to_owned();
                            match service_info_to_printer(&info, tls) {
                                Ok(printer) => {
                                    info!(
                                        name = %printer.name,
                                        uri = %printer.uri,
                                        "printer resolved"
                                    );
                                    printers
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .insert(fullname, printer);
                                }
                                Err(e) => {
                                    warn!(
                                        fullname = %fullname,
                                        error = %e,
                                        "failed to convert resolved service to printer"
                                    );
                                }
                            }
                        }
                        ServiceEvent::ServiceRemoved(stype, fullname) => {
                            info!(service_type = %stype, name = %fullname, "printer removed");
                            printers
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .remove(&fullname);
                        }
                        ServiceEvent::SearchStopped(stype) => {
                            debug!(service_type = %stype, "mDNS search stopped");
                            break;
                        }
                    }
                }
            })
            .expect("failed to spawn mDNS listener thread");
    }
}

/// Convert a resolved `ServiceInfo` into a `DiscoveredPrinter`.
///
/// TXT record keys (case-insensitive) commonly found on IPP printers:
///   - `printer-make-and-model` — human-readable make/model string
///   - `printer-location`       — physical location
///   - `Color`                  — "T" or "F"
///   - `Duplex`                 — "T" or "F"
///   - `rp`                     — resource path (e.g. "ipp/print")
fn service_info_to_printer(info: &ServiceInfo, tls: bool) -> Result<DiscoveredPrinter> {
    let name = info.get_fullname().to_owned();
    let port = info.get_port();

    // Pick the first address — prefer IPv4 for wider printer compatibility.
    let ip: IpAddr = info
        .get_addresses()
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| info.get_addresses().iter().next())
        .copied()
        .ok_or_else(|| PresswerkError::Discovery(format!("no address for service {name}")))?;

    // Build the IPP URI from TXT `rp` key or fall back to "ipp/print".
    let resource_path = info.get_property_val_str("rp").unwrap_or("ipp/print");

    let scheme = if tls { "ipps" } else { "ipp" };
    let uri = format!("{scheme}://{ip}:{port}/{resource_path}");

    // Parse capability flags from TXT records.
    let supports_color = txt_bool(info, "Color");
    let supports_duplex = txt_bool(info, "Duplex");

    let make_and_model = info
        .get_property_val_str("printer-make-and-model")
        .map(String::from);
    let location = info
        .get_property_val_str("printer-location")
        .map(String::from);

    Ok(DiscoveredPrinter {
        name,
        uri,
        ip,
        port,
        supports_color,
        supports_duplex,
        supports_tls: tls,
        paper_sizes: Vec::new(), // determined later via Get-Printer-Attributes
        make_and_model,
        location,
        last_seen: Utc::now(),
        stale: false,
        manually_added: false,
    })
}

/// Read a boolean TXT record value.  IPP Everywhere uses "T"/"F".
fn txt_bool(info: &ServiceInfo, key: &str) -> bool {
    info.get_property_val_str(key)
        .map(|v| v.eq_ignore_ascii_case("t") || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn txt_bool_logic_parses_true_variants() {
        // Tests the boolean-parsing logic used by `txt_bool`.
        // Full integration with `ServiceInfo` requires a live mDNS network.
        let parse = |v: &str| v.eq_ignore_ascii_case("t") || v.eq_ignore_ascii_case("true");
        assert!(parse("T"));
        assert!(parse("t"));
        assert!(parse("true"));
        assert!(parse("TRUE"));
        assert!(!parse("F"));
        assert!(!parse("false"));
        assert!(!parse(""));
    }
}
