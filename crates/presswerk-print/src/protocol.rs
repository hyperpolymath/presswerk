// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Protocol downgrade chain for maximum printer compatibility.
//
// SECURITY: Always starts with the most secure protocol (IPPS with TLS),
// then steps down ONLY when the printer cannot speak it. Never skips
// straight to raw TCP — each level is tried in order.
//
// Chain: IPPS (TLS) → IPP/1.1 → IPP/1.0 → LPR/LPD (port 515) → Raw TCP (port 9100)

use tracing::{debug, info, warn};

use presswerk_core::error::Result;
use presswerk_core::types::{DocumentType, PrintSettings};

/// Supported print protocols, ordered from most secure to least.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintProtocol {
    /// IPP over TLS (port 631, ipps://).
    Ipps,
    /// IPP/1.1 plain (port 631, ipp://).
    Ipp11,
    /// IPP/1.0 plain (legacy printers).
    Ipp10,
    /// LPR/LPD (RFC 1179, port 515).
    Lpr,
    /// Raw TCP socket (port 9100, JetDirect).
    RawTcp,
}

impl PrintProtocol {
    /// All protocols in security-preferred order.
    pub fn chain() -> &'static [PrintProtocol] {
        &[
            PrintProtocol::Ipps,
            PrintProtocol::Ipp11,
            PrintProtocol::Ipp10,
            PrintProtocol::Lpr,
            PrintProtocol::RawTcp,
        ]
    }

    /// Human-readable name for UI display.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ipps => "Secure IPP (TLS)",
            Self::Ipp11 => "IPP 1.1",
            Self::Ipp10 => "IPP 1.0",
            Self::Lpr => "LPR/LPD",
            Self::RawTcp => "Direct TCP",
        }
    }

    /// Default port for this protocol.
    pub fn default_port(&self) -> u16 {
        match self {
            Self::Ipps | Self::Ipp11 | Self::Ipp10 => 631,
            Self::Lpr => 515,
            Self::RawTcp => 9100,
        }
    }
}

/// Result of a protocol probe — can we talk to the printer this way?
pub struct ProbeResult {
    pub protocol: PrintProtocol,
    pub success: bool,
    pub error: Option<String>,
}

/// Probe all protocols to find which ones the printer supports.
///
/// Returns the results for ALL protocols (not just the first success).
/// This is used by the diagnostics engine to give a complete picture.
pub async fn probe_all_protocols(ip: &str, base_port: u16) -> Vec<ProbeResult> {
    let mut results = Vec::new();

    for protocol in PrintProtocol::chain() {
        let port = if base_port != 631 {
            base_port
        } else {
            protocol.default_port()
        };
        let result = probe_protocol(ip, port, *protocol).await;
        results.push(result);
    }

    results
}

/// Find the best (most secure) working protocol for a printer.
///
/// Tries each protocol in security order and returns the first that works.
/// Transparent to the user — they just see "Trying the best way to talk to
/// your printer..."
pub async fn find_best_protocol(ip: &str, base_port: u16) -> Option<PrintProtocol> {
    for protocol in PrintProtocol::chain() {
        let port = if base_port != 631 {
            base_port
        } else {
            protocol.default_port()
        };
        let result = probe_protocol(ip, port, *protocol).await;
        if result.success {
            info!(
                protocol = protocol.display_name(),
                ip, port, "found working protocol"
            );
            return Some(*protocol);
        }
        debug!(
            protocol = protocol.display_name(),
            error = result.error.as_deref().unwrap_or("unknown"),
            "protocol not supported, trying next"
        );
    }

    warn!(ip, "no working protocol found for printer");
    None
}

/// Send a print job using the specified protocol.
pub async fn send_via_protocol(
    protocol: PrintProtocol,
    ip: &str,
    port: u16,
    document_bytes: Vec<u8>,
    document_type: DocumentType,
    job_name: &str,
    settings: &PrintSettings,
) -> Result<()> {
    match protocol {
        PrintProtocol::Ipps => {
            let uri = format!("ipps://{}:{}/ipp/print", ip, port);
            let client = crate::ipp_client::IppClient::new(&uri)?;
            client
                .print_job(document_bytes, document_type, job_name, settings)
                .await?;
            Ok(())
        }
        PrintProtocol::Ipp11 | PrintProtocol::Ipp10 => {
            let uri = format!("ipp://{}:{}/ipp/print", ip, port);
            let client = crate::ipp_client::IppClient::new(&uri)?;
            client
                .print_job(document_bytes, document_type, job_name, settings)
                .await?;
            Ok(())
        }
        PrintProtocol::Lpr => {
            crate::lpr_client::send_lpr(ip, port, &document_bytes, job_name).await
        }
        PrintProtocol::RawTcp => crate::raw_client::send_raw(ip, port, &document_bytes).await,
    }
}

/// Probe a specific protocol.
async fn probe_protocol(ip: &str, port: u16, protocol: PrintProtocol) -> ProbeResult {
    let success = match protocol {
        PrintProtocol::Ipps => {
            let uri = format!("ipps://{}:{}/ipp/print", ip, port);
            probe_ipp(&uri).await
        }
        PrintProtocol::Ipp11 | PrintProtocol::Ipp10 => {
            let uri = format!("ipp://{}:{}/ipp/print", ip, port);
            probe_ipp(&uri).await
        }
        PrintProtocol::Lpr => probe_tcp(ip, port).await,
        PrintProtocol::RawTcp => probe_tcp(ip, port).await,
    };

    ProbeResult {
        protocol,
        success: success.is_ok(),
        error: success.err(),
    }
}

async fn probe_ipp(uri: &str) -> std::result::Result<(), String> {
    let client = crate::ipp_client::IppClient::new(uri).map_err(|e| e.to_string())?;
    client
        .get_printer_attributes()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn probe_tcp(ip: &str, port: u16) -> std::result::Result<(), String> {
    let addr = format!("{}:{}", ip, port);
    let addr: std::net::SocketAddr = addr
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;
    tokio::net::TcpStream::connect(addr)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
