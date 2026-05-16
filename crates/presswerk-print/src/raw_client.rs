// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Raw TCP print client (JetDirect, port 9100).
//
// The simplest possible print protocol: open a TCP socket and dump bytes.
// This is the ultimate fallback for printers that don't speak IPP or LPR.
// No settings, no job tracking, no feedback — just raw data transmission.
// The printer must be able to interpret the document format natively.

use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::{debug, info};

use presswerk_core::error::{PresswerkError, Result};

/// Default raw TCP port (HP JetDirect).
pub const RAW_PORT: u16 = 9100;

/// Timeout for raw TCP operations.
const RAW_TIMEOUT_SECS: u64 = 60;

/// Send document bytes directly to a printer via raw TCP (port 9100).
///
/// This is the lowest-level fallback. The printer must natively understand
/// the document format — there's no protocol negotiation, no settings
/// transmission, no job tracking.
///
/// Returns the number of bytes sent (for progress tracking/resumption).
pub async fn send_raw(ip: &str, port: u16, document_bytes: &[u8]) -> Result<()> {
    send_raw_with_offset(ip, port, document_bytes, 0).await
}

/// Send document bytes starting from a specific offset (for resumption).
pub async fn send_raw_with_offset(
    ip: &str,
    port: u16,
    document_bytes: &[u8],
    offset: usize,
) -> Result<()> {
    let addr = format!("{}:{}", ip, port);
    info!(
        addr = %addr,
        total = document_bytes.len(),
        offset,
        "connecting via raw TCP"
    );

    let mut stream = tokio::time::timeout(
        Duration::from_secs(RAW_TIMEOUT_SECS),
        TcpStream::connect(&addr),
    )
    .await
    .map_err(|_| {
        PresswerkError::IppRequest(format!(
            "Raw TCP connection to {} timed out after {}s",
            addr, RAW_TIMEOUT_SECS
        ))
    })?
    .map_err(|e| PresswerkError::IppRequest(format!("Raw TCP connect to {}: {}", addr, e)))?;

    // Send data from offset (for resumption after partial send)
    let remaining = &document_bytes[offset..];
    let chunk_size = 8192; // 8KB chunks for progress tracking

    let mut sent = offset;
    for chunk in remaining.chunks(chunk_size) {
        stream.write_all(chunk).await.map_err(|e| {
            PresswerkError::IppRequest(format!("Raw TCP send failed at byte {}: {}", sent, e))
        })?;
        sent += chunk.len();
        debug!(sent, total = document_bytes.len(), "raw TCP progress");
    }

    // Flush and shutdown cleanly
    stream
        .flush()
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("Raw TCP flush: {e}")))?;
    stream
        .shutdown()
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("Raw TCP shutdown: {e}")))?;

    info!(
        total = document_bytes.len(),
        "raw TCP print job sent successfully"
    );
    Ok(())
}
