// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Audit trail — append-only SQLite log of every security-relevant operation.
//
// Schema:
//   audit_log(
//     id            INTEGER PRIMARY KEY AUTOINCREMENT,
//     timestamp     TEXT    NOT NULL,   -- RFC 3339
//     action        TEXT    NOT NULL,   -- e.g. "encrypt", "decrypt", "print"
//     document_hash TEXT    NOT NULL,   -- SHA-256 hex digest
//     success       INTEGER NOT NULL,   -- 0 = failure, 1 = success
//     details       TEXT                -- optional free-form context
//   )

use std::path::Path;

use chrono::Utc;
use presswerk_core::error::PresswerkError;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

// ---------------------------------------------------------------------------
// Local error helpers
// ---------------------------------------------------------------------------

/// Convert a `rusqlite::Error` into a `PresswerkError::Database`.
fn db_err(e: rusqlite::Error) -> PresswerkError {
    PresswerkError::Database(e.to_string())
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single entry in the audit log, used for queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub timestamp: String,
    pub action: String,
    pub document_hash: String,
    pub success: bool,
    pub details: Option<String>,
}

/// Append-only audit log backed by a SQLite database.
///
/// Every security-relevant operation (encrypt, decrypt, print, integrity
/// check, certificate generation, ...) is recorded with a timestamp, action
/// type, the SHA-256 hash of the document involved, and a success/failure
/// flag.
pub struct AuditLog {
    conn: Connection,
}

impl AuditLog {
    /// Open (or create) the audit database at `path`.
    ///
    /// The `audit_log` table is created automatically if it does not already
    /// exist.  WAL mode is enabled for better concurrent-read performance.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PresswerkError> {
        let conn = Connection::open(path).map_err(db_err)?;

        // Enable WAL for concurrent readers.
        conn.execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(db_err)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp     TEXT    NOT NULL,
                action        TEXT    NOT NULL,
                document_hash TEXT    NOT NULL,
                success       INTEGER NOT NULL,
                details       TEXT
            );",
        )
        .map_err(db_err)?;

        debug!("audit log opened");
        Ok(Self { conn })
    }

    /// Open an in-memory audit database (useful for tests).
    pub fn open_in_memory() -> Result<Self, PresswerkError> {
        let conn = Connection::open_in_memory().map_err(db_err)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp     TEXT    NOT NULL,
                action        TEXT    NOT NULL,
                document_hash TEXT    NOT NULL,
                success       INTEGER NOT NULL,
                details       TEXT
            );",
        )
        .map_err(db_err)?;

        debug!("in-memory audit log opened");
        Ok(Self { conn })
    }

    /// Record a new audit entry.
    ///
    /// `action` is a short verb describing the operation (e.g. `"encrypt"`,
    /// `"decrypt"`, `"print"`).  `document_hash` should be the SHA-256 hex
    /// digest of the document bytes involved.
    #[instrument(skip(self, details), fields(%action, %document_hash, success))]
    pub fn record(
        &self,
        action: &str,
        document_hash: &str,
        success: bool,
        details: Option<&str>,
    ) -> Result<(), PresswerkError> {
        let timestamp = Utc::now().to_rfc3339();
        let success_int: i32 = if success { 1 } else { 0 };

        self.conn
            .execute(
                "INSERT INTO audit_log (timestamp, action, document_hash, success, details)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![timestamp, action, document_hash, success_int, details],
            )
            .map_err(db_err)?;

        debug!("audit entry recorded");
        Ok(())
    }

    /// Retrieve all entries for a given document hash, ordered by timestamp
    /// ascending.
    pub fn entries_for_hash(&self, document_hash: &str) -> Result<Vec<AuditEntry>, PresswerkError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, timestamp, action, document_hash, success, details
                 FROM audit_log
                 WHERE document_hash = ?1
                 ORDER BY timestamp ASC",
            )
            .map_err(db_err)?;

        let rows = stmt
            .query_map(params![document_hash], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    action: row.get(2)?,
                    document_hash: row.get(3)?,
                    success: row.get::<_, i32>(4)? != 0,
                    details: row.get(5)?,
                })
            })
            .map_err(db_err)?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(db_err)?);
        }
        Ok(entries)
    }

    /// Retrieve the most recent `limit` entries, ordered newest-first.
    pub fn recent_entries(&self, limit: u32) -> Result<Vec<AuditEntry>, PresswerkError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, timestamp, action, document_hash, success, details
                 FROM audit_log
                 ORDER BY id DESC
                 LIMIT ?1",
            )
            .map_err(db_err)?;

        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    action: row.get(2)?,
                    document_hash: row.get(3)?,
                    success: row.get::<_, i32>(4)? != 0,
                    details: row.get(5)?,
                })
            })
            .map_err(db_err)?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(db_err)?);
        }
        Ok(entries)
    }

    /// Return the total number of entries in the audit log.
    pub fn count(&self) -> Result<u64, PresswerkError> {
        self.conn
            .query_row("SELECT COUNT(*) FROM audit_log", [], |row| row.get(0))
            .map_err(db_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_log() -> AuditLog {
        AuditLog::open_in_memory().expect("open in-memory audit log")
    }

    #[test]
    fn record_and_count() {
        let log = make_log();
        assert_eq!(log.count().unwrap(), 0);

        log.record("encrypt", "abc123", true, None).unwrap();
        log.record("decrypt", "abc123", true, Some("round-trip test"))
            .unwrap();

        assert_eq!(log.count().unwrap(), 2);
    }

    #[test]
    fn entries_for_hash() {
        let log = make_log();
        log.record("encrypt", "aaa", true, None).unwrap();
        log.record("print", "bbb", true, None).unwrap();
        log.record("decrypt", "aaa", false, Some("wrong key"))
            .unwrap();

        let entries = log.entries_for_hash("aaa").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].action, "encrypt");
        assert!(entries[0].success);
        assert_eq!(entries[1].action, "decrypt");
        assert!(!entries[1].success);
    }

    #[test]
    fn recent_entries_ordering() {
        let log = make_log();
        for i in 0..5 {
            log.record("op", &format!("hash_{i}"), true, None).unwrap();
        }

        let recent = log.recent_entries(3).unwrap();
        assert_eq!(recent.len(), 3);
        // Newest first — IDs should be descending.
        assert!(recent[0].id > recent[1].id);
        assert!(recent[1].id > recent[2].id);
    }

    #[test]
    fn failure_entry() {
        let log = make_log();
        log.record("decrypt", "deadbeef", false, Some("bad passphrase"))
            .unwrap();

        let entries = log.entries_for_hash("deadbeef").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].success);
        assert_eq!(entries[0].details.as_deref(), Some("bad passphrase"));
    }
}
