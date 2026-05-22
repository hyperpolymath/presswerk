// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Network self-healing and offline job buffering.
//
// Monitors network connectivity and buffers print jobs when offline.
// Automatically flushes the buffer when connectivity returns.
// User sees: "You're offline. We'll hold your document and print it
// automatically when you reconnect. (N document(s) waiting)"

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tracing::{info, warn};

use presswerk_core::types::{DocumentType, JobId, PrintSettings};

/// A buffered print job waiting for network connectivity.
#[derive(Debug, Clone)]
pub struct BufferedJob {
    pub job_id: JobId,
    pub document_bytes: Vec<u8>,
    pub document_type: DocumentType,
    pub document_name: String,
    pub printer_uri: String,
    pub settings: PrintSettings,
    pub buffered_at: chrono::DateTime<chrono::Utc>,
}

/// Network connectivity state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityState {
    /// Network is available.
    Online,
    /// No network connectivity.
    Offline,
    /// Checking connectivity.
    Probing,
}

/// Network resilience manager.
///
/// Buffers jobs during network outages and auto-delivers when connectivity
/// returns. Periodically probes to detect reconnection.
pub struct NetworkResilience {
    /// Buffered jobs waiting to be sent.
    buffer: Arc<Mutex<VecDeque<BufferedJob>>>,
    /// Current connectivity state.
    state: Arc<Mutex<ConnectivityState>>,
}

impl Default for NetworkResilience {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkResilience {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(VecDeque::new())),
            state: Arc::new(Mutex::new(ConnectivityState::Online)),
        }
    }

    /// Check current network connectivity.
    pub fn check_connectivity(&self) -> ConnectivityState {
        let is_online = std::net::UdpSocket::bind("0.0.0.0:0")
            .and_then(|s| {
                s.connect("8.8.8.8:53")?;
                s.local_addr()
            })
            .map(|addr| !addr.ip().is_loopback())
            .unwrap_or(false);

        let new_state = if is_online {
            ConnectivityState::Online
        } else {
            ConnectivityState::Offline
        };

        if let Ok(mut state) = self.state.lock() {
            let old = *state;
            *state = new_state;
            if old == ConnectivityState::Offline && new_state == ConnectivityState::Online {
                info!("network connectivity restored");
            } else if old == ConnectivityState::Online && new_state == ConnectivityState::Offline {
                warn!("network connectivity lost");
            }
        }

        new_state
    }

    /// Buffer a job for later delivery.
    pub fn buffer_job(&self, job: BufferedJob) {
        if let Ok(mut buffer) = self.buffer.lock() {
            info!(
                job_id = %job.job_id,
                name = %job.document_name,
                "buffering job for offline delivery"
            );
            buffer.push_back(job);
        }
    }

    /// Number of jobs in the buffer.
    pub fn buffered_count(&self) -> usize {
        self.buffer.lock().map(|b| b.len()).unwrap_or(0)
    }

    /// Take all buffered jobs for delivery (empties the buffer).
    pub fn drain_buffer(&self) -> Vec<BufferedJob> {
        self.buffer
            .lock()
            .map(|mut b| b.drain(..).collect())
            .unwrap_or_default()
    }

    /// Get the current connectivity state.
    pub fn connectivity(&self) -> ConnectivityState {
        self.state
            .lock()
            .map(|s| *s)
            .unwrap_or(ConnectivityState::Online)
    }

    /// User-facing status message.
    pub fn status_message(&self) -> Option<String> {
        let count = self.buffered_count();
        if count > 0 && self.connectivity() == ConnectivityState::Offline {
            Some(format!(
                "You're offline. We'll hold your {} and print {} automatically when you reconnect.",
                if count == 1 { "document" } else { "documents" },
                if count == 1 { "it" } else { "them" },
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_buffered_job() -> BufferedJob {
        BufferedJob {
            job_id: JobId::new(),
            document_bytes: vec![1, 2, 3],
            document_type: DocumentType::Pdf,
            document_name: "test.pdf".into(),
            printer_uri: "ipp://test:631/".into(),
            settings: PrintSettings::default(),
            buffered_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn buffer_and_drain() {
        let resilience = NetworkResilience::new();
        assert_eq!(resilience.buffered_count(), 0);

        resilience.buffer_job(test_buffered_job());
        resilience.buffer_job(test_buffered_job());
        assert_eq!(resilience.buffered_count(), 2);

        let drained = resilience.drain_buffer();
        assert_eq!(drained.len(), 2);
        assert_eq!(resilience.buffered_count(), 0);
    }
}
