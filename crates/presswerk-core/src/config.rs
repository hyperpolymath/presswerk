// SPDX-License-Identifier: MPL-2.0
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
    /// How the print path treats a machine-filled form (issue #118).
    ///
    /// Defaults to `Record`: every PDF that arrives for printing is inspected,
    /// the determination is stored on the job and written to the audit trail,
    /// and nothing is held. `HoldForReview` additionally parks machine-filled
    /// jobs in `Held` until the user releases them.
    ///
    /// `#[serde(default)]` keeps configuration files written before this
    /// setting existed loadable rather than silently resetting every setting.
    #[serde(default)]
    pub form_provenance_policy: crate::FormProvenancePolicy,
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
            form_provenance_policy: crate::FormProvenancePolicy::Record,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.server_port, 631);
        assert_eq!(config.print_timeout_secs, 60);
        assert_eq!(config.query_timeout_secs, 15);
        assert!(!config.auto_start_server);
        assert!(config.server_require_tls);
        assert!(!config.auto_accept_network_jobs);
        assert!(config.audit_enabled);
        assert!(config.encryption_enabled);
        assert!(config.easy_mode);
    }

    #[test]
    fn test_config_serialization() {
        let config = AppConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.is_empty());

        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(config.server_port, restored.server_port);
        assert_eq!(config.print_timeout_secs, restored.print_timeout_secs);
    }

    #[test]
    fn test_custom_config() {
        let config = AppConfig {
            server_port: 9631,
            auto_start_server: true,
            server_require_tls: false,
            print_timeout_secs: 120,
            query_timeout_secs: 30,
            easy_mode: false,
            ..Default::default()
        };

        assert_eq!(config.server_port, 9631);
        assert!(config.auto_start_server);
        assert!(!config.server_require_tls);
        assert_eq!(config.print_timeout_secs, 120);
        assert_eq!(config.query_timeout_secs, 30);
        assert!(!config.easy_mode);
    }

    #[test]
    fn test_config_clone() {
        let config1 = AppConfig::default();
        let config2 = config1.clone();
        assert_eq!(config1.server_port, config2.server_port);
        assert_eq!(config1.auto_start_server, config2.auto_start_server);
    }

    #[test]
    fn test_timeout_invariant() {
        let config = AppConfig::default();
        // Print timeout should generally be >= query timeout
        assert!(config.print_timeout_secs >= config.query_timeout_secs);
    }

    #[test]
    fn test_form_provenance_policy_defaults_to_record() {
        let config = AppConfig::default();
        assert_eq!(
            config.form_provenance_policy,
            crate::FormProvenancePolicy::Record
        );
        // Recording is on by default: the ruling is to record provenance.
        assert!(config.form_provenance_policy.inspects());
    }

    #[test]
    fn test_config_without_form_provenance_still_loads() {
        // A config file written by an older Presswerk has no such key.
        // It must still deserialise rather than resetting every setting.
        let legacy = r#"{
            "default_paper_size": "A4",
            "auto_start_server": true,
            "server_port": 9631,
            "server_require_tls": false,
            "auto_accept_network_jobs": false,
            "audit_enabled": true,
            "encryption_enabled": true,
            "print_timeout_secs": 60,
            "query_timeout_secs": 15,
            "easy_mode": true
        }"#;

        let config: AppConfig = serde_json::from_str(legacy).expect("legacy config must load");
        assert_eq!(config.server_port, 9631);
        assert!(config.auto_start_server);
        assert_eq!(
            config.form_provenance_policy,
            crate::FormProvenancePolicy::Record
        );
    }

    #[test]
    fn test_form_provenance_policy_survives_roundtrip() {
        let config = AppConfig {
            form_provenance_policy: crate::FormProvenancePolicy::HoldForReview,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            restored.form_provenance_policy,
            crate::FormProvenancePolicy::HoldForReview
        );
    }
}
