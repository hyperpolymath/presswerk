// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Printer health tracking with circuit breaker pattern.
//
// If a printer is repeatedly failing, stop hammering it with requests that
// will just time out. Instead, short-circuit immediately and tell the user
// the printer is having trouble. Periodically allow a probe request through
// to check if the printer has recovered.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation — requests pass through.
    Closed,
    /// Too many failures — requests are blocked. Cooldown timer running.
    Open,
    /// Cooldown expired — allow one probe request through to test recovery.
    HalfOpen,
}

/// Health status for a single printer.
#[derive(Debug, Clone)]
pub struct PrinterHealth {
    /// Current circuit breaker state.
    pub state: CircuitState,
    /// Number of consecutive failures.
    pub consecutive_failures: u32,
    /// When the circuit was opened (for cooldown calculation).
    pub opened_at: Option<Instant>,
    /// Last successful operation timestamp.
    pub last_success: Option<Instant>,
    /// Last failure message.
    pub last_error: Option<String>,
}

impl Default for PrinterHealth {
    fn default() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            last_success: None,
            last_error: None,
        }
    }
}

/// Manages health tracking for all known printers.
pub struct HealthTracker {
    /// Per-printer health keyed by printer URI.
    printers: HashMap<String, PrinterHealth>,
    /// Number of failures before opening the circuit.
    failure_threshold: u32,
}

impl Default for HealthTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthTracker {
    pub fn new() -> Self {
        Self {
            printers: HashMap::new(),
            failure_threshold: 3,
        }
    }

    /// Check whether a request to this printer should be allowed through.
    ///
    /// Returns `true` if the circuit is closed or half-open (probe allowed).
    /// Returns `false` if the circuit is open (cooldown still active).
    pub fn allow_request(&mut self, printer_uri: &str) -> bool {
        let health = self
            .printers
            .entry(printer_uri.to_string())
            .or_default();

        match health.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if cooldown has expired
                if let Some(opened_at) = health.opened_at {
                    let cooldown = cooldown_duration(health.consecutive_failures);
                    if opened_at.elapsed() >= cooldown {
                        info!(
                            uri = printer_uri,
                            "circuit half-open — allowing probe request"
                        );
                        health.state = CircuitState::HalfOpen;
                        true
                    } else {
                        debug!(
                            uri = printer_uri,
                            remaining_ms = (cooldown - opened_at.elapsed()).as_millis(),
                            "circuit open — blocking request"
                        );
                        false
                    }
                } else {
                    // No timestamp — shouldn't happen, close the circuit
                    health.state = CircuitState::Closed;
                    true
                }
            }
            CircuitState::HalfOpen => {
                // Already let one probe through — block further requests
                // until the probe completes
                false
            }
        }
    }

    /// Record a successful operation for this printer.
    pub fn record_success(&mut self, printer_uri: &str) {
        let health = self
            .printers
            .entry(printer_uri.to_string())
            .or_default();

        if health.state != CircuitState::Closed {
            info!(
                uri = printer_uri,
                prev_state = ?health.state,
                "printer recovered — closing circuit"
            );
        }

        health.state = CircuitState::Closed;
        health.consecutive_failures = 0;
        health.opened_at = None;
        health.last_success = Some(Instant::now());
        health.last_error = None;
    }

    /// Record a failed operation for this printer.
    pub fn record_failure(&mut self, printer_uri: &str, error: &str) {
        let health = self
            .printers
            .entry(printer_uri.to_string())
            .or_default();

        health.consecutive_failures += 1;
        health.last_error = Some(error.to_string());

        if health.consecutive_failures >= self.failure_threshold
            && health.state != CircuitState::Open
        {
            warn!(
                uri = printer_uri,
                failures = health.consecutive_failures,
                "opening circuit breaker for printer"
            );
            health.state = CircuitState::Open;
            health.opened_at = Some(Instant::now());
        } else if health.state == CircuitState::HalfOpen {
            // Probe failed — back to open with extended cooldown
            warn!(
                uri = printer_uri,
                "probe failed — reopening circuit breaker"
            );
            health.state = CircuitState::Open;
            health.opened_at = Some(Instant::now());
        }
    }

    /// Get the health status for a printer (if tracked).
    pub fn get_health(&self, printer_uri: &str) -> Option<&PrinterHealth> {
        self.printers.get(printer_uri)
    }

    /// Get a human-readable status message for the printer.
    pub fn status_message(&self, printer_uri: &str) -> Option<String> {
        let health = self.printers.get(printer_uri)?;
        match health.state {
            CircuitState::Closed => None,
            CircuitState::Open => {
                let cooldown = cooldown_duration(health.consecutive_failures);
                let remaining = health
                    .opened_at
                    .map(|t| cooldown.saturating_sub(t.elapsed()))
                    .unwrap_or(Duration::ZERO);
                Some(format!(
                    "This printer seems to be having trouble ({} failures). We'll try again in {} seconds.",
                    health.consecutive_failures,
                    remaining.as_secs()
                ))
            }
            CircuitState::HalfOpen => {
                Some("Checking if the printer has recovered...".into())
            }
        }
    }
}

/// Calculate cooldown duration based on failure count.
///
/// 3 failures: 30 seconds
/// 5 failures: 2 minutes
/// 10+ failures: 5 minutes
fn cooldown_duration(failures: u32) -> Duration {
    if failures >= 10 {
        Duration::from_secs(300) // 5 minutes
    } else if failures >= 5 {
        Duration::from_secs(120) // 2 minutes
    } else {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_printer_allows_requests() {
        let mut tracker = HealthTracker::new();
        assert!(tracker.allow_request("ipp://test:631/"));
    }

    #[test]
    fn circuit_opens_after_threshold() {
        let mut tracker = HealthTracker::new();
        let uri = "ipp://test:631/";

        tracker.record_failure(uri, "timeout");
        tracker.record_failure(uri, "timeout");
        assert!(tracker.allow_request(uri)); // 2 failures < threshold

        tracker.record_failure(uri, "timeout"); // 3rd failure = threshold
        assert!(!tracker.allow_request(uri)); // circuit open
    }

    #[test]
    fn success_resets_circuit() {
        let mut tracker = HealthTracker::new();
        let uri = "ipp://test:631/";

        for _ in 0..5 {
            tracker.record_failure(uri, "error");
        }
        assert!(!tracker.allow_request(uri));

        tracker.record_success(uri);
        assert!(tracker.allow_request(uri));
        let Some(health) = tracker.get_health(uri) else {
            panic!("no health status for {uri}");
        };
        assert_eq!(health.consecutive_failures, 0);
    }

    #[test]
    fn status_message_when_open() {
        let mut tracker = HealthTracker::new();
        let uri = "ipp://test:631/";

        for _ in 0..3 {
            tracker.record_failure(uri, "timeout");
        }

        let Some(msg) = tracker.status_message(uri) else {
            panic!("expected status message for {uri}");
        };
        assert!(msg.contains("having trouble"));
    }

    #[test]
    fn no_status_message_when_healthy() {
        let mut tracker = HealthTracker::new();
        let uri = "ipp://test:631/";
        tracker.record_success(uri);
        assert!(tracker.status_message(uri).is_none());
    }
}
