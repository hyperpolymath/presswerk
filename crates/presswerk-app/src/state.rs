// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Global application state — reactive signals for the Dioxus UI.

use presswerk_core::AppConfig;
use presswerk_core::types::{DiscoveredPrinter, PrintJob, ServerStatus};

use crate::services::app_services::AppServices;

/// Print progress stages for the UI progress indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PrintStage {
    /// No active print operation.
    Idle,
    /// Reading and preparing the document.
    Preparing,
    /// Querying the printer for readiness.
    CheckingPrinter,
    /// Transmitting bytes to the printer.
    Sending,
    /// Waiting for printer confirmation.
    Confirming,
    /// Successfully sent.
    Complete,
    /// Print failed.
    Failed,
    /// Retrying after a transient failure.
    Retrying,
}

/// Detailed print progress for the UI.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PrintProgress {
    /// Current stage of the print operation.
    pub stage: PrintStage,
    /// Percentage complete (0–100), if known.
    pub percent: Option<u8>,
    /// Human-readable status message.
    pub message: String,
}

impl Default for PrintProgress {
    fn default() -> Self {
        Self {
            stage: PrintStage::Idle,
            percent: None,
            message: String::new(),
        }
    }
}

/// Shared state accessible to all pages via `use_context`.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Printers found on the local network.
    pub printers: Vec<DiscoveredPrinter>,
    /// Currently selected printer URI.
    pub selected_printer: Option<String>,
    /// All print jobs (local + network-received).
    pub jobs: Vec<PrintJob>,
    /// Status of the embedded IPP print server.
    pub server_status: ServerStatus,
    /// Application settings.
    pub config: AppConfig,
    /// Whether a discovery scan is in progress.
    pub scanning: bool,
    /// Status message for user feedback.
    pub status_message: Option<String>,
    /// Currently loaded document bytes (for print/edit flows).
    pub current_document: Option<Vec<u8>>,
    /// Name of the currently loaded document.
    pub current_document_name: Option<String>,
    /// Current print progress.
    #[allow(dead_code)]
    pub print_progress: PrintProgress,
    /// Whether Easy Mode is active (default: true).
    #[allow(dead_code)]
    pub easy_mode: bool,
}

impl AppState {
    /// Create initial state from the backend services.
    pub fn new(svc: &AppServices) -> Self {
        let config = svc.config();
        let jobs = svc.all_jobs().unwrap_or_default();
        let printers = svc.discovered_printers();
        let scanning = svc.is_discovering();

        Self {
            printers,
            selected_printer: None,
            jobs,
            server_status: ServerStatus::Stopped,
            config,
            scanning,
            status_message: None,
            current_document: None,
            current_document_name: None,
            print_progress: PrintProgress::default(),
            easy_mode: true,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            printers: Vec::new(),
            selected_printer: None,
            jobs: Vec::new(),
            server_status: ServerStatus::Stopped,
            config: AppConfig::default(),
            scanning: false,
            status_message: None,
            current_document: None,
            current_document_name: None,
            print_progress: PrintProgress::default(),
            easy_mode: true,
        }
    }
}
