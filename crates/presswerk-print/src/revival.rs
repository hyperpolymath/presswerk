// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Dead printer revival utilities.
//
// Wake sleeping printers, clear stuck spoolers, and probe status.
// Integrated into the Print Doctor: "Your printer seems asleep. [Wake it up]"

use std::net::UdpSocket;

use tracing::{debug, info, warn};

use presswerk_core::error::{PresswerkError, Result};

/// Send a Wake-on-LAN (WoL) magic packet to wake a sleeping printer.
///
/// Requires the printer's MAC address (6 bytes). Sends the standard
/// magic packet (6x 0xFF followed by 16 repetitions of the MAC) to
/// the broadcast address on port 9 (discard protocol).
pub fn wake_on_lan(mac_address: &[u8; 6]) -> Result<()> {
    let mut magic_packet = Vec::with_capacity(102);

    // Preamble: 6 bytes of 0xFF
    magic_packet.extend_from_slice(&[0xFF; 6]);

    // Payload: MAC address repeated 16 times
    for _ in 0..16 {
        magic_packet.extend_from_slice(mac_address);
    }

    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| PresswerkError::IppRequest(format!("WoL bind: {e}")))?;
    socket
        .set_broadcast(true)
        .map_err(|e| PresswerkError::IppRequest(format!("WoL broadcast: {e}")))?;

    socket
        .send_to(&magic_packet, "255.255.255.255:9")
        .map_err(|e| PresswerkError::IppRequest(format!("WoL send: {e}")))?;

    info!(
        mac = format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac_address[0],
            mac_address[1],
            mac_address[2],
            mac_address[3],
            mac_address[4],
            mac_address[5]
        ),
        "Wake-on-LAN magic packet sent"
    );

    Ok(())
}

/// Try to clear a stuck printer spooler via IPP Purge-Jobs.
///
/// Some printers get stuck with stale jobs in the queue. This sends
/// Purge-Jobs to clear them all.
pub async fn purge_stuck_jobs(printer_uri: &str) -> Result<()> {
    let client = crate::ipp_client::IppClient::new(printer_uri)?;

    // Cancel all jobs — Purge-Jobs is not universally supported,
    // so we fall back to getting all jobs and cancelling individually.
    let jobs = client.get_jobs().await?;

    if jobs.is_empty() {
        debug!("no stuck jobs to purge");
        return Ok(());
    }

    info!(count = jobs.len(), "purging stuck jobs from printer");

    let mut purged = 0;
    for job in &jobs {
        match client.cancel_job(job.job_id).await {
            Ok(()) => {
                purged += 1;
                debug!(job_id = job.job_id, "cancelled stuck job");
            }
            Err(e) => {
                warn!(
                    job_id = job.job_id,
                    error = %e,
                    "could not cancel job (may already be complete)"
                );
            }
        }
    }

    info!(purged, total = jobs.len(), "finished purging stuck jobs");
    Ok(())
}

/// Probe a printer's current status.
///
/// Returns a tuple of (state, reasons) from Get-Printer-Attributes.
pub async fn probe_status(
    printer_uri: &str,
) -> Result<(String, Vec<String>)> {
    let client = crate::ipp_client::IppClient::new(printer_uri)?;
    let attrs = client.get_printer_attributes().await?;

    let state = attrs
        .get("printer-state")
        .cloned()
        .unwrap_or_else(|| "unknown".into());

    let reasons: Vec<String> = attrs
        .get("printer-state-reasons")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| s != "none")
                .collect()
        })
        .unwrap_or_default();

    Ok((state, reasons))
}

/// Parse a MAC address string (e.g. "AA:BB:CC:DD:EE:FF") into bytes.
pub fn parse_mac(mac_str: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = mac_str.split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        mac[i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mac_colon_format() {
        let mac = parse_mac("AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(mac, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn parse_mac_dash_format() {
        let mac = parse_mac("11-22-33-44-55-66").unwrap();
        assert_eq!(mac, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    }

    #[test]
    fn parse_mac_invalid() {
        assert!(parse_mac("not-a-mac").is_none());
        assert!(parse_mac("AA:BB:CC").is_none());
    }
}
