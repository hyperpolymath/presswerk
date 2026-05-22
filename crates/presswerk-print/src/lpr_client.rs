// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// LPR/LPD client (RFC 1179) for legacy printers.
//
// This is the fallback for printers that don't speak IPP but accept LPR
// on port 515. The protocol is simple: open connection, send a control
// file (metadata), then send the data file (document bytes).

use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::{info, warn};

use presswerk_core::error::{PresswerkError, Result};

/// Default LPR port.
pub const LPR_PORT: u16 = 515;

/// Timeout for LPR operations.
const LPR_TIMEOUT_SECS: u64 = 60;

/// Send a document via LPR/LPD protocol.
///
/// Implements a minimal RFC 1179 client:
/// 1. Send "receive job" command (0x02)
/// 2. Send control file with job metadata
/// 3. Send data file with document bytes
pub async fn send_lpr(ip: &str, port: u16, document_bytes: &[u8], job_name: &str) -> Result<()> {
    let addr = format!("{}:{}", ip, port);
    info!(addr = %addr, job = job_name, "connecting via LPR");

    let mut stream = tokio::time::timeout(
        Duration::from_secs(LPR_TIMEOUT_SECS),
        TcpStream::connect(&addr),
    )
    .await
    .map_err(|_| {
        PresswerkError::IppRequest(format!(
            "LPR connection to {} timed out after {}s",
            addr, LPR_TIMEOUT_SECS
        ))
    })?
    .map_err(|e| PresswerkError::IppRequest(format!("LPR connect to {}: {}", addr, e)))?;

    // RFC 1179: Send "receive a printer job" command
    // Format: 0x02 <queue-name> LF
    let queue = "lp"; // default queue
    let cmd = format!("\x02{}\n", queue);
    stream
        .write_all(cmd.as_bytes())
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR command: {e}")))?;

    // Wait for ACK (0x00)
    let mut ack = [0u8; 1];
    tokio::io::AsyncReadExt::read_exact(&mut stream, &mut ack)
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR ack: {e}")))?;

    if ack[0] != 0 {
        return Err(PresswerkError::IppRequest(
            "LPR printer rejected the job request".into(),
        ));
    }

    // Send control file
    let job_num = 1; // simplified — a real client would track this
    let hostname = "presswerk";
    let control_file = format!(
        "H{hostname}\nP{hostname}\nJ{job_name}\nldfA{job_num:03}{hostname}\nUdfA{job_num:03}{hostname}\nN{job_name}\n"
    );
    let cf_header = format!("\x02{} cfA{:03}{}\n", control_file.len(), job_num, hostname);

    stream
        .write_all(cf_header.as_bytes())
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR control header: {e}")))?;

    let mut ack = [0u8; 1];
    tokio::io::AsyncReadExt::read_exact(&mut stream, &mut ack)
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR control ack: {e}")))?;

    stream
        .write_all(control_file.as_bytes())
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR control file: {e}")))?;
    stream
        .write_all(&[0])
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR control term: {e}")))?;

    let mut ack = [0u8; 1];
    tokio::io::AsyncReadExt::read_exact(&mut stream, &mut ack)
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR data ack: {e}")))?;

    // Send data file
    let df_header = format!(
        "\x03{} dfA{:03}{}\n",
        document_bytes.len(),
        job_num,
        hostname
    );

    stream
        .write_all(df_header.as_bytes())
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR data header: {e}")))?;

    let mut ack = [0u8; 1];
    tokio::io::AsyncReadExt::read_exact(&mut stream, &mut ack)
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR data file ack: {e}")))?;

    stream
        .write_all(document_bytes)
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR data send: {e}")))?;
    stream
        .write_all(&[0])
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR data term: {e}")))?;

    let mut ack = [0u8; 1];
    tokio::io::AsyncReadExt::read_exact(&mut stream, &mut ack)
        .await
        .map_err(|e| PresswerkError::IppRequest(format!("LPR final ack: {e}")))?;

    if ack[0] != 0 {
        warn!("LPR printer returned non-zero ack after data transfer");
    }

    info!(job = job_name, "LPR job sent successfully");
    Ok(())
}
