// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Embedded IPP/1.1 print server -- makes the phone act as a network printer.
//
// The server listens on a configurable TCP port (default 631) for incoming IPP
// requests from other devices.  Received print jobs are injected into the
// local `JobQueue` for the user to preview and forward to a real printer.
//
// # Protocol implementation
//
// IPP is transported over HTTP POST (RFC 8010 SS3), but this implementation
// operates directly on raw TCP for simplicity on mobile devices where a full
// HTTP server is unnecessary overhead.  Clients send an HTTP POST with an
// `application/ipp` body; we parse the HTTP framing just enough to extract
// the IPP payload, then respond with a minimal HTTP/1.1 200 OK wrapping the
// IPP response body.
//
// # Supported operations
//
//   - Print-Job         (0x0002)  RFC 8011 SS4.2.1
//   - Validate-Job      (0x0004)  RFC 8011 SS4.2.3
//   - Cancel-Job        (0x0008)  RFC 8011 SS4.3.3
//   - Get-Jobs          (0x000A)  RFC 8011 SS4.2.6
//   - Get-Printer-Attrs (0x000B)  RFC 8011 SS4.2.5
//
// # mDNS advertisement
//
// On start the server registers `_ipp._tcp.local.` via mDNS-SD so other
// devices on the LAN can discover it automatically.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use presswerk_core::error::{PresswerkError, Result};
use presswerk_core::types::{DocumentType, JobId, JobSource, JobStatus, PrintJob, ServerStatus};

use crate::queue::JobQueue;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default port for the IPP print server (IANA-assigned for IPP).
const DEFAULT_PORT: u16 = 631;

/// Maximum bytes to read from a connection before rejecting it.
/// Prevents unbounded memory consumption from misbehaving clients.
const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024; // 64 MiB

/// IPP version 1.1 major byte.
pub const IPP_VERSION_MAJOR: u8 = 0x01;

/// IPP version 1.1 minor byte.
pub const IPP_VERSION_MINOR: u8 = 0x01;

/// Default printer name advertised via mDNS and returned in attributes.
const PRINTER_NAME: &str = "Presswerk Virtual Printer";

/// mDNS service type for plain IPP.
const IPP_SERVICE_TYPE: &str = "_ipp._tcp.local.";

// ---------------------------------------------------------------------------
// IPP delimiter tags (RFC 8010 SS3.5.1)
// ---------------------------------------------------------------------------

/// Operation attributes group delimiter.
pub const TAG_OPERATION_ATTRIBUTES: u8 = 0x01;

/// Job attributes group delimiter.
pub const TAG_JOB_ATTRIBUTES: u8 = 0x02;

/// End-of-attributes-tag -- terminates the attribute section.
pub const TAG_END_OF_ATTRIBUTES: u8 = 0x03;

/// Printer attributes group delimiter.
pub const TAG_PRINTER_ATTRIBUTES: u8 = 0x04;

// ---------------------------------------------------------------------------
// IPP value tags (RFC 8010 SS3.5.2)
// ---------------------------------------------------------------------------

/// Integer value (4 bytes, signed big-endian).
pub const VALUE_TAG_INTEGER: u8 = 0x21;

/// Boolean value (1 byte: 0x00 = false, 0x01 = true).
pub const VALUE_TAG_BOOLEAN: u8 = 0x22;

/// Enum value (4 bytes, same encoding as integer).
pub const VALUE_TAG_ENUM: u8 = 0x23;

/// textWithoutLanguage (UTF-8 string).
pub const VALUE_TAG_TEXT: u8 = 0x41;

/// nameWithoutLanguage (UTF-8 string).
pub const VALUE_TAG_NAME: u8 = 0x42;

/// keyword (US-ASCII string, used for document-format etc.).
pub const VALUE_TAG_KEYWORD: u8 = 0x44;

/// uri (US-ASCII string).
pub const VALUE_TAG_URI: u8 = 0x45;

/// charset (US-ASCII string, e.g. "utf-8").
pub const VALUE_TAG_CHARSET: u8 = 0x47;

/// naturalLanguage (US-ASCII string, e.g. "en").
pub const VALUE_TAG_NATURAL_LANGUAGE: u8 = 0x48;

// ---------------------------------------------------------------------------
// IPP operation IDs (RFC 8011 SS4)
// ---------------------------------------------------------------------------

/// Print-Job operation identifier.
pub const OP_PRINT_JOB: u16 = 0x0002;

/// Validate-Job operation identifier.
pub const OP_VALIDATE_JOB: u16 = 0x0004;

/// Cancel-Job operation identifier.
pub const OP_CANCEL_JOB: u16 = 0x0008;

/// Get-Jobs operation identifier.
pub const OP_GET_JOBS: u16 = 0x000A;

/// Get-Printer-Attributes operation identifier.
pub const OP_GET_PRINTER_ATTRIBUTES: u16 = 0x000B;

// ---------------------------------------------------------------------------
// IPP status codes (RFC 8011 SS4.1.8)
// ---------------------------------------------------------------------------

/// Successful completion.
pub const STATUS_OK: u16 = 0x0000;

/// Client sent a malformed request.
const STATUS_CLIENT_ERROR_BAD_REQUEST: u16 = 0x0400;

/// The requested job was not found.
const STATUS_CLIENT_ERROR_NOT_FOUND: u16 = 0x0406;

/// The requested operation is not supported.
const STATUS_SERVER_ERROR_OPERATION_NOT_SUPPORTED: u16 = 0x0501;

/// Internal server error.
const STATUS_SERVER_ERROR_INTERNAL: u16 = 0x0500;

// ---------------------------------------------------------------------------
// IPP job-state values (RFC 8011 SS4.3.7)
// ---------------------------------------------------------------------------

/// Job has been created and is waiting to be processed.
const JOB_STATE_PENDING: i32 = 3;

/// Job is held for user review.
const JOB_STATE_HELD: i32 = 4;

/// Job is currently being processed.
const JOB_STATE_PROCESSING: i32 = 5;

/// Job has completed successfully.
const JOB_STATE_COMPLETED: i32 = 9;

/// Job has been cancelled.
const JOB_STATE_CANCELED: i32 = 7;

/// Job processing was aborted (failed).
const JOB_STATE_ABORTED: i32 = 8;

// ---------------------------------------------------------------------------
// IPP printer-state values (RFC 8011 SS4.4.11)
// ---------------------------------------------------------------------------

/// Printer is idle and ready.
const PRINTER_STATE_IDLE: i32 = 3;

// ---------------------------------------------------------------------------
// Parsed IPP request
// ---------------------------------------------------------------------------

/// A single parsed IPP attribute.
#[derive(Debug, Clone)]
pub struct IppAttribute {
    /// The value tag that describes the type of this attribute.
    /// Retained for future use (e.g. distinguishing keyword vs text responses).
    #[allow(dead_code)]
    pub value_tag: u8,
    /// Attribute name (empty for additional values in a 1setOf).
    pub name: String,
    /// Raw value bytes.
    pub value: Vec<u8>,
}

/// A group of attributes delimited by a group tag.
#[derive(Debug, Clone)]
pub struct IppAttributeGroup {
    /// The delimiter tag for this group (0x01, 0x02, 0x04, etc.)
    pub delimiter: u8,
    /// Ordered list of attributes within the group.
    pub attributes: Vec<IppAttribute>,
}

impl IppAttributeGroup {
    /// Convenience: find the first attribute with the given name.
    pub fn get(&self, name: &str) -> Option<&IppAttribute> {
        self.attributes.iter().find(|a| a.name == name)
    }

    /// Read the first attribute with the given name as a UTF-8 string.
    pub fn get_string(&self, name: &str) -> Option<String> {
        self.get(name)
            .and_then(|a| String::from_utf8(a.value.clone()).ok())
    }

    /// Read the first attribute with the given name as an i32 integer.
    pub fn get_integer(&self, name: &str) -> Option<i32> {
        self.get(name).and_then(|a| {
            if a.value.len() == 4 {
                Some(i32::from_be_bytes([
                    a.value[0], a.value[1], a.value[2], a.value[3],
                ]))
            } else {
                None
            }
        })
    }
}

/// A fully parsed IPP request.
#[derive(Debug)]
pub struct IppRequest {
    /// IPP version major (should be 1).
    pub version_major: u8,
    /// IPP version minor (should be 1).
    pub version_minor: u8,
    /// The operation identifier (e.g. 0x0002 for Print-Job).
    pub operation_id: u16,
    /// The request-id (echoed back in the response).
    pub request_id: u32,
    /// All attribute groups in order.
    pub attribute_groups: Vec<IppAttributeGroup>,
    /// Document data (everything after the end-of-attributes tag).
    pub document_data: Vec<u8>,
}

impl IppRequest {
    /// Get the first operation-attributes group.
    pub fn operation_attributes(&self) -> Option<&IppAttributeGroup> {
        self.attribute_groups
            .iter()
            .find(|g| g.delimiter == TAG_OPERATION_ATTRIBUTES)
    }

    /// Get the first job-attributes group.
    #[allow(dead_code)]
    pub fn job_attributes(&self) -> Option<&IppAttributeGroup> {
        self.attribute_groups
            .iter()
            .find(|g| g.delimiter == TAG_JOB_ATTRIBUTES)
    }
}

// ---------------------------------------------------------------------------
// IPP binary parser
// ---------------------------------------------------------------------------

/// Parse a raw IPP message body into an `IppRequest`.
///
/// Implements the binary encoding described in RFC 8010 SS3.1.  The format is:
///
/// ```text
/// version-number:  2 bytes (major, minor)
/// operation-id:    2 bytes (big-endian u16)
/// request-id:      4 bytes (big-endian u32)
/// attribute-groups: variable
///   delimiter-tag: 1 byte
///   attributes:    variable
///     value-tag:    1 byte
///     name-length:  2 bytes (big-endian u16)
///     name:         name-length bytes
///     value-length: 2 bytes (big-endian u16)
///     value:        value-length bytes
/// end-of-attributes-tag: 1 byte (0x03)
/// document-data: remainder
/// ```
pub fn parse_ipp_request(data: &[u8]) -> std::result::Result<IppRequest, String> {
    if data.len() < 8 {
        return Err(format!(
            "IPP request too short: {} bytes (minimum 8)",
            data.len()
        ));
    }

    let version_major = data[0];
    let version_minor = data[1];
    let operation_id = u16::from_be_bytes([data[2], data[3]]);
    let request_id = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);

    let mut pos = 8;
    let mut attribute_groups: Vec<IppAttributeGroup> = Vec::new();
    let mut current_group: Option<IppAttributeGroup> = None;

    while pos < data.len() {
        let tag = data[pos];

        // Delimiter tags are in the range 0x00..=0x0F.
        if tag <= 0x0F {
            // Save any in-progress group before starting a new one.
            if let Some(group) = current_group.take() {
                attribute_groups.push(group);
            }

            if tag == TAG_END_OF_ATTRIBUTES {
                pos += 1;
                break;
            }

            // Start a new attribute group.
            current_group = Some(IppAttributeGroup {
                delimiter: tag,
                attributes: Vec::new(),
            });
            pos += 1;
            continue;
        }

        // Otherwise this is a value tag -- parse a full attribute.
        let value_tag = tag;
        pos += 1;

        if pos + 2 > data.len() {
            return Err("truncated name-length field".into());
        }
        let name_length = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        if pos + name_length > data.len() {
            return Err("truncated attribute name".into());
        }
        let name = String::from_utf8_lossy(&data[pos..pos + name_length]).to_string();
        pos += name_length;

        if pos + 2 > data.len() {
            return Err("truncated value-length field".into());
        }
        let value_length = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        if pos + value_length > data.len() {
            return Err("truncated attribute value".into());
        }
        let value = data[pos..pos + value_length].to_vec();
        pos += value_length;

        let attr = IppAttribute {
            value_tag,
            name,
            value,
        };

        if let Some(ref mut group) = current_group {
            group.attributes.push(attr);
        } else {
            // Attribute outside a group -- discard per spec (malformed).
            warn!("IPP attribute outside of any group -- discarded");
        }
    }

    // Flush the last group if we hit end-of-data before end-of-attributes.
    if let Some(group) = current_group.take() {
        attribute_groups.push(group);
    }

    // Everything remaining is document data.
    let document_data = if pos < data.len() {
        data[pos..].to_vec()
    } else {
        Vec::new()
    };

    Ok(IppRequest {
        version_major,
        version_minor,
        operation_id,
        request_id,
        attribute_groups,
        document_data,
    })
}

// ---------------------------------------------------------------------------
// IPP binary response builder
// ---------------------------------------------------------------------------

/// Builder for constructing IPP response messages.
///
/// Produces the binary encoding described in RFC 8010 SS3.4.
pub struct IppResponseBuilder {
    /// Accumulated response bytes.
    pub buf: Vec<u8>,
    /// Whether we are currently inside an attribute group.
    pub in_group: bool,
}

impl IppResponseBuilder {
    /// Create a new response with the given status code and request-id.
    pub fn new(status_code: u16, request_id: u32) -> Self {
        let mut buf = Vec::with_capacity(256);
        // version-number: IPP 1.1
        buf.push(IPP_VERSION_MAJOR);
        buf.push(IPP_VERSION_MINOR);
        // status-code
        buf.extend_from_slice(&status_code.to_be_bytes());
        // request-id (echoed from the request)
        buf.extend_from_slice(&request_id.to_be_bytes());
        Self {
            buf,
            in_group: false,
        }
    }

    /// Start a new attribute group.
    pub fn begin_group(&mut self, delimiter: u8) -> &mut Self {
        self.buf.push(delimiter);
        self.in_group = true;
        self
    }

    /// Write a textWithoutLanguage attribute.
    pub fn text(&mut self, name: &str, value: &str) -> &mut Self {
        self.write_attr(VALUE_TAG_TEXT, name, value.as_bytes())
    }

    /// Write a nameWithoutLanguage attribute.
    pub fn name_attr(&mut self, name: &str, value: &str) -> &mut Self {
        self.write_attr(VALUE_TAG_NAME, name, value.as_bytes())
    }

    /// Write a keyword attribute.
    pub fn keyword(&mut self, name: &str, value: &str) -> &mut Self {
        self.write_attr(VALUE_TAG_KEYWORD, name, value.as_bytes())
    }

    /// Write an additional keyword value for a 1setOf keyword.
    ///
    /// Per RFC 8010 SS3.1.4, additional values have name-length = 0.
    pub fn keyword_additional(&mut self, value: &str) -> &mut Self {
        self.write_attr(VALUE_TAG_KEYWORD, "", value.as_bytes())
    }

    /// Write a URI attribute.
    pub fn uri(&mut self, name: &str, value: &str) -> &mut Self {
        self.write_attr(VALUE_TAG_URI, name, value.as_bytes())
    }

    /// Write a charset attribute.
    pub fn charset(&mut self, name: &str, value: &str) -> &mut Self {
        self.write_attr(VALUE_TAG_CHARSET, name, value.as_bytes())
    }

    /// Write a naturalLanguage attribute.
    pub fn natural_language(&mut self, name: &str, value: &str) -> &mut Self {
        self.write_attr(VALUE_TAG_NATURAL_LANGUAGE, name, value.as_bytes())
    }

    /// Write an integer attribute.
    pub fn integer(&mut self, name: &str, value: i32) -> &mut Self {
        self.write_attr(VALUE_TAG_INTEGER, name, &value.to_be_bytes())
    }

    /// Write an enum attribute (same wire encoding as integer).
    pub fn enum_attr(&mut self, name: &str, value: i32) -> &mut Self {
        self.write_attr(VALUE_TAG_ENUM, name, &value.to_be_bytes())
    }

    /// Write a boolean attribute.
    pub fn boolean(&mut self, name: &str, value: bool) -> &mut Self {
        self.write_attr(VALUE_TAG_BOOLEAN, name, &[if value { 0x01 } else { 0x00 }])
    }

    /// Write a raw attribute (value-tag, name, value bytes).
    pub fn write_attr(&mut self, value_tag: u8, name: &str, value: &[u8]) -> &mut Self {
        // value-tag: 1 byte
        self.buf.push(value_tag);
        // name-length: 2 bytes (big-endian)
        let name_bytes = name.as_bytes();
        self.buf
            .extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
        // name
        self.buf.extend_from_slice(name_bytes);
        // value-length: 2 bytes (big-endian)
        self.buf
            .extend_from_slice(&(value.len() as u16).to_be_bytes());
        // value
        self.buf.extend_from_slice(value);
        self
    }

    /// Finalise the response: write end-of-attributes tag and return bytes.
    pub fn build(mut self) -> Vec<u8> {
        self.buf.push(TAG_END_OF_ATTRIBUTES);
        self.buf
    }
}

// ---------------------------------------------------------------------------
// Minimal HTTP request parser
// ---------------------------------------------------------------------------

/// Result of parsing a minimal HTTP POST request for IPP.
struct HttpRequest {
    /// The Content-Length value, if present.
    #[allow(dead_code)]
    content_length: Option<usize>,
    /// The offset where the HTTP body (IPP payload) begins.
    body_offset: usize,
}

/// Parse the bare minimum of an HTTP/1.1 POST request to find the body.
///
/// IPP over HTTP uses `Content-Type: application/ipp`.  We only need to
/// find where the headers end (double CRLF) and extract Content-Length.
/// Returns `None` if the data doesn't look like an HTTP request (in which
/// case we treat the entire payload as raw IPP).
fn parse_http_envelope(data: &[u8]) -> Option<HttpRequest> {
    // Look for the end of headers: \r\n\r\n
    let header_end = find_subsequence(data, b"\r\n\r\n")?;
    let body_offset = header_end + 4;

    // Extract Content-Length if present.
    let headers = &data[..header_end];
    let headers_str = String::from_utf8_lossy(headers);
    let content_length = headers_str
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|line| line.split(':').nth(1))
        .and_then(|val| val.trim().parse::<usize>().ok());

    Some(HttpRequest {
        content_length,
        body_offset,
    })
}

/// Find the first occurrence of `needle` in `haystack`.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ---------------------------------------------------------------------------
// Shared state passed to connection handlers
// ---------------------------------------------------------------------------

/// State shared across all connection-handling tasks.
struct SharedState {
    /// The job queue for persisting incoming print jobs.
    job_queue: Arc<Mutex<JobQueue>>,
    /// Counter of active connections (for the UI).
    active_connections: Arc<AtomicU32>,
    /// The port we are listening on (used to build printer-uri).
    port: u16,
    /// Internal job ID counter (IPP uses sequential integers, not UUIDs).
    next_ipp_job_id: Arc<AtomicU32>,
    /// Map from IPP integer job-id to our internal UUID-based JobId.
    ipp_to_internal: Arc<Mutex<HashMap<i32, JobId>>>,
    /// Directory for persisting document data files.
    data_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// IppServer
// ---------------------------------------------------------------------------

/// Embedded IPP print server.
///
/// Binds a TCP listener and accepts connections from other devices that want
/// to print to this phone/tablet.  Incoming print jobs are placed into the
/// local job queue for user review.
pub struct IppServer {
    /// The TCP port to listen on.
    port: u16,
    /// Current lifecycle state of the server.
    status: ServerStatus,
    /// Notification handle used to signal a graceful shutdown.
    shutdown_signal: Arc<Notify>,
    /// Handle to the Tokio task running the accept loop.
    task_handle: Option<JoinHandle<()>>,
    /// Counter of currently active TCP connections.
    active_connections: Arc<AtomicU32>,
    /// Handle to the mDNS daemon for service advertisement.
    mdns_daemon: Option<mdns_sd::ServiceDaemon>,
    /// The mDNS service fullname (for unregistration on stop).
    mdns_fullname: Option<String>,
    /// Root directory for persistent data (documents subdirectory lives here).
    data_dir: PathBuf,
}

impl IppServer {
    /// Create a new server bound to the given port.
    ///
    /// The server is created in `Stopped` state.  Call [`start`] to begin
    /// accepting connections.
    ///
    /// `data_dir` specifies the root directory where document data is persisted.
    /// If `None`, a temporary directory is used (suitable for tests).
    pub fn new(port: Option<u16>, data_dir: Option<PathBuf>) -> Self {
        let data_dir = data_dir.unwrap_or_else(|| std::env::temp_dir().join("presswerk"));
        Self {
            port: port.unwrap_or(DEFAULT_PORT),
            status: ServerStatus::Stopped,
            shutdown_signal: Arc::new(Notify::new()),
            task_handle: None,
            active_connections: Arc::new(AtomicU32::new(0)),
            mdns_daemon: None,
            mdns_fullname: None,
            data_dir,
        }
    }

    /// Return the port this server will bind to (or is bound to).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Return the current server status.
    pub fn status(&self) -> ServerStatus {
        self.status
    }

    /// Return the number of currently active client connections.
    pub fn active_connections(&self) -> u32 {
        self.active_connections.load(Ordering::Relaxed)
    }

    /// Return the filesystem path where a document with the given hash is
    /// (or would be) stored.
    ///
    /// The path is `{data_dir}/documents/{hash}.dat`.  This method does not
    /// check whether the file actually exists on disk.
    pub fn document_path(&self, hash: &str) -> PathBuf {
        self.data_dir.join("documents").join(format!("{hash}.dat"))
    }

    /// Read the raw document bytes for the given content hash from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file does not exist or cannot be read.
    pub fn retrieve_document(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.document_path(hash);
        std::fs::read(&path).map_err(|e| {
            PresswerkError::PrintServer(format!("read document {}: {e}", path.display()))
        })
    }

    /// Start the IPP print server.
    ///
    /// Binds a TCP listener on `0.0.0.0:{port}` and spawns a Tokio task that
    /// accepts incoming connections.  Each connection is handled in its own
    /// spawned task.  Also registers the printer via mDNS for network
    /// discovery.
    ///
    /// The `job_queue` is shared with the rest of the application and receives
    /// incoming print jobs from network clients.
    ///
    /// # Errors
    ///
    /// Returns an error if the port is already in use or the listener cannot
    /// be created.
    pub async fn start(&mut self, job_queue: Arc<Mutex<JobQueue>>) -> Result<()> {
        if self.status == ServerStatus::Running {
            debug!(port = self.port, "IPP server already running");
            return Ok(());
        }

        self.status = ServerStatus::Starting;

        let bind_addr: SocketAddr = ([0, 0, 0, 0], self.port).into();
        let listener = TcpListener::bind(bind_addr)
            .await
            .map_err(|e| PresswerkError::PrintServer(format!("bind {bind_addr}: {e}")))?;

        info!(port = self.port, "IPP print server listening");

        // Register via mDNS so other devices discover us.
        self.register_mdns();

        // Ensure the documents subdirectory exists for persisting print data.
        let documents_dir = self.data_dir.join("documents");
        std::fs::create_dir_all(&documents_dir).map_err(|e| {
            PresswerkError::PrintServer(format!(
                "create documents directory {}: {e}",
                documents_dir.display()
            ))
        })?;
        info!(path = %documents_dir.display(), "document storage directory ready");

        let shutdown = Arc::clone(&self.shutdown_signal);
        let connections = Arc::clone(&self.active_connections);
        let port = self.port;

        let shared = Arc::new(SharedState {
            job_queue,
            active_connections: connections,
            port,
            next_ipp_job_id: Arc::new(AtomicU32::new(1)),
            ipp_to_internal: Arc::new(Mutex::new(HashMap::new())),
            data_dir: self.data_dir.clone(),
        });

        let handle = tokio::spawn(async move {
            Self::accept_loop(listener, shutdown, port, shared).await;
        });

        self.task_handle = Some(handle);
        self.status = ServerStatus::Running;
        Ok(())
    }

    /// Gracefully stop the server.
    ///
    /// Signals the accept loop to exit and awaits its completion.  Existing
    /// connections that are mid-transfer will be allowed to finish.
    /// Unregisters the mDNS service advertisement.
    pub async fn stop(&mut self) -> Result<()> {
        if self.status != ServerStatus::Running {
            return Ok(());
        }

        info!(port = self.port, "stopping IPP print server");

        // Unregister mDNS service.
        self.unregister_mdns();

        self.shutdown_signal.notify_one();

        if let Some(handle) = self.task_handle.take() {
            handle
                .await
                .map_err(|e| PresswerkError::PrintServer(format!("task join: {e}")))?;
        }

        self.status = ServerStatus::Stopped;
        info!(port = self.port, "IPP print server stopped");
        Ok(())
    }

    /// Register this printer via mDNS-SD as `_ipp._tcp.local.`.
    ///
    /// If mDNS registration fails we log a warning but do not fail the
    /// server start -- the printer will still work via direct IP.
    fn register_mdns(&mut self) {
        let daemon = match mdns_sd::ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                warn!(error = %e, "failed to create mDNS daemon for advertisement");
                return;
            }
        };

        // Build TXT record properties.
        let properties = [
            ("txtvers", "1"),
            ("qtotal", "1"),
            ("rp", "ipp/print"),
            ("ty", PRINTER_NAME),
            ("pdl", "application/pdf,image/jpeg,image/png,text/plain"),
            ("Color", "T"),
            ("Duplex", "T"),
            ("URF", "none"),
        ];

        let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "presswerk".into());

        let service_name = PRINTER_NAME.to_string();

        match mdns_sd::ServiceInfo::new(
            IPP_SERVICE_TYPE,
            &service_name,
            &format!("{hostname}.local."),
            "", // empty = auto-detect IP
            self.port,
            &properties[..],
        ) {
            Ok(service_info) => {
                let fullname = service_info.get_fullname().to_owned();
                match daemon.register(service_info) {
                    Ok(_) => {
                        info!(
                            service_type = IPP_SERVICE_TYPE,
                            name = %service_name,
                            port = self.port,
                            "mDNS service registered"
                        );
                        self.mdns_fullname = Some(fullname);
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to register mDNS service");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to create mDNS ServiceInfo");
            }
        }

        self.mdns_daemon = Some(daemon);
    }

    /// Unregister the mDNS service and shut down the daemon.
    fn unregister_mdns(&mut self) {
        if let Some(daemon) = self.mdns_daemon.take() {
            if let Some(fullname) = self.mdns_fullname.take() {
                match daemon.unregister(&fullname) {
                    Ok(_) => {
                        info!(name = %fullname, "mDNS service unregistered");
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to unregister mDNS service");
                    }
                }
            }
            if let Err(e) = daemon.shutdown() {
                warn!(error = %e, "failed to shut down mDNS daemon");
            }
        }
    }

    /// The main accept loop.
    ///
    /// Runs until the shutdown signal is received.  Each incoming connection
    /// is handed off to [`handle_connection`] in a separate task.
    async fn accept_loop(
        listener: TcpListener,
        shutdown: Arc<Notify>,
        port: u16,
        shared: Arc<SharedState>,
    ) {
        loop {
            tokio::select! {
                // Wait for the shutdown signal.
                _ = shutdown.notified() => {
                    debug!(port, "accept loop received shutdown signal");
                    break;
                }

                // Accept a new connection.
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, peer_addr)) => {
                            info!(peer = %peer_addr, "incoming IPP connection");
                            let state = Arc::clone(&shared);
                            tokio::spawn(async move {
                                state.active_connections.fetch_add(1, Ordering::Relaxed);
                                if let Err(e) = Self::handle_connection(stream, peer_addr, state.clone()).await {
                                    warn!(
                                        peer = %peer_addr,
                                        error = %e,
                                        "connection handler error"
                                    );
                                }
                                state.active_connections.fetch_sub(1, Ordering::Relaxed);
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "failed to accept connection");
                        }
                    }
                }
            }
        }
    }

    /// Handle a single incoming TCP connection.
    ///
    /// Reads the full request, strips HTTP framing if present, parses the
    /// IPP binary payload, dispatches to the appropriate operation handler,
    /// and writes back an IPP response wrapped in a minimal HTTP response.
    async fn handle_connection(
        mut stream: tokio::net::TcpStream,
        peer_addr: SocketAddr,
        state: Arc<SharedState>,
    ) -> Result<()> {
        let mut buf = Vec::with_capacity(8192);

        // Read up to MAX_REQUEST_BYTES.
        let mut limited = (&mut stream).take(MAX_REQUEST_BYTES as u64);
        let bytes_read = limited
            .read_to_end(&mut buf)
            .await
            .map_err(|e| PresswerkError::PrintServer(format!("read from {peer_addr}: {e}")))?;

        debug!(
            peer = %peer_addr,
            bytes = bytes_read,
            "received IPP request data"
        );

        if bytes_read == 0 {
            debug!(peer = %peer_addr, "empty request -- closing connection");
            return Ok(());
        }

        // Strip HTTP envelope if present.  Some IPP clients send raw IPP
        // over TCP (especially in test environments), others wrap it in HTTP.
        let ipp_body = match parse_http_envelope(&buf) {
            Some(http_req) => {
                debug!(
                    peer = %peer_addr,
                    body_offset = http_req.body_offset,
                    content_length = ?http_req.content_length,
                    "HTTP envelope detected"
                );
                &buf[http_req.body_offset..]
            }
            None => {
                debug!(peer = %peer_addr, "no HTTP envelope -- treating as raw IPP");
                &buf[..]
            }
        };

        // Parse the IPP request.
        let ipp_request = match parse_ipp_request(ipp_body) {
            Ok(req) => req,
            Err(e) => {
                warn!(peer = %peer_addr, error = %e, "malformed IPP request");
                let response = build_error_response(
                    STATUS_CLIENT_ERROR_BAD_REQUEST,
                    0, // no valid request-id
                    &format!("Malformed IPP request: {e}"),
                );
                send_response(&mut stream, &response).await?;
                return Ok(());
            }
        };

        debug!(
            peer = %peer_addr,
            version = %format!("{}.{}", ipp_request.version_major, ipp_request.version_minor),
            operation_id = %format!("0x{:04X}", ipp_request.operation_id),
            request_id = ipp_request.request_id,
            groups = ipp_request.attribute_groups.len(),
            doc_bytes = ipp_request.document_data.len(),
            "parsed IPP request"
        );

        // Dispatch to the appropriate operation handler.
        let response_bytes = dispatch_operation(&ipp_request, peer_addr, &state);

        send_response(&mut stream, &response_bytes).await?;

        info!(
            peer = %peer_addr,
            operation = %format!("0x{:04X}", ipp_request.operation_id),
            response_bytes = response_bytes.len(),
            "IPP response sent"
        );

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Operation dispatch
// ---------------------------------------------------------------------------

/// Route the parsed IPP request to the appropriate handler.
fn dispatch_operation(request: &IppRequest, peer_addr: SocketAddr, state: &SharedState) -> Vec<u8> {
    match request.operation_id {
        OP_PRINT_JOB => handle_print_job(request, peer_addr, state),
        OP_VALIDATE_JOB => handle_validate_job(request),
        OP_CANCEL_JOB => handle_cancel_job(request, state),
        OP_GET_JOBS => handle_get_jobs(request, state),
        OP_GET_PRINTER_ATTRIBUTES => handle_get_printer_attributes(request, state),
        _ => {
            warn!(
                operation = %format!("0x{:04X}", request.operation_id),
                "unsupported IPP operation"
            );
            build_error_response(
                STATUS_SERVER_ERROR_OPERATION_NOT_SUPPORTED,
                request.request_id,
                &format!("Operation 0x{:04X} is not supported", request.operation_id),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Operation handlers
// ---------------------------------------------------------------------------

/// Handle a Print-Job (0x0002) request.
///
/// Creates a new `PrintJob`, stores it in the `JobQueue`, and returns
/// a response with the job-id and job-state.
fn handle_print_job(request: &IppRequest, peer_addr: SocketAddr, state: &SharedState) -> Vec<u8> {
    let op_attrs = request.operation_attributes();

    // Extract the document name from operation attributes.
    let document_name = op_attrs
        .and_then(|g| g.get_string("job-name"))
        .or_else(|| op_attrs.and_then(|g| g.get_string("document-name")))
        .unwrap_or_else(|| "Untitled Document".into());

    // Determine the document format.
    let document_format = op_attrs
        .and_then(|g| g.get_string("document-format"))
        .unwrap_or_else(|| "application/octet-stream".into());

    let document_type = mime_to_document_type(&document_format);

    // Compute SHA-256 hash of the document data.
    let document_hash = if request.document_data.is_empty() {
        "empty".into()
    } else {
        let mut hasher = Sha256::new();
        hasher.update(&request.document_data);
        hex::encode(hasher.finalize())
    };

    // Create the internal print job.
    let ip = peer_addr.ip();
    let job = PrintJob::new(
        JobSource::Network { remote_addr: ip },
        document_type,
        document_name.clone(),
        document_hash.clone(),
    );

    let internal_job_id = job.id;

    // Assign an IPP integer job-id.
    let ipp_job_id = state.next_ipp_job_id.fetch_add(1, Ordering::Relaxed) as i32;

    // Map IPP job-id to internal JobId.
    if let Ok(mut map) = state.ipp_to_internal.lock() {
        map.insert(ipp_job_id, internal_job_id);
    }

    // Insert into the job queue.
    match state.job_queue.lock() {
        Ok(queue) => {
            if let Err(e) = queue.insert_job(&job) {
                error!(error = %e, "failed to insert job into queue");
                return build_error_response(
                    STATUS_SERVER_ERROR_INTERNAL,
                    request.request_id,
                    &format!("Failed to enqueue job: {e}"),
                );
            }
        }
        Err(e) => {
            error!(error = %e, "job queue lock poisoned");
            return build_error_response(
                STATUS_SERVER_ERROR_INTERNAL,
                request.request_id,
                "Internal server error: queue lock poisoned",
            );
        }
    }

    // Persist document data to disk using content-addressed storage.
    // If the file already exists we skip the write -- same hash means
    // identical content, so the existing file is already correct.
    if !request.document_data.is_empty() {
        let doc_path = state
            .data_dir
            .join("documents")
            .join(format!("{document_hash}.dat"));

        if doc_path.exists() {
            info!(
                hash = %document_hash,
                path = %doc_path.display(),
                "document already on disk (content-addressed); skipping write"
            );
        } else {
            match std::fs::write(&doc_path, &request.document_data) {
                Ok(()) => {
                    info!(
                        hash = %document_hash,
                        path = %doc_path.display(),
                        bytes = request.document_data.len(),
                        "document data persisted to disk"
                    );
                }
                Err(e) => {
                    error!(
                        hash = %document_hash,
                        path = %doc_path.display(),
                        error = %e,
                        "failed to persist document data to disk"
                    );
                    return build_error_response(
                        STATUS_SERVER_ERROR_INTERNAL,
                        request.request_id,
                        &format!("Failed to store document data: {e}"),
                    );
                }
            }
        }
    }

    info!(
        ipp_job_id = ipp_job_id,
        internal_id = %internal_job_id,
        doc_name = %document_name,
        doc_bytes = request.document_data.len(),
        "Print-Job accepted"
    );

    // Build a successful response.
    let printer_uri = format!("ipp://localhost:{}/ipp/print", state.port);

    let mut resp = IppResponseBuilder::new(STATUS_OK, request.request_id);
    resp.begin_group(TAG_OPERATION_ATTRIBUTES)
        .charset("attributes-charset", "utf-8")
        .natural_language("attributes-natural-language", "en")
        .text("status-message", "successful-ok");

    resp.begin_group(TAG_JOB_ATTRIBUTES)
        .integer("job-id", ipp_job_id)
        .uri("job-uri", &format!("{printer_uri}/jobs/{ipp_job_id}"))
        .enum_attr("job-state", JOB_STATE_PENDING)
        .keyword("job-state-reasons", "none");

    resp.build()
}

/// Handle a Validate-Job (0x0004) request.
///
/// Simply returns successful-ok -- the request is syntactically valid.
fn handle_validate_job(request: &IppRequest) -> Vec<u8> {
    debug!("Validate-Job: returning successful-ok");

    let mut resp = IppResponseBuilder::new(STATUS_OK, request.request_id);
    resp.begin_group(TAG_OPERATION_ATTRIBUTES)
        .charset("attributes-charset", "utf-8")
        .natural_language("attributes-natural-language", "en")
        .text("status-message", "successful-ok");

    resp.build()
}

/// Handle a Cancel-Job (0x0008) request.
///
/// Looks up the job by IPP job-id and marks it as cancelled.
fn handle_cancel_job(request: &IppRequest, state: &SharedState) -> Vec<u8> {
    let op_attrs = request.operation_attributes();

    let ipp_job_id = op_attrs.and_then(|g| g.get_integer("job-id"));

    let ipp_job_id = match ipp_job_id {
        Some(id) => id,
        None => {
            warn!("Cancel-Job: missing job-id attribute");
            return build_error_response(
                STATUS_CLIENT_ERROR_BAD_REQUEST,
                request.request_id,
                "Missing required job-id attribute",
            );
        }
    };

    // Look up the internal JobId.
    let internal_id = state
        .ipp_to_internal
        .lock()
        .ok()
        .and_then(|map| map.get(&ipp_job_id).copied());

    let internal_id = match internal_id {
        Some(id) => id,
        None => {
            warn!(ipp_job_id, "Cancel-Job: job not found");
            return build_error_response(
                STATUS_CLIENT_ERROR_NOT_FOUND,
                request.request_id,
                &format!("Job {ipp_job_id} not found"),
            );
        }
    };

    // Update the job status in the queue.
    match state.job_queue.lock() {
        Ok(queue) => {
            if let Err(e) = queue.update_status(&internal_id, JobStatus::Cancelled, None) {
                error!(error = %e, "Cancel-Job: failed to update status");
                return build_error_response(
                    STATUS_SERVER_ERROR_INTERNAL,
                    request.request_id,
                    &format!("Failed to cancel job: {e}"),
                );
            }
        }
        Err(e) => {
            error!(error = %e, "job queue lock poisoned");
            return build_error_response(
                STATUS_SERVER_ERROR_INTERNAL,
                request.request_id,
                "Internal server error: queue lock poisoned",
            );
        }
    }

    info!(ipp_job_id, "Cancel-Job: job cancelled");

    let mut resp = IppResponseBuilder::new(STATUS_OK, request.request_id);
    resp.begin_group(TAG_OPERATION_ATTRIBUTES)
        .charset("attributes-charset", "utf-8")
        .natural_language("attributes-natural-language", "en")
        .text("status-message", "successful-ok");

    resp.build()
}

/// Handle a Get-Jobs (0x000A) request.
///
/// Returns all jobs from the queue with their IPP attributes.
fn handle_get_jobs(request: &IppRequest, state: &SharedState) -> Vec<u8> {
    let jobs = match state.job_queue.lock() {
        Ok(queue) => match queue.get_all_jobs() {
            Ok(jobs) => jobs,
            Err(e) => {
                error!(error = %e, "Get-Jobs: failed to retrieve jobs");
                return build_error_response(
                    STATUS_SERVER_ERROR_INTERNAL,
                    request.request_id,
                    &format!("Failed to retrieve jobs: {e}"),
                );
            }
        },
        Err(e) => {
            error!(error = %e, "job queue lock poisoned");
            return build_error_response(
                STATUS_SERVER_ERROR_INTERNAL,
                request.request_id,
                "Internal server error: queue lock poisoned",
            );
        }
    };

    // We need the reverse mapping from internal JobId to IPP integer id.
    let id_map: HashMap<JobId, i32> = state
        .ipp_to_internal
        .lock()
        .map(|map| map.iter().map(|(&k, &v)| (v, k)).collect())
        .unwrap_or_default();

    let printer_uri = format!("ipp://localhost:{}/ipp/print", state.port);

    let mut resp = IppResponseBuilder::new(STATUS_OK, request.request_id);
    resp.begin_group(TAG_OPERATION_ATTRIBUTES)
        .charset("attributes-charset", "utf-8")
        .natural_language("attributes-natural-language", "en")
        .text("status-message", "successful-ok");

    for job in &jobs {
        let ipp_id = id_map.get(&job.id).copied().unwrap_or(0);
        let job_state = job_status_to_ipp_state(job.status);

        resp.begin_group(TAG_JOB_ATTRIBUTES)
            .integer("job-id", ipp_id)
            .uri("job-uri", &format!("{printer_uri}/jobs/{ipp_id}"))
            .name_attr("job-name", &job.document_name)
            .enum_attr("job-state", job_state)
            .keyword("job-state-reasons", job_state_reason(job.status));
    }

    debug!(count = jobs.len(), "Get-Jobs: returning job list");

    resp.build()
}

/// Handle a Get-Printer-Attributes (0x000B) request.
///
/// Returns the printer's capabilities and current state.
fn handle_get_printer_attributes(request: &IppRequest, state: &SharedState) -> Vec<u8> {
    let printer_uri = format!("ipp://localhost:{}/ipp/print", state.port);

    let mut resp = IppResponseBuilder::new(STATUS_OK, request.request_id);
    resp.begin_group(TAG_OPERATION_ATTRIBUTES)
        .charset("attributes-charset", "utf-8")
        .natural_language("attributes-natural-language", "en")
        .text("status-message", "successful-ok");

    resp.begin_group(TAG_PRINTER_ATTRIBUTES)
        // Identification
        .uri("printer-uri-supported", &printer_uri)
        .name_attr("printer-name", PRINTER_NAME)
        .text("printer-info", "Presswerk mobile print router")
        .text("printer-make-and-model", "Presswerk Virtual Printer 1.0")
        .text("printer-location", "Mobile Device")
        // State
        .enum_attr("printer-state", PRINTER_STATE_IDLE)
        .keyword("printer-state-reasons", "none")
        // Capabilities
        .keyword("ipp-versions-supported", "1.1")
        .keyword("operations-supported", "Print-Job")
        .keyword_additional("Validate-Job")
        .keyword_additional("Cancel-Job")
        .keyword_additional("Get-Jobs")
        .keyword_additional("Get-Printer-Attributes")
        // Supported document formats
        .keyword("document-format-supported", "application/pdf")
        .keyword_additional("image/jpeg")
        .keyword_additional("image/png")
        .keyword_additional("text/plain")
        .keyword_additional("application/octet-stream")
        .keyword("document-format-default", "application/pdf")
        // Media
        .keyword("media-supported", "iso_a4_210x297mm")
        .keyword_additional("iso_a3_297x420mm")
        .keyword_additional("iso_a5_148x210mm")
        .keyword_additional("na_letter_8.5x11in")
        .keyword_additional("na_legal_8.5x14in")
        .keyword("media-default", "iso_a4_210x297mm")
        // Duplex
        .keyword("sides-supported", "one-sided")
        .keyword_additional("two-sided-long-edge")
        .keyword_additional("two-sided-short-edge")
        .keyword("sides-default", "one-sided")
        // Color
        .boolean("color-supported", true)
        // Charset/language
        .charset("charset-configured", "utf-8")
        .charset("charset-supported", "utf-8")
        .natural_language("natural-language-configured", "en")
        .natural_language("generated-natural-language-supported", "en")
        // URI security and auth
        .keyword("uri-security-supported", "none")
        .keyword("uri-authentication-supported", "none")
        // Compression
        .keyword("compression-supported", "none")
        // PDL override
        .keyword("pdl-override-supported", "not-attempted");

    debug!("Get-Printer-Attributes: returning capabilities");

    resp.build()
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Build a minimal error response with the given status code.
fn build_error_response(status: u16, request_id: u32, message: &str) -> Vec<u8> {
    let mut resp = IppResponseBuilder::new(status, request_id);
    resp.begin_group(TAG_OPERATION_ATTRIBUTES)
        .charset("attributes-charset", "utf-8")
        .natural_language("attributes-natural-language", "en")
        .text("status-message", message);
    resp.build()
}

/// Send an IPP response wrapped in a minimal HTTP/1.1 200 OK.
async fn send_response(stream: &mut tokio::net::TcpStream, ipp_body: &[u8]) -> Result<()> {
    let http_response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/ipp\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        ipp_body.len()
    );

    stream
        .write_all(http_response.as_bytes())
        .await
        .map_err(|e| PresswerkError::PrintServer(format!("write HTTP headers: {e}")))?;

    stream
        .write_all(ipp_body)
        .await
        .map_err(|e| PresswerkError::PrintServer(format!("write IPP body: {e}")))?;

    stream
        .flush()
        .await
        .map_err(|e| PresswerkError::PrintServer(format!("flush: {e}")))?;

    Ok(())
}

/// Map a MIME type string to a `DocumentType`.
fn mime_to_document_type(mime: &str) -> DocumentType {
    match mime {
        "application/pdf" => DocumentType::Pdf,
        "image/jpeg" => DocumentType::Jpeg,
        "image/png" => DocumentType::Png,
        "image/tiff" => DocumentType::Tiff,
        "text/plain" => DocumentType::PlainText,
        _ => DocumentType::NativeDelegate,
    }
}

/// Map internal `JobStatus` to an IPP job-state integer.
fn job_status_to_ipp_state(status: JobStatus) -> i32 {
    match status {
        JobStatus::Pending => JOB_STATE_PENDING,
        JobStatus::Held => JOB_STATE_HELD,
        JobStatus::Processing => JOB_STATE_PROCESSING,
        JobStatus::Completed => JOB_STATE_COMPLETED,
        JobStatus::Cancelled => JOB_STATE_CANCELED,
        JobStatus::Failed => JOB_STATE_ABORTED,
        JobStatus::RetryPending => JOB_STATE_PENDING,
    }
}

/// Map internal `JobStatus` to an IPP job-state-reasons keyword.
fn job_state_reason(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "none",
        JobStatus::Held => "job-hold-until-specified",
        JobStatus::Processing => "job-printing",
        JobStatus::Completed => "job-completed-successfully",
        JobStatus::Cancelled => "job-canceled-by-user",
        JobStatus::Failed => "aborted-by-system",
        JobStatus::RetryPending => "job-queued-for-marker",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Original tests (preserved) -----------------------------------------

    #[test]
    fn default_port_is_631() {
        let server = IppServer::new(None, None);
        assert_eq!(server.port(), 631);
    }

    #[test]
    fn custom_port_is_respected() {
        let server = IppServer::new(Some(9100), None);
        assert_eq!(server.port(), 9100);
    }

    #[test]
    fn initial_status_is_stopped() {
        let server = IppServer::new(None, None);
        assert_eq!(server.status(), ServerStatus::Stopped);
    }

    // -- IPP request parsing ------------------------------------------------

    /// Build a minimal IPP request for testing.
    fn build_test_ipp_request(
        operation_id: u16,
        request_id: u32,
        attributes: &[(u8, &str, &[u8])], // (value_tag, name, value)
        document_data: &[u8],
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        // version 1.1
        buf.push(IPP_VERSION_MAJOR);
        buf.push(IPP_VERSION_MINOR);
        // operation-id
        buf.extend_from_slice(&operation_id.to_be_bytes());
        // request-id
        buf.extend_from_slice(&request_id.to_be_bytes());
        // operation attributes group
        buf.push(TAG_OPERATION_ATTRIBUTES);
        // Required: attributes-charset
        write_test_attr(&mut buf, VALUE_TAG_CHARSET, "attributes-charset", b"utf-8");
        // Required: attributes-natural-language
        write_test_attr(
            &mut buf,
            VALUE_TAG_NATURAL_LANGUAGE,
            "attributes-natural-language",
            b"en",
        );
        // Additional attributes
        for &(tag, name, value) in attributes {
            write_test_attr(&mut buf, tag, name, value);
        }
        // end-of-attributes
        buf.push(TAG_END_OF_ATTRIBUTES);
        // document data
        buf.extend_from_slice(document_data);
        buf
    }

    /// Write a single attribute to a buffer.
    fn write_test_attr(buf: &mut Vec<u8>, value_tag: u8, name: &str, value: &[u8]) {
        buf.push(value_tag);
        buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
        buf.extend_from_slice(value);
    }

    #[test]
    fn parse_minimal_ipp_request() {
        let data = build_test_ipp_request(OP_GET_PRINTER_ATTRIBUTES, 42, &[], &[]);
        let req = parse_ipp_request(&data).expect("parse should succeed");

        assert_eq!(req.version_major, 1);
        assert_eq!(req.version_minor, 1);
        assert_eq!(req.operation_id, OP_GET_PRINTER_ATTRIBUTES);
        assert_eq!(req.request_id, 42);
        assert_eq!(req.attribute_groups.len(), 1);
        assert!(req.document_data.is_empty());
    }

    #[test]
    fn parse_request_with_document_data() {
        let doc = b"Hello, printer!";
        let data = build_test_ipp_request(OP_PRINT_JOB, 100, &[], doc);
        let req = parse_ipp_request(&data).expect("parse should succeed");

        assert_eq!(req.operation_id, OP_PRINT_JOB);
        assert_eq!(req.request_id, 100);
        assert_eq!(req.document_data, doc);
    }

    #[test]
    fn parse_request_with_custom_attributes() {
        let attrs = vec![
            (VALUE_TAG_NAME, "job-name", b"Test Print Job" as &[u8]),
            (VALUE_TAG_KEYWORD, "document-format", b"application/pdf"),
        ];
        let data = build_test_ipp_request(OP_PRINT_JOB, 7, &attrs, &[]);
        let req = parse_ipp_request(&data).expect("parse should succeed");

        let op_group = req.operation_attributes().expect("should have op attrs");
        assert_eq!(
            op_group.get_string("job-name").as_deref(),
            Some("Test Print Job")
        );
        assert_eq!(
            op_group.get_string("document-format").as_deref(),
            Some("application/pdf")
        );
    }

    #[test]
    fn parse_request_with_integer_attribute() {
        let job_id_bytes = 42i32.to_be_bytes();
        let attrs = vec![(VALUE_TAG_INTEGER, "job-id", &job_id_bytes[..])];
        let data = build_test_ipp_request(OP_CANCEL_JOB, 5, &attrs, &[]);
        let req = parse_ipp_request(&data).expect("parse should succeed");

        let op_group = req.operation_attributes().expect("should have op attrs");
        assert_eq!(op_group.get_integer("job-id"), Some(42));
    }

    #[test]
    fn parse_rejects_too_short_request() {
        let data = [0x01, 0x01, 0x00]; // only 3 bytes
        let result = parse_ipp_request(&data);
        assert!(result.is_err());
    }

    #[test]
    fn parse_handles_empty_document_data() {
        let data = build_test_ipp_request(OP_VALIDATE_JOB, 1, &[], &[]);
        let req = parse_ipp_request(&data).expect("parse should succeed");
        assert!(req.document_data.is_empty());
    }

    // -- IPP response building ----------------------------------------------

    #[test]
    fn response_builder_creates_valid_header() {
        let resp = IppResponseBuilder::new(STATUS_OK, 99);
        let bytes = resp.build();

        // Minimum: 8 bytes header + 1 byte end-of-attributes = 9 bytes
        assert!(bytes.len() >= 9);
        // version 1.1
        assert_eq!(bytes[0], IPP_VERSION_MAJOR);
        assert_eq!(bytes[1], IPP_VERSION_MINOR);
        // status-code 0x0000
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), STATUS_OK);
        // request-id 99
        assert_eq!(
            u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            99
        );
        // Last byte is end-of-attributes
        assert_eq!(bytes.last().copied().unwrap_or_else(|| panic!("empty bytes")), TAG_END_OF_ATTRIBUTES);
    }

    #[test]
    fn response_builder_roundtrip_with_attributes() {
        let mut builder = IppResponseBuilder::new(STATUS_OK, 42);
        builder
            .begin_group(TAG_OPERATION_ATTRIBUTES)
            .charset("attributes-charset", "utf-8")
            .natural_language("attributes-natural-language", "en")
            .text("status-message", "successful-ok");
        builder
            .begin_group(TAG_JOB_ATTRIBUTES)
            .integer("job-id", 7)
            .enum_attr("job-state", JOB_STATE_PENDING);

        let bytes = builder.build();

        // Parse the response back as if it were a request (same binary format).
        // The status-code field occupies the same position as operation-id.
        let parsed = parse_ipp_request(&bytes).expect("should parse response");

        assert_eq!(parsed.version_major, 1);
        assert_eq!(parsed.version_minor, 1);
        assert_eq!(parsed.operation_id, STATUS_OK); // status-code in response
        assert_eq!(parsed.request_id, 42);
        assert_eq!(parsed.attribute_groups.len(), 2);

        // Operation attributes group
        let op_group = &parsed.attribute_groups[0];
        assert_eq!(op_group.delimiter, TAG_OPERATION_ATTRIBUTES);
        assert_eq!(
            op_group.get_string("attributes-charset").as_deref(),
            Some("utf-8")
        );
        assert_eq!(
            op_group.get_string("status-message").as_deref(),
            Some("successful-ok")
        );

        // Job attributes group
        let job_group = &parsed.attribute_groups[1];
        assert_eq!(job_group.delimiter, TAG_JOB_ATTRIBUTES);
        assert_eq!(job_group.get_integer("job-id"), Some(7));
        assert_eq!(job_group.get_integer("job-state"), Some(JOB_STATE_PENDING));
    }

    #[test]
    fn error_response_has_correct_status() {
        let bytes = build_error_response(STATUS_CLIENT_ERROR_BAD_REQUEST, 10, "bad request");
        let parsed = parse_ipp_request(&bytes).expect("should parse error response");

        assert_eq!(parsed.operation_id, STATUS_CLIENT_ERROR_BAD_REQUEST);
        assert_eq!(parsed.request_id, 10);

        let op_group = parsed.operation_attributes().expect("should have op attrs");
        assert_eq!(
            op_group.get_string("status-message").as_deref(),
            Some("bad request")
        );
    }

    // -- HTTP envelope parsing ----------------------------------------------

    #[test]
    fn parse_http_envelope_finds_body() {
        let http = b"POST /ipp/print HTTP/1.1\r\n\
                     Host: 192.168.1.5:631\r\n\
                     Content-Type: application/ipp\r\n\
                     Content-Length: 42\r\n\
                     \r\n\
                     <ipp body here>";
        let result = parse_http_envelope(http);
        assert!(result.is_some());
        let Some(req) = result else { panic!("parse_http_envelope failed") }; let req = req;
        assert_eq!(req.content_length, Some(42));
        assert!(req.body_offset > 0);
        assert_eq!(&http[req.body_offset..], b"<ipp body here>");
    }

    #[test]
    fn parse_http_envelope_returns_none_for_raw_ipp() {
        // Raw IPP starts with version bytes, not "POST" or "GET".
        let raw_ipp = build_test_ipp_request(OP_GET_PRINTER_ATTRIBUTES, 1, &[], &[]);
        let result = parse_http_envelope(&raw_ipp);
        // Should be None because there is no \r\n\r\n sequence in a well-formed
        // IPP message (the binary data may coincidentally contain it, but the
        // test data here will not).
        assert!(result.is_none());
    }

    // -- MIME type mapping --------------------------------------------------

    #[test]
    fn mime_to_document_type_known_types() {
        assert_eq!(mime_to_document_type("application/pdf"), DocumentType::Pdf);
        assert_eq!(mime_to_document_type("image/jpeg"), DocumentType::Jpeg);
        assert_eq!(mime_to_document_type("image/png"), DocumentType::Png);
        assert_eq!(mime_to_document_type("image/tiff"), DocumentType::Tiff);
        assert_eq!(mime_to_document_type("text/plain"), DocumentType::PlainText);
    }

    #[test]
    fn mime_to_document_type_unknown_falls_back() {
        assert_eq!(
            mime_to_document_type("application/octet-stream"),
            DocumentType::NativeDelegate
        );
        assert_eq!(
            mime_to_document_type("application/postscript"),
            DocumentType::NativeDelegate
        );
    }

    // -- Job status mapping -------------------------------------------------

    #[test]
    fn job_status_to_ipp_state_mapping() {
        assert_eq!(
            job_status_to_ipp_state(JobStatus::Pending),
            JOB_STATE_PENDING
        );
        assert_eq!(job_status_to_ipp_state(JobStatus::Held), JOB_STATE_HELD);
        assert_eq!(
            job_status_to_ipp_state(JobStatus::Processing),
            JOB_STATE_PROCESSING
        );
        assert_eq!(
            job_status_to_ipp_state(JobStatus::Completed),
            JOB_STATE_COMPLETED
        );
        assert_eq!(
            job_status_to_ipp_state(JobStatus::Cancelled),
            JOB_STATE_CANCELED
        );
        assert_eq!(
            job_status_to_ipp_state(JobStatus::Failed),
            JOB_STATE_ABORTED
        );
    }

    // -- Operation dispatch (integration-style) -----------------------------

    /// Create a temporary directory for test document storage.
    fn make_test_data_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().expect("create temp dir for test data")
    }

    fn make_shared_state_with_dir(data_dir: &std::path::Path) -> SharedState {
        let queue = JobQueue::open_in_memory().expect("open in-memory queue");
        let documents_dir = data_dir.join("documents");
        std::fs::create_dir_all(&documents_dir).expect("create documents dir");
        SharedState {
            job_queue: Arc::new(Mutex::new(queue)),
            active_connections: Arc::new(AtomicU32::new(0)),
            port: 9100,
            next_ipp_job_id: Arc::new(AtomicU32::new(1)),
            ipp_to_internal: Arc::new(Mutex::new(HashMap::new())),
            data_dir: data_dir.to_path_buf(),
        }
    }

    fn make_shared_state() -> SharedState {
        let tmp = make_test_data_dir();
        // Leak the TempDir so it lives for the duration of the test.
        // Tests that need the TempDir handle should use make_shared_state_with_dir.
        let path = tmp.path().to_path_buf();
        std::mem::forget(tmp);
        make_shared_state_with_dir(&path)
    }

    #[test]
    fn dispatch_get_printer_attributes_returns_ok() {
        let state = make_shared_state();
        let data = build_test_ipp_request(OP_GET_PRINTER_ATTRIBUTES, 50, &[], &[]);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "127.0.0.1:12345".parse().expect("hardcoded address is invalid");

        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");

        // Status should be successful-ok.
        assert_eq!(parsed.operation_id, STATUS_OK);
        assert_eq!(parsed.request_id, 50);

        // Should have operation-attributes and printer-attributes groups.
        assert!(parsed.attribute_groups.len() >= 2);

        let printer_group = parsed
            .attribute_groups
            .iter()
            .find(|g| g.delimiter == TAG_PRINTER_ATTRIBUTES)
            .expect("should have printer attributes group");

        assert_eq!(
            printer_group.get_string("printer-name").as_deref(),
            Some(PRINTER_NAME)
        );
    }

    #[test]
    fn dispatch_validate_job_returns_ok() {
        let state = make_shared_state();
        let data = build_test_ipp_request(OP_VALIDATE_JOB, 12, &[], &[]);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "127.0.0.1:12345".parse().expect("hardcoded address is invalid");

        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");

        assert_eq!(parsed.operation_id, STATUS_OK);
        assert_eq!(parsed.request_id, 12);
    }

    #[test]
    fn dispatch_print_job_creates_job() {
        let state = make_shared_state();
        let doc = b"%%PDF-1.4 fake pdf content";
        let attrs = vec![
            (VALUE_TAG_NAME, "job-name", b"Test Doc" as &[u8]),
            (VALUE_TAG_KEYWORD, "document-format", b"application/pdf"),
        ];
        let data = build_test_ipp_request(OP_PRINT_JOB, 20, &attrs, doc);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "192.168.1.50:54321".parse().expect("hardcoded address is invalid");

        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");

        // Should succeed.
        assert_eq!(parsed.operation_id, STATUS_OK);
        assert_eq!(parsed.request_id, 20);

        // Should include job attributes with a job-id.
        let job_group = parsed
            .attribute_groups
            .iter()
            .find(|g| g.delimiter == TAG_JOB_ATTRIBUTES)
            .expect("should have job attributes group");

        let ipp_job_id = job_group.get_integer("job-id").expect("should have job-id");
        assert!(ipp_job_id > 0);

        // Verify the job was inserted into the queue.
        let queue = state.job_queue.lock().expect("mutex poisoned");
        let all_jobs = queue.get_all_jobs().expect("get_all_jobs failed");
        assert_eq!(all_jobs.len(), 1);
        assert_eq!(all_jobs[0].document_name, "Test Doc");
    }

    #[test]
    fn dispatch_cancel_job_cancels_job() {
        let state = make_shared_state();

        // First, submit a job.
        let doc = b"some data";
        let data = build_test_ipp_request(OP_PRINT_JOB, 30, &[], doc);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "127.0.0.1:12345".parse().expect("hardcoded address is invalid");
        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");
        let Some(job_group) = parsed
            .attribute_groups
            .iter()
            .find(|g| g.delimiter == TAG_JOB_ATTRIBUTES) else {
            panic!("job attributes group missing from response");
        };
        let ipp_job_id = job_group.get_integer("job-id").expect("job-id attribute missing");

        // Now cancel it.
        let job_id_bytes = ipp_job_id.to_be_bytes();
        let cancel_attrs = vec![(VALUE_TAG_INTEGER, "job-id", &job_id_bytes[..])];
        let cancel_data = build_test_ipp_request(OP_CANCEL_JOB, 31, &cancel_attrs, &[]);
        let cancel_req = parse_ipp_request(&cancel_data).expect("parse_ipp_request failed");

        let cancel_response = dispatch_operation(&cancel_req, peer, &state);
        let cancel_parsed = parse_ipp_request(&cancel_response).expect("parse_ipp_request failed");

        assert_eq!(cancel_parsed.operation_id, STATUS_OK);
        assert_eq!(cancel_parsed.request_id, 31);

        // Verify the job status is now Cancelled.
        let queue = state.job_queue.lock().expect("mutex poisoned");
        let all_jobs = queue.get_all_jobs().expect("get_all_jobs failed");
        assert_eq!(all_jobs.len(), 1);
        assert_eq!(all_jobs[0].status, JobStatus::Cancelled);
    }

    #[test]
    fn dispatch_cancel_nonexistent_job_returns_not_found() {
        let state = make_shared_state();
        let job_id_bytes = 9999i32.to_be_bytes();
        let attrs = vec![(VALUE_TAG_INTEGER, "job-id", &job_id_bytes[..])];
        let data = build_test_ipp_request(OP_CANCEL_JOB, 40, &attrs, &[]);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "127.0.0.1:12345".parse().expect("hardcoded address is invalid");

        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");

        assert_eq!(parsed.operation_id, STATUS_CLIENT_ERROR_NOT_FOUND);
    }

    #[test]
    fn dispatch_get_jobs_returns_empty_list() {
        let state = make_shared_state();
        let data = build_test_ipp_request(OP_GET_JOBS, 60, &[], &[]);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "127.0.0.1:12345".parse().expect("hardcoded address is invalid");

        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");

        assert_eq!(parsed.operation_id, STATUS_OK);
        // Only operation-attributes group, no job groups.
        assert_eq!(parsed.attribute_groups.len(), 1);
    }

    #[test]
    fn dispatch_get_jobs_after_print() {
        let state = make_shared_state();
        let peer: SocketAddr = "127.0.0.1:12345".parse().expect("hardcoded address is invalid");

        // Submit two jobs.
        for i in 0..2 {
            let name_bytes = format!("Job {i}");
            let attrs = vec![(VALUE_TAG_NAME, "job-name", name_bytes.as_bytes())];
            let data = build_test_ipp_request(OP_PRINT_JOB, 100 + i as u32, &attrs, b"data");
            let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
            dispatch_operation(&req, peer, &state);
        }

        // Get-Jobs should return both.
        let data = build_test_ipp_request(OP_GET_JOBS, 200, &[], &[]);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");

        assert_eq!(parsed.operation_id, STATUS_OK);
        // 1 operation-attributes group + 2 job-attributes groups = 3
        let job_groups: Vec<_> = parsed
            .attribute_groups
            .iter()
            .filter(|g| g.delimiter == TAG_JOB_ATTRIBUTES)
            .collect();
        assert_eq!(job_groups.len(), 2);
    }

    #[test]
    fn dispatch_unknown_operation_returns_not_supported() {
        let state = make_shared_state();
        // Use a non-existent operation ID.
        let data = build_test_ipp_request(0x00FF, 70, &[], &[]);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "127.0.0.1:12345".parse().expect("hardcoded address is invalid");

        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");

        assert_eq!(
            parsed.operation_id,
            STATUS_SERVER_ERROR_OPERATION_NOT_SUPPORTED
        );
    }

    #[test]
    fn active_connections_starts_at_zero() {
        let server = IppServer::new(None, None);
        assert_eq!(server.active_connections(), 0);
    }

    // -- find_subsequence ---------------------------------------------------

    #[test]
    fn find_subsequence_basic() {
        assert_eq!(find_subsequence(b"hello world", b"world"), Some(6));
        assert_eq!(find_subsequence(b"hello world", b"hello"), Some(0));
        assert_eq!(find_subsequence(b"hello world", b"xyz"), None);
    }

    #[test]
    fn find_subsequence_crlf() {
        let data = b"Header: value\r\n\r\nBody";
        assert_eq!(find_subsequence(data, b"\r\n\r\n"), Some(13));
    }

    // -- operations-supported encoding (1setOf keyword) ---------------------

    #[test]
    fn keyword_additional_has_zero_name_length() {
        let mut builder = IppResponseBuilder::new(STATUS_OK, 1);
        builder.begin_group(TAG_OPERATION_ATTRIBUTES);
        builder.keyword("test-attr", "first-value");
        builder.keyword_additional("second-value");
        let bytes = builder.build();

        // Parse it back and verify the second value has an empty name.
        let parsed = parse_ipp_request(&bytes).expect("parse_ipp_request failed");
        let group = &parsed.attribute_groups[0];

        // First attribute: "test-attr" with value "first-value"
        let first = &group.attributes[0];
        assert_eq!(first.name, "test-attr");
        assert_eq!(String::from_utf8_lossy(&first.value), "first-value");

        // Second attribute: empty name with value "second-value"
        let second = &group.attributes[1];
        assert_eq!(second.name, "");
        assert_eq!(String::from_utf8_lossy(&second.value), "second-value");
    }

    // -- Document storage ---------------------------------------------------

    #[test]
    fn print_job_persists_document_to_disk() {
        let tmp = make_test_data_dir();
        let state = make_shared_state_with_dir(tmp.path());

        let doc = b"%%PDF-1.4 test document content";
        let attrs = vec![
            (VALUE_TAG_NAME, "job-name", b"Storage Test" as &[u8]),
            (VALUE_TAG_KEYWORD, "document-format", b"application/pdf"),
        ];
        let data = build_test_ipp_request(OP_PRINT_JOB, 200, &attrs, doc);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "10.0.0.1:9999".parse().expect("hardcoded address is invalid");

        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");
        assert_eq!(parsed.operation_id, STATUS_OK);

        // Compute the expected hash.
        let mut hasher = Sha256::new();
        hasher.update(doc);
        let expected_hash = hex::encode(hasher.finalize());

        // Verify the file was written to disk.
        let doc_path = tmp
            .path()
            .join("documents")
            .join(format!("{expected_hash}.dat"));
        assert!(doc_path.exists(), "document file should exist on disk");

        let stored = std::fs::read(&doc_path).expect("read stored document");
        assert_eq!(stored, doc, "stored content should match original");
    }

    #[test]
    fn print_job_skips_write_for_duplicate_hash() {
        let tmp = make_test_data_dir();
        let state = make_shared_state_with_dir(tmp.path());

        let doc = b"identical content for dedup test";
        let peer: SocketAddr = "10.0.0.1:9999".parse().expect("hardcoded address is invalid");

        // Submit the same document data twice (different job names).
        for i in 0..2u32 {
            let name = format!("Dedup Test {i}");
            let attrs = vec![(VALUE_TAG_NAME, "job-name", name.as_bytes())];
            let data = build_test_ipp_request(OP_PRINT_JOB, 300 + i, &attrs, doc);
            let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
            let response = dispatch_operation(&req, peer, &state);
            let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");
            assert_eq!(parsed.operation_id, STATUS_OK);
        }

        // Compute hash and verify a single file exists with correct content.
        let mut hasher = Sha256::new();
        hasher.update(doc);
        let expected_hash = hex::encode(hasher.finalize());

        let doc_path = tmp
            .path()
            .join("documents")
            .join(format!("{expected_hash}.dat"));
        assert!(doc_path.exists());

        let stored = std::fs::read(&doc_path).expect("read stored document");
        assert_eq!(stored, doc);
    }

    #[test]
    fn print_job_empty_document_not_written() {
        let tmp = make_test_data_dir();
        let state = make_shared_state_with_dir(tmp.path());

        // Submit a job with empty document data.
        let data = build_test_ipp_request(OP_PRINT_JOB, 400, &[], &[]);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "10.0.0.1:9999".parse().expect("hardcoded address is invalid");

        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");
        assert_eq!(parsed.operation_id, STATUS_OK);

        // The documents directory should have no files (empty data is hashed
        // as "empty", but the write is skipped for empty payloads).
        let entries: Vec<_> = std::fs::read_dir(tmp.path().join("documents"))
            .expect("read documents dir")
            .collect();
        assert!(
            entries.is_empty(),
            "no document file should be written for empty data"
        );
    }

    #[test]
    fn document_path_returns_expected_location() {
        let tmp = make_test_data_dir();
        let server = IppServer::new(None, Some(tmp.path().to_path_buf()));

        let path = server.document_path("abc123");
        assert_eq!(path, tmp.path().join("documents").join("abc123.dat"));
    }

    #[test]
    fn retrieve_document_reads_stored_file() {
        let tmp = make_test_data_dir();
        let server = IppServer::new(None, Some(tmp.path().to_path_buf()));

        // Manually write a document file.
        let documents_dir = tmp.path().join("documents");
        std::fs::create_dir_all(&documents_dir).expect("filesystem operation failed");
        let content = b"test document bytes";
        std::fs::write(documents_dir.join("deadbeef.dat"), content).expect("filesystem operation failed");

        let retrieved = server
            .retrieve_document("deadbeef")
            .expect("should read document");
        assert_eq!(retrieved, content);
    }

    #[test]
    fn retrieve_document_returns_error_for_missing_file() {
        let tmp = make_test_data_dir();
        let server = IppServer::new(None, Some(tmp.path().to_path_buf()));

        let result = server.retrieve_document("nonexistent");
        assert!(result.is_err(), "should return error for missing document");
    }

    #[test]
    fn retrieve_document_roundtrip_via_print_job() {
        let tmp = make_test_data_dir();
        let state = make_shared_state_with_dir(tmp.path());
        let server = IppServer::new(None, Some(tmp.path().to_path_buf()));

        // Ensure documents dir exists (normally done by start()).
        std::fs::create_dir_all(tmp.path().join("documents")).expect("filesystem operation failed");

        let doc = b"roundtrip content verification payload";
        let attrs = vec![(VALUE_TAG_NAME, "job-name", b"Roundtrip Test" as &[u8])];
        let data = build_test_ipp_request(OP_PRINT_JOB, 500, &attrs, doc);
        let req = parse_ipp_request(&data).expect("parse_ipp_request failed");
        let peer: SocketAddr = "10.0.0.1:9999".parse().expect("hardcoded address is invalid");

        let response = dispatch_operation(&req, peer, &state);
        let parsed = parse_ipp_request(&response).expect("parse_ipp_request failed");
        assert_eq!(parsed.operation_id, STATUS_OK);

        // Compute the hash the same way the server does.
        let mut hasher = Sha256::new();
        hasher.update(doc);
        let hash = hex::encode(hasher.finalize());

        // Retrieve through the public API.
        let retrieved = server
            .retrieve_document(&hash)
            .expect("should retrieve document");
        assert_eq!(retrieved, doc, "retrieved content must match original");
    }
}
