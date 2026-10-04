// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Persistent print job queue backed by SQLite.
//
// The queue stores all print job metadata (but NOT the document bytes) in a
// local SQLite database.  This ensures jobs survive process restarts and
// device reboots.  Document payloads are stored separately on disk and
// referenced by their SHA-256 hash.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use tracing::{debug, info, instrument};

use presswerk_core::error::{PresswerkError, Result};
use presswerk_core::provenance::{FfpClassification, FfpRecord, FormProvenanceLegacy};
use presswerk_core::types::{
    DocumentType, ErrorClass, JobId, JobSource, JobStatus, PrintJob, PrintSettings,
};

/// SQLite schema for the jobs table.
const CREATE_TABLE_SQL: &str = r#"
    CREATE TABLE IF NOT EXISTS jobs (
        id TEXT PRIMARY KEY,
        source TEXT NOT NULL,
        status TEXT NOT NULL,
        document_type TEXT NOT NULL,
        document_name TEXT NOT NULL,
        document_hash TEXT NOT NULL,
        settings TEXT NOT NULL,
        printer_uri TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        error_message TEXT,
        retry_count INTEGER NOT NULL DEFAULT 0,
        max_retries INTEGER NOT NULL DEFAULT 5,
        error_class TEXT,
        error_history TEXT NOT NULL DEFAULT '[]',
        bytes_sent INTEGER NOT NULL DEFAULT 0,
        total_bytes INTEGER NOT NULL DEFAULT 0,
        form_origin TEXT NOT NULL DEFAULT 'Unknown',
        form_provenance TEXT NOT NULL DEFAULT '{}'
    )
"#;

/// Migration to add retry/resume columns to existing databases.
const MIGRATE_RETRY_COLUMNS_SQL: &str = r#"
    ALTER TABLE jobs ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE jobs ADD COLUMN max_retries INTEGER NOT NULL DEFAULT 5;
    ALTER TABLE jobs ADD COLUMN error_class TEXT;
    ALTER TABLE jobs ADD COLUMN error_history TEXT NOT NULL DEFAULT '[]';
    ALTER TABLE jobs ADD COLUMN bytes_sent INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE jobs ADD COLUMN total_bytes INTEGER NOT NULL DEFAULT 0;
"#;

/// Migration to add form-provenance columns to existing databases.
///
/// `form_origin` holds the bare token (`Machine`, `Human`, `Empty`,
/// `NotAForm`, `Unknown`) so routing and audit queries can filter in SQL
/// without parsing JSON; `form_provenance` holds the full determination.
/// The `'{}'` default deliberately does not parse into a `FormProvenance`,
/// so pre-existing rows surface as "not inspected" rather than as a guess.
const MIGRATE_FORM_PROVENANCE_SQL: &str = r#"
    ALTER TABLE jobs ADD COLUMN form_origin TEXT NOT NULL DEFAULT 'Unknown';
    ALTER TABLE jobs ADD COLUMN form_provenance TEXT NOT NULL DEFAULT '{}';
"#;

/// Persistent job queue backed by a SQLite database.
///
/// All methods are synchronous because `rusqlite` does not support async
/// natively.  In an async context, wrap calls in `tokio::task::spawn_blocking`.
pub struct JobQueue {
    /// The open SQLite connection.
    conn: Connection,
}

impl JobQueue {
    /// Open (or create) the job queue database at the given path.
    ///
    /// Applies WAL journal mode for better concurrent-read performance on
    /// mobile devices and creates the `jobs` table if it does not exist.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref())
            .map_err(|e| PresswerkError::Database(format!("open: {e}")))?;

        // WAL mode is better for concurrent readers (UI thread + background
        // sync) and survives unclean shutdowns more gracefully.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| PresswerkError::Database(format!("WAL pragma: {e}")))?;

        conn.execute_batch(CREATE_TABLE_SQL)
            .map_err(|e| PresswerkError::Database(format!("create table: {e}")))?;

        // Run migration for existing databases that lack retry columns.
        Self::migrate_retry_columns(&conn);

        // Run migration for existing databases that lack provenance columns.
        Self::migrate_form_provenance_columns(&conn);

        info!("job queue database opened");
        Ok(Self { conn })
    }

    /// Open an in-memory database (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| PresswerkError::Database(format!("open in-memory: {e}")))?;

        conn.execute_batch(CREATE_TABLE_SQL)
            .map_err(|e| PresswerkError::Database(format!("create table: {e}")))?;

        debug!("in-memory job queue database opened");
        Ok(Self { conn })
    }

    /// Apply retry/resume column migration to existing databases.
    /// Silently skips if columns already exist.
    fn migrate_retry_columns(conn: &Connection) {
        // Each ALTER TABLE is run individually — if the column exists the
        // statement fails harmlessly and we continue to the next.
        for stmt in MIGRATE_RETRY_COLUMNS_SQL.split(';') {
            let trimmed = stmt.trim();
            if trimmed.is_empty() {
                continue;
            }
            if conn.execute_batch(trimmed).is_err() {
                // Column already exists — expected on migrated databases.
            }
        }
    }

    /// Apply form-provenance column migration to existing databases.
    /// Silently skips if columns already exist.
    fn migrate_form_provenance_columns(conn: &Connection) {
        for stmt in MIGRATE_FORM_PROVENANCE_SQL.split(';') {
            let trimmed = stmt.trim();
            if trimmed.is_empty() {
                continue;
            }
            if conn.execute_batch(trimmed).is_err() {
                // Column already exists — expected on migrated databases.
            }
        }
    }

    /// Insert a new print job into the queue.
    ///
    /// The job's `id`, `created_at`, and `updated_at` fields must already be
    /// populated (they are set by `PrintJob::new`).
    #[instrument(skip(self, job), fields(job_id = %job.id))]
    pub fn insert_job(&self, job: &PrintJob) -> Result<()> {
        let source_json = serde_json::to_string(&job.source)
            .map_err(|e| PresswerkError::Database(format!("serialize source: {e}")))?;
        let status_json = serde_json::to_string(&job.status)
            .map_err(|e| PresswerkError::Database(format!("serialize status: {e}")))?;
        let doc_type_json = serde_json::to_string(&job.document_type)
            .map_err(|e| PresswerkError::Database(format!("serialize document_type: {e}")))?;
        let settings_json = serde_json::to_string(&job.settings)
            .map_err(|e| PresswerkError::Database(format!("serialize settings: {e}")))?;

        let error_class_json = job
            .error_class
            .as_ref()
            .map(|ec| serde_json::to_string(ec).unwrap_or_default());
        let error_history_json = serde_json::to_string(&job.error_history)
            .map_err(|e| PresswerkError::Database(format!("serialize error_history: {e}")))?;
        let form_provenance_json = job.form_provenance.audit_json();
        let form_origin = job.form_provenance.classification.as_str();

        self.conn
            .execute(
                "INSERT INTO jobs (id, source, status, document_type, document_name,
                 document_hash, settings, printer_uri, created_at, updated_at, error_message,
                 retry_count, max_retries, error_class, error_history, bytes_sent, total_bytes,
                 form_origin, form_provenance)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18, ?19)",
                params![
                    job.id.to_string(),
                    source_json,
                    status_json,
                    doc_type_json,
                    job.document_name,
                    job.document_hash,
                    settings_json,
                    job.printer_uri,
                    job.created_at.to_rfc3339(),
                    job.updated_at.to_rfc3339(),
                    job.error_message,
                    job.retry_count,
                    job.max_retries,
                    error_class_json,
                    error_history_json,
                    job.bytes_sent as i64,
                    job.total_bytes as i64,
                    form_origin,
                    form_provenance_json,
                ],
            )
            .map_err(|e| PresswerkError::Database(format!("insert job: {e}")))?;

        info!(job_id = %job.id, "job inserted into queue");
        Ok(())
    }

    /// Update the status (and optionally the error message) of an existing job.
    ///
    /// Also bumps `updated_at` to the current time.
    #[instrument(skip(self), fields(job_id = %job_id))]
    pub fn update_status(
        &self,
        job_id: &JobId,
        status: JobStatus,
        error_message: Option<&str>,
    ) -> Result<()> {
        let status_json = serde_json::to_string(&status)
            .map_err(|e| PresswerkError::Database(format!("serialize status: {e}")))?;
        let now = Utc::now().to_rfc3339();

        let rows = self
            .conn
            .execute(
                "UPDATE jobs SET status = ?1, updated_at = ?2, error_message = ?3
                 WHERE id = ?4",
                params![status_json, now, error_message, job_id.to_string()],
            )
            .map_err(|e| PresswerkError::Database(format!("update status: {e}")))?;

        if rows == 0 {
            return Err(PresswerkError::Database(format!("job {job_id} not found")));
        }

        debug!(job_id = %job_id, status = ?status, "job status updated");
        Ok(())
    }

    /// Retrieve a single job by its ID.
    ///
    /// Returns `None` if the job does not exist.
    #[instrument(skip(self), fields(job_id = %job_id))]
    pub fn get_job(&self, job_id: &JobId) -> Result<Option<PrintJob>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, source, status, document_type, document_name,
                        document_hash, settings, printer_uri, created_at,
                        updated_at, error_message, retry_count, max_retries,
                        error_class, error_history, bytes_sent, total_bytes,
                        form_origin, form_provenance
                 FROM jobs WHERE id = ?1",
            )
            .map_err(|e| PresswerkError::Database(format!("prepare get_job: {e}")))?;

        let mut rows = stmt
            .query_map(params![job_id.to_string()], row_to_print_job)
            .map_err(|e| PresswerkError::Database(format!("query get_job: {e}")))?;

        match rows.next() {
            Some(Ok(job)) => Ok(Some(job)),
            Some(Err(e)) => Err(PresswerkError::Database(format!("row parse: {e}"))),
            None => Ok(None),
        }
    }

    /// Retrieve all jobs, ordered by creation time (newest first).
    #[instrument(skip(self))]
    pub fn get_all_jobs(&self) -> Result<Vec<PrintJob>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, source, status, document_type, document_name,
                        document_hash, settings, printer_uri, created_at,
                        updated_at, error_message, retry_count, max_retries,
                        error_class, error_history, bytes_sent, total_bytes,
                        form_origin, form_provenance
                 FROM jobs ORDER BY created_at DESC",
            )
            .map_err(|e| PresswerkError::Database(format!("prepare get_all_jobs: {e}")))?;

        let jobs = stmt
            .query_map([], row_to_print_job)
            .map_err(|e| PresswerkError::Database(format!("query get_all_jobs: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| PresswerkError::Database(format!("collect rows: {e}")))?;

        debug!(count = jobs.len(), "retrieved all jobs");
        Ok(jobs)
    }

    /// Retrieve all jobs with `Pending` status, ordered by creation time
    /// (oldest first, i.e. FIFO).
    #[instrument(skip(self))]
    pub fn get_pending_jobs(&self) -> Result<Vec<PrintJob>> {
        let pending_json = serde_json::to_string(&JobStatus::Pending)
            .map_err(|e| PresswerkError::Database(format!("serialize Pending: {e}")))?;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, source, status, document_type, document_name,
                        document_hash, settings, printer_uri, created_at,
                        updated_at, error_message, retry_count, max_retries,
                        error_class, error_history, bytes_sent, total_bytes,
                        form_origin, form_provenance
                 FROM jobs WHERE status = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| PresswerkError::Database(format!("prepare get_pending: {e}")))?;

        let jobs = stmt
            .query_map(params![pending_json], row_to_print_job)
            .map_err(|e| PresswerkError::Database(format!("query get_pending: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| PresswerkError::Database(format!("collect rows: {e}")))?;

        debug!(count = jobs.len(), "retrieved pending jobs");
        Ok(jobs)
    }

    /// Retrieve every job whose document was classified with the given form
    /// provenance, newest first.
    ///
    /// This is the query audit and routing use to answer "which of these print
    /// jobs were machine-filled forms?" (issue #118, ruling D189). Filtering
    /// happens on the `form_origin` token column, so it never has to parse the
    /// JSON determination.
    #[instrument(skip(self), fields(origin = %origin))]
    pub fn get_jobs_with_form_origin(&self, origin: FfpClassification) -> Result<Vec<PrintJob>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, source, status, document_type, document_name,
                        document_hash, settings, printer_uri, created_at,
                        updated_at, error_message, retry_count, max_retries,
                        error_class, error_history, bytes_sent, total_bytes,
                        form_origin, form_provenance
                 FROM jobs WHERE form_origin = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| PresswerkError::Database(format!("prepare get_by_origin: {e}")))?;

        let jobs = stmt
            .query_map(params![origin.as_str()], row_to_print_job)
            .map_err(|e| PresswerkError::Database(format!("query get_by_origin: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| PresswerkError::Database(format!("collect rows: {e}")))?;

        debug!(count = jobs.len(), origin = %origin, "retrieved jobs by form origin");
        Ok(jobs)
    }

    /// Delete a job from the queue.
    ///
    /// Returns `Ok(())` even if the job did not exist (idempotent).
    #[instrument(skip(self), fields(job_id = %job_id))]
    pub fn delete_job(&self, job_id: &JobId) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM jobs WHERE id = ?1",
                params![job_id.to_string()],
            )
            .map_err(|e| PresswerkError::Database(format!("delete job: {e}")))?;

        info!(job_id = %job_id, "job deleted from queue");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Row mapping
// ---------------------------------------------------------------------------

/// Map a SQLite row to a `PrintJob`.
///
/// Column indices must match the SELECT order used in the query methods above.
fn row_to_print_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<PrintJob> {
    let id_str: String = row.get(0)?;
    let source_json: String = row.get(1)?;
    let status_json: String = row.get(2)?;
    let doc_type_json: String = row.get(3)?;
    let document_name: String = row.get(4)?;
    let document_hash: String = row.get(5)?;
    let settings_json: String = row.get(6)?;
    let printer_uri: Option<String> = row.get(7)?;
    let created_at_str: String = row.get(8)?;
    let updated_at_str: String = row.get(9)?;
    let error_message: Option<String> = row.get(10)?;
    let retry_count: u32 = row.get::<_, i32>(11).unwrap_or(0) as u32;
    let max_retries: u32 = row.get::<_, i32>(12).unwrap_or(5) as u32;
    let error_class_json: Option<String> = row.get(13).unwrap_or(None);
    let error_history_json: String = row.get::<_, String>(14).unwrap_or_else(|_| "[]".into());
    let bytes_sent: u64 = row.get::<_, i64>(15).unwrap_or(0) as u64;
    let total_bytes: u64 = row.get::<_, i64>(16).unwrap_or(0) as u64;
    // Columns 17 and 18 were added by migration. On a pre-migration row they
    // are absent; on a row written before provenance existed they carry the
    // schema default (`'Unknown'` / `'{}'`), and `'{}'` does not parse into a
    // `FormProvenance`. Every one of those cases must surface as "not
    // inspected" — never as a guessed determination.
    let form_origin_token: Option<String> = row.get(17).unwrap_or(None);
    let form_provenance_json: Option<String> = row.get(18).unwrap_or(None);
    // FFP v1.0 record; fallback to legacy JSON, then to origin token, then to unreadable.
    let form_provenance: FfpRecord = form_provenance_json
        .as_deref()
        .and_then(|json| {
            // Try new shape
            if let Ok(rec) = serde_json::from_str::<FfpRecord>(json) {
                // `ffp` must be "1.0" or missing defaults to 1.0; guard against empty `{}`
                if rec.ffp == "1.0" || rec.ffp.is_empty() {
                    // Empty `{}` parsed as default? Ensure classification is not default unreadable due to empty
                    // If json was "{}", rec will be unreadable with UNREADABLE evidence — that's correct for legacy default.
                    return Some(rec);
                }
                return Some(rec);
            }
            // Try legacy shape
            if let Ok(legacy) = serde_json::from_str::<FormProvenanceLegacy>(json) {
                return Some(FfpRecord::from(legacy));
            }
            None
        })
        .unwrap_or_else(|| {
            // No JSON — infer from legacy origin token if present
            if let Some(tok) = form_origin_token.as_deref() {
                let cls = FfpClassification::from_str(tok);
                // Handle legacy tokens that are not kebab-case
                let mapped = match tok {
                    "NotAForm" => FfpClassification::NoForm,
                    "Empty" => FfpClassification::BlankForm,
                    "Human" => FfpClassification::FilledUnknown,
                    "Machine" => FfpClassification::MachineFilledSuspected,
                    "Unknown" => FfpClassification::Unreadable,
                    _ => cls,
                };
                match mapped {
                    FfpClassification::NoForm => {
                        FfpRecord::no_form(vec!["FFP-E-NO-ACROFORM".to_string()])
                    }
                    FfpClassification::BlankForm => FfpRecord {
                        ffp: "1.0".to_string(),
                        classification: FfpClassification::BlankForm,
                        form: presswerk_core::provenance::FfpForm::Present,
                        filled_fields: 0,
                        total_fields: 0,
                        appearances: presswerk_core::provenance::FfpAppearances::NotApplicable,
                        declared: None,
                        evidence: vec!["FFP-E-NO-VALUES".to_string()],
                    },
                    FfpClassification::Unreadable => {
                        FfpRecord::unreadable("legacy row without provenance")
                    }
                    _ => FfpRecord {
                        ffp: "1.0".to_string(),
                        classification: mapped,
                        form: presswerk_core::provenance::FfpForm::Unknown,
                        filled_fields: 0,
                        total_fields: 0,
                        appearances: presswerk_core::provenance::FfpAppearances::Unknown,
                        declared: None,
                        evidence: vec!["FFP-E-UNREADABLE".to_string()],
                    },
                }
            } else {
                FfpRecord::unreadable("pre-migration row without provenance columns")
            }
        });

    // Parse the UUID.  If the stored value is malformed we surface a
    // meaningful error rather than panicking.
    let uuid = uuid::Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;

    let source: JobSource = serde_json::from_str(&source_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;

    let status: JobStatus = serde_json::from_str(&status_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;

    let document_type: DocumentType = serde_json::from_str(&doc_type_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;

    let settings: PrintSettings = serde_json::from_str(&settings_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
    })?;

    let created_at: DateTime<Utc> = DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
        })?;

    let updated_at: DateTime<Utc> = DateTime::parse_from_rfc3339(&updated_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
        })?;

    let error_class: Option<ErrorClass> =
        error_class_json.and_then(|s| serde_json::from_str(&s).ok());

    let error_history: Vec<String> = serde_json::from_str(&error_history_json).unwrap_or_default();

    Ok(PrintJob {
        id: JobId(uuid),
        source,
        status,
        document_type,
        document_name,
        document_hash,
        settings,
        printer_uri,
        created_at,
        updated_at,
        error_message,
        retry_count,
        max_retries,
        error_class,
        error_history,
        bytes_sent,
        total_bytes,
        form_provenance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use presswerk_core::types::JobSource;

    /// Helper: create a minimal test job.
    fn test_job() -> PrintJob {
        PrintJob::new(
            JobSource::Local,
            DocumentType::Pdf,
            "test-document.pdf".into(),
            "abc123def456".into(),
        )
    }

    #[test]
    fn insert_and_retrieve_job() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");
        let job = test_job();
        queue.insert_job(&job).expect("insert");

        let retrieved = queue.get_job(&job.id).expect("get_job").expect("found");
        assert_eq!(retrieved.id, job.id);
        assert_eq!(retrieved.document_name, "test-document.pdf");
        assert_eq!(retrieved.document_hash, "abc123def456");
    }

    #[test]
    fn update_status() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");
        let job = test_job();
        queue.insert_job(&job).expect("insert");

        queue
            .update_status(&job.id, JobStatus::Processing, None)
            .expect("update");

        let updated = queue.get_job(&job.id).expect("get_job").expect("found");
        assert_eq!(updated.status, JobStatus::Processing);
        assert!(updated.error_message.is_none());
    }

    #[test]
    fn update_status_with_error() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");
        let job = test_job();
        queue.insert_job(&job).expect("insert");

        queue
            .update_status(&job.id, JobStatus::Failed, Some("paper jam"))
            .expect("update");

        let updated = queue.get_job(&job.id).expect("get_job").expect("found");
        assert_eq!(updated.status, JobStatus::Failed);
        assert_eq!(updated.error_message.as_deref(), Some("paper jam"));
    }

    #[test]
    fn get_all_jobs_returns_newest_first() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");

        let job1 = test_job();
        let job2 = test_job();
        queue.insert_job(&job1).expect("insert 1");
        queue.insert_job(&job2).expect("insert 2");

        let all = queue.get_all_jobs().expect("get_all");
        assert_eq!(all.len(), 2);
        // Newest first — job2 was created after job1.
        assert!(all[0].created_at >= all[1].created_at);
    }

    #[test]
    fn get_pending_jobs_filters_correctly() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");

        let job1 = test_job();
        let job2 = test_job();
        queue.insert_job(&job1).expect("insert 1");
        queue.insert_job(&job2).expect("insert 2");

        // Mark job1 as completed.
        queue
            .update_status(&job1.id, JobStatus::Completed, None)
            .expect("update");

        let pending = queue.get_pending_jobs().expect("get_pending");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, job2.id);
    }

    #[test]
    fn delete_job_is_idempotent() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");
        let job = test_job();
        queue.insert_job(&job).expect("insert");

        queue.delete_job(&job.id).expect("delete first time");
        queue
            .delete_job(&job.id)
            .expect("delete second time (idempotent)");

        let result = queue.get_job(&job.id).expect("get_job");
        assert!(result.is_none());
    }

    #[test]
    fn get_nonexistent_job_returns_none() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");
        let result = queue.get_job(&JobId::new()).expect("get_job");
        assert!(result.is_none());
    }

    #[test]
    fn update_nonexistent_job_returns_error() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");
        let result = queue.update_status(&JobId::new(), JobStatus::Cancelled, None);
        assert!(result.is_err());
    }

    // -- Form provenance (issue #118, ruling D189) --------------------------

    /// A machine-filled determination, as the document crate would produce it (FFP v1.0).
    fn machine_filled_job() -> PrintJob {
        let mut job = test_job();
        job.form_provenance = FfpRecord {
            ffp: "1.0".to_string(),
            classification: FfpClassification::MachineFilled,
            form: presswerk_core::provenance::FfpForm::Present,
            filled_fields: 12,
            total_fields: 14,
            appearances: presswerk_core::provenance::FfpAppearances::Incomplete,
            declared: Some(presswerk_core::provenance::FfpDeclared {
                filled_by: "machine".to_string(),
                tool: Some("blocky-writer 0.4.2".to_string()),
                tool_version: Some("0.4.2".to_string()),
                filled_at: None,
                appearances_generated: Some(false),
            }),
            evidence: vec![
                "FFP-E-DECL-MACHINE".to_string(),
                "FFP-E-NEED-APPEARANCES".to_string(),
                "FFP-E-AP-INCOMPLETE".to_string(),
            ],
        };
        job
    }

    #[test]
    fn provenance_roundtrips_through_the_queue() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");
        let job = machine_filled_job();
        queue.insert_job(&job).expect("insert");

        let retrieved = queue.get_job(&job.id).expect("get_job").expect("found");
        assert_eq!(retrieved.form_provenance, job.form_provenance);
        assert!(retrieved.form_provenance.is_machine_filled());
        assert_eq!(
            retrieved
                .form_provenance
                .declared
                .as_ref()
                .and_then(|d| d.tool.as_deref()),
            Some("blocky-writer 0.4.2")
        );
        // canonical line preserved
        assert!(
            retrieved
                .form_provenance
                .canonical_line()
                .contains("machine-filled")
        );
    }

    #[test]
    fn jobs_can_be_filtered_by_form_origin() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");

        let machine = machine_filled_job();
        let plain = test_job();
        queue.insert_job(&machine).expect("insert machine");
        queue.insert_job(&plain).expect("insert plain");

        let machine_jobs = queue
            .get_jobs_with_form_origin(FfpClassification::MachineFilled)
            .expect("query machine");
        assert_eq!(machine_jobs.len(), 1);
        assert_eq!(machine_jobs[0].id, machine.id);

        let unreadable_jobs = queue
            .get_jobs_with_form_origin(FfpClassification::Unreadable)
            .expect("query unreadable");
        assert_eq!(unreadable_jobs.len(), 1);
        assert_eq!(unreadable_jobs[0].id, plain.id);

        assert!(
            queue
                .get_jobs_with_form_origin(FfpClassification::FilledUnknown)
                .expect("query human")
                .is_empty()
        );
    }

    #[test]
    fn every_read_path_returns_the_stored_provenance() {
        let queue = JobQueue::open_in_memory().expect("open in-memory db");
        let job = machine_filled_job();
        queue.insert_job(&job).expect("insert");

        assert!(
            queue.get_all_jobs().expect("get_all")[0]
                .form_provenance
                .is_machine_filled()
        );
        assert!(
            queue.get_pending_jobs().expect("get_pending")[0]
                .form_provenance
                .is_machine_filled()
        );
    }

    /// The `jobs` table as it stood before form provenance existed.
    const LEGACY_SCHEMA: &str = r#"
        CREATE TABLE jobs (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            status TEXT NOT NULL,
            document_type TEXT NOT NULL,
            document_name TEXT NOT NULL,
            document_hash TEXT NOT NULL,
            settings TEXT NOT NULL,
            printer_uri TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            error_message TEXT,
            retry_count INTEGER NOT NULL DEFAULT 0,
            max_retries INTEGER NOT NULL DEFAULT 5,
            error_class TEXT,
            error_history TEXT NOT NULL DEFAULT '[]',
            bytes_sent INTEGER NOT NULL DEFAULT 0,
            total_bytes INTEGER NOT NULL DEFAULT 0
        );
    "#;

    #[test]
    fn legacy_database_is_migrated_and_reads_as_uninspected() {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let path = dir.path().join("legacy-jobs.db");

        // Write a row using the pre-provenance schema.
        {
            let conn = rusqlite::Connection::open(&path).expect("open legacy db");
            conn.execute_batch(LEGACY_SCHEMA)
                .expect("create legacy table");
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO jobs (id, source, status, document_type, document_name,
                 document_hash, settings, printer_uri, created_at, updated_at, error_message,
                 retry_count, max_retries, error_class, error_history, bytes_sent, total_bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    "11111111-1111-1111-1111-111111111111",
                    "\"Local\"",
                    "\"Pending\"",
                    "\"Pdf\"",
                    "legacy-form.pdf",
                    "legacyhash",
                    serde_json::to_string(&PrintSettings::default()).unwrap(),
                    Option::<String>::None,
                    now,
                    now,
                    Option::<String>::None,
                    0,
                    5,
                    Option::<String>::None,
                    "[]",
                    0,
                    0,
                ],
            )
            .expect("insert legacy row");
        }

        // Reopening must migrate the schema without losing the row.
        let queue = JobQueue::open(&path).expect("open migrated db");
        let jobs = queue.get_all_jobs().expect("get_all_jobs");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].document_name, "legacy-form.pdf");

        // A row with no provenance must read back as unreadable, never as a guessed determination.
        let provenance = &jobs[0].form_provenance;
        assert_eq!(provenance.classification, FfpClassification::Unreadable);
        assert_eq!(
            provenance.form,
            presswerk_core::provenance::FfpForm::Unknown
        );
        assert!(!provenance.is_machine_filled());

        // And new jobs written after migration carry their determination.
        let fresh = machine_filled_job();
        queue.insert_job(&fresh).expect("insert post-migration job");
        let machine_jobs = queue
            .get_jobs_with_form_origin(FfpClassification::MachineFilled)
            .expect("query machine");
        assert_eq!(machine_jobs.len(), 1);
        assert_eq!(machine_jobs[0].id, fresh.id);
    }
}
