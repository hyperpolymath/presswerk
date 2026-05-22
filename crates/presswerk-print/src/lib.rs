// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Presswerk Print — IPP client/server, mDNS printer discovery, and persistent
// job queue.  This crate bridges between the core domain types defined in
// `presswerk-core` and the actual network printing infrastructure.

#![forbid(unsafe_code)]
pub mod capabilities;
pub mod diagnostics;
pub mod discovery;
pub mod health;
pub mod ipp_client;
pub mod ipp_server;
pub mod lpr_client;
pub mod protocol;
pub mod queue;
pub mod raw_client;
pub mod resilience;
pub mod retry;
pub mod revival;

pub use capabilities::PrinterCapabilities;
pub use discovery::PrinterDiscovery;
pub use health::HealthTracker;
pub use ipp_client::IppClient;
pub use ipp_server::IppServer;
pub use queue::JobQueue;
pub use retry::RetryConfig;
