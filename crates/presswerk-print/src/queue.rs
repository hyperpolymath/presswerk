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
        total_bytes INTEGER NOT NULL DEFAULT 0
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

        self.conn
            .execute(
                "INSERT INTO jobs (id, source, status, document_type, document_name,
                 document_hash, settings, printer_uri, created_at, updated_at, error_message,
                 retry_count, max_retries, error_class, error_history, bytes_sent, total_bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
                        error_class, error_history, bytes_sent, total_bytes
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
                        error_class, error_history, bytes_sent, total_bytes
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
                        error_class, error_history, bytes_sent, total_bytes
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
}
