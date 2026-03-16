// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Application configuration.

use serde::{Deserialize, Serialize};

/// Persistent application settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Default paper size for new print jobs.
    pub default_paper_size: crate::PaperSize,
    /// Whether the IPP print server starts automatically on launch.
    pub auto_start_server: bool,
    /// Port for the IPP print server (default 631).
    pub server_port: u16,
    /// Require TLS for print server connections.
    pub server_require_tls: bool,
    /// Auto-accept incoming network print jobs (if false, jobs are held for review).
    pub auto_accept_network_jobs: bool,
    /// Enable audit trail logging.
    pub audit_enabled: bool,
    /// Enable encrypted local storage.
    pub encryption_enabled: bool,
    /// Timeout for print operations (seconds).
    pub print_timeout_secs: u64,
    /// Timeout for query operations like Get-Printer-Attributes (seconds).
    pub query_timeout_secs: u64,
    /// Whether Easy Mode is the default interface.
    pub easy_mode: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_paper_size: crate::PaperSize::A4,
            auto_start_server: false,
            server_port: 631,
            server_require_tls: true,
            auto_accept_network_jobs: false,
            audit_enabled: true,
            encryption_enabled: true,
            print_timeout_secs: 60,
            query_timeout_secs: 15,
            easy_mode: true,
        }
    }
}
