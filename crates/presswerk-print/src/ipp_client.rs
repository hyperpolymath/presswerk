// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Async IPP client for communicating with network printers.
//
// Uses the `ipp` crate's async API to send standard IPP operations:
//   - Get-Printer-Attributes  (RFC 8011 §4.2.5)
//   - Print-Job               (RFC 8011 §4.2.1)
//   - Get-Jobs                (RFC 8011 §4.2.6)
//   - Cancel-Job              (RFC 8011 §4.2.8)

use std::collections::HashMap;
use std::io::Cursor;
use std::time::Duration;

use ipp::prelude::*;
use tracing::{debug, error, info, instrument};

use presswerk_core::error::{PresswerkError, Result};
use presswerk_core::types::{DocumentType, PrintSettings};

/// Attributes returned by a Get-Printer-Attributes response.
///
/// This is a flattened map of attribute-name to a human-readable string value.
/// The raw IPP attribute groups are available via [`get_printer_attributes_raw`].
pub type PrinterAttributes = HashMap<String, String>;

/// Summary of a remote print job as returned by Get-Jobs.
#[derive(Debug, Clone)]
pub struct RemoteJobInfo {
    /// IPP job-id (integer assigned by the printer).
    pub job_id: i32,
    /// Human-readable job name (`job-name` attribute).
    pub job_name: String,
    /// IPP job-state keyword (e.g. "processing", "completed").
    pub job_state: String,
}

/// Timeout for print operations (seconds).
const PRINT_TIMEOUT_SECS: u64 = 60;

/// Timeout for query operations like Get-Printer-Attributes, Get-Jobs (seconds).
const QUERY_TIMEOUT_SECS: u64 = 15;

/// Async IPP client wrapping the `ipp` crate.
///
/// Each instance is bound to a single printer URI.  All methods are async and
/// require a Tokio runtime.
pub struct IppClient {
    /// The target printer URI (ipp:// or ipps://).
    uri: Uri,
}

impl IppClient {
    /// Create a new client targeting the given printer URI.
    ///
    /// The URI should be an `ipp://` or `ipps://` address, typically obtained
    /// from mDNS discovery or user configuration.
    pub fn new(uri: &str) -> Result<Self> {
        let parsed: Uri = uri
            .parse()
            .map_err(|e| PresswerkError::IppRequest(format!("invalid URI '{uri}': {e}")))?;
        Ok(Self { uri: parsed })
    }

    /// Return the printer URI this client is targeting.
    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    /// Query the printer for its capabilities and current state.
    ///
    /// Sends a Get-Printer-Attributes operation and returns the response as a
    /// flat map of attribute names to their string representations.
    #[instrument(skip(self), fields(uri = %self.uri))]
    pub async fn get_printer_attributes(&self) -> Result<PrinterAttributes> {
        let operation = IppOperationBuilder::get_printer_attributes(self.uri.clone()).build();
        let client = AsyncIppClient::new(self.uri.clone());

        debug!("sending Get-Printer-Attributes");
        let response = tokio::time::timeout(
            Duration::from_secs(QUERY_TIMEOUT_SECS),
            client.send(operation),
        )
        .await
        .map_err(|_| {
            PresswerkError::IppRequest(format!(
                "Get-Printer-Attributes timed out after {}s",
                QUERY_TIMEOUT_SECS
            ))
        })?
        .map_err(|e| PresswerkError::IppRequest(format!("Get-Printer-Attributes: {e}")))?;

        if !response.header().status_code().is_success() {
            let code = response.header().status_code();
            error!(status = ?code, "Get-Printer-Attributes failed");
            return Err(PresswerkError::IppRequest(format!(
                "Get-Printer-Attributes returned status {code:?}"
            )));
        }

        let attrs = flatten_attributes(response.attributes());
        debug!(count = attrs.len(), "received printer attributes");
        Ok(attrs)
    }

    /// Submit a document to the printer as a Print-Job.
    ///
    /// Returns the job-id assigned by the printer on success.
    ///
    /// # Arguments
    ///
    /// * `document_bytes` — raw bytes of the document to print.
    /// * `document_type`  — the document MIME type (used for `document-format`).
    /// * `job_name`       — human-readable name shown in the printer queue.
    /// * `settings`       — user print settings (copies, paper, duplex, colour, etc.)
    #[instrument(skip(self, document_bytes, settings), fields(uri = %self.uri, job_name = %job_name))]
    pub async fn print_job(
        &self,
        document_bytes: Vec<u8>,
        document_type: DocumentType,
        job_name: &str,
        settings: &PrintSettings,
    ) -> Result<i32> {
        let payload = IppPayload::new(Cursor::new(document_bytes));

        let mut builder = IppOperationBuilder::print_job(self.uri.clone(), payload)
            .job_title(job_name)
            .document_format(document_type.mime_type());

        // Inject print settings as IPP job-template attributes.
        builder = builder.attribute(IppAttribute::new(
            "copies",
            IppValue::Integer(settings.copies as i32),
        ));
        builder = builder.attribute(IppAttribute::new(
            "media",
            IppValue::Keyword(settings.paper_size.ipp_media_keyword().into()),
        ));
        builder = builder.attribute(IppAttribute::new(
            "sides",
            IppValue::Keyword(settings.duplex.ipp_sides_keyword().into()),
        ));
        builder = builder.attribute(IppAttribute::new(
            "orientation-requested",
            IppValue::Enum(settings.orientation.ipp_enum_value()),
        ));
        builder = builder.attribute(IppAttribute::new(
            "print-color-mode",
            IppValue::Keyword(
                if settings.color {
                    "color"
                } else {
                    "monochrome"
                }
                .into(),
            ),
        ));

        // Page ranges (1-indexed, inclusive)
        if let Some(ref range) = settings.page_range {
            builder = builder.attribute(IppAttribute::new(
                "page-ranges",
                IppValue::RangeOfInteger {
                    min: range.start as i32,
                    max: range.end as i32,
                },
            ));
        }

        let operation = builder.build();
        let client = AsyncIppClient::new(self.uri.clone());

        info!(
            mime = document_type.mime_type(),
            copies = settings.copies,
            duplex = settings.duplex.ipp_sides_keyword(),
            color = settings.color,
            "sending Print-Job with settings"
        );

        let response = tokio::time::timeout(
            Duration::from_secs(PRINT_TIMEOUT_SECS),
            client.send(operation),
        )
        .await
        .map_err(|_| {
            PresswerkError::IppRequest(format!(
                "Print-Job timed out after {}s — printer may be busy or offline",
                PRINT_TIMEOUT_SECS
            ))
        })?
        .map_err(|e| PresswerkError::IppRequest(format!("Print-Job: {e}")))?;

        if !response.header().status_code().is_success() {
            let code = response.header().status_code();
            error!(status = ?code, "Print-Job failed");
            return Err(PresswerkError::IppRequest(format!(
                "Print-Job returned status {code:?}"
            )));
        }

        // The job-id is in the Job Attributes group.
        let job_id = extract_job_id(response.attributes()).ok_or_else(|| {
            PresswerkError::IppRequest("Print-Job response missing job-id attribute".into())
        })?;

        info!(job_id, "print job accepted by printer");
        Ok(job_id)
    }

    /// Retrieve the list of jobs currently known to the printer.
    #[instrument(skip(self), fields(uri = %self.uri))]
    pub async fn get_jobs(&self) -> Result<Vec<RemoteJobInfo>> {
        let operation = IppOperationBuilder::get_jobs(self.uri.clone()).build();
        let client = AsyncIppClient::new(self.uri.clone());

        debug!("sending Get-Jobs");
        let response = tokio::time::timeout(
            Duration::from_secs(QUERY_TIMEOUT_SECS),
            client.send(operation),
        )
        .await
        .map_err(|_| {
            PresswerkError::IppRequest(format!("Get-Jobs timed out after {}s", QUERY_TIMEOUT_SECS))
        })?
        .map_err(|e| PresswerkError::IppRequest(format!("Get-Jobs: {e}")))?;

        if !response.header().status_code().is_success() {
            let code = response.header().status_code();
            error!(status = ?code, "Get-Jobs failed");
            return Err(PresswerkError::IppRequest(format!(
                "Get-Jobs returned status {code:?}"
            )));
        }

        let jobs = parse_jobs(response.attributes());
        debug!(count = jobs.len(), "received job list");
        Ok(jobs)
    }

    /// Cancel a specific job on the printer.
    ///
    /// Returns `Ok(())` if the printer accepted the cancellation.
    #[instrument(skip(self), fields(uri = %self.uri, job_id))]
    pub async fn cancel_job(&self, job_id: i32) -> Result<()> {
        let operation = IppOperationBuilder::cancel_job(self.uri.clone(), job_id).build();
        let client = AsyncIppClient::new(self.uri.clone());

        info!(job_id, "sending Cancel-Job");
        let response = tokio::time::timeout(
            Duration::from_secs(QUERY_TIMEOUT_SECS),
            client.send(operation),
        )
        .await
        .map_err(|_| {
            PresswerkError::IppRequest(format!(
                "Cancel-Job({job_id}) timed out after {}s",
                QUERY_TIMEOUT_SECS
            ))
        })?
        .map_err(|e| PresswerkError::IppRequest(format!("Cancel-Job({}): {e}", job_id)))?;

        if !response.header().status_code().is_success() {
            let code = response.header().status_code();
            error!(status = ?code, job_id, "Cancel-Job failed");
            return Err(PresswerkError::IppRequest(format!(
                "Cancel-Job({job_id}) returned status {code:?}"
            )));
        }

        info!(job_id, "job cancelled");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helper functions for parsing IPP responses
// ---------------------------------------------------------------------------

/// Flatten all attribute groups in an IPP response into a single map.
///
/// Multi-valued attributes are joined with `", "`.  This intentionally
/// discards group-level context in favour of a simpler lookup interface.
fn flatten_attributes(attrs: &IppAttributes) -> PrinterAttributes {
    let mut map = HashMap::new();
    for group in attrs.groups() {
        for (name, attr) in group.attributes() {
            map.insert(name.clone(), format!("{}", attr.value()));
        }
    }
    map
}

/// Extract the `job-id` integer from a response's Job Attributes group.
fn extract_job_id(attrs: &IppAttributes) -> Option<i32> {
    for group in attrs.groups_of(DelimiterTag::JobAttributes) {
        if let Some(attr) = group.attributes().get("job-id")
            && let IppValue::Integer(id) = attr.value()
        {
            return Some(*id);
        }
    }
    None
}

/// Parse the Get-Jobs response into a vec of `RemoteJobInfo`.
///
/// Each job is represented as a separate Job Attributes group in the IPP
/// response.
fn parse_jobs(attrs: &IppAttributes) -> Vec<RemoteJobInfo> {
    let mut jobs = Vec::new();

    for group in attrs.groups_of(DelimiterTag::JobAttributes) {
        let attributes = group.attributes();

        let job_id = attributes.get("job-id").and_then(|a| {
            if let IppValue::Integer(id) = a.value() {
                Some(*id)
            } else {
                None
            }
        });

        let job_name = attributes
            .get("job-name")
            .map(|a| format!("{}", a.value()))
            .unwrap_or_default();

        let job_state = attributes
            .get("job-state")
            .map(|a| format!("{}", a.value()))
            .unwrap_or_else(|| "unknown".into());

        if let Some(id) = job_id {
            jobs.push(RemoteJobInfo {
                job_id: id,
                job_name,
                job_state,
            });
        }
    }

    jobs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_invalid_uri() {
        let result = IppClient::new("not a valid uri %%%");
        assert!(result.is_err());
    }

    #[test]
    fn new_accepts_valid_ipp_uri() {
        let client = IppClient::new("ipp://192.168.1.100:631/ipp/print");
        assert!(client.is_ok());
    }
}
