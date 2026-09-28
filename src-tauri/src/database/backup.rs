//! Database backup and recovery
//!
//! Provides database snapshot backup and recovery.

use super::{lock_conn, Database};
use crate::config::get_app_config_dir;
use crate::error::AppError;
use chrono::{Local, Utc};
use rusqlite::backup::{Backup, StepResult};
#[cfg(test)]
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tempfile::{Builder, NamedTempFile};

const COPILOT_BRIDGE_ATLAS_SQL_EXPORT_HEADER: &str = "-- Copilot Bridge Atlas SQLite export";

/// Bound combined INSERT batches while still amortizing statement parsing.
/// A row larger than this cap is emitted alone because it cannot be split.
#[cfg(test)]
const INSERT_BATCH_MAX_ROWS: usize = 200;
#[cfg(test)]
const INSERT_BATCH_MAX_BYTES: usize = 1024 * 1024;

/// Serialize every operation that observes or mutates the database-backup
/// directory. Always acquire this guard before `Database.conn`.
static BACKUP_FILE_OPERATION_LOCK: Mutex<()> = Mutex::new(());
type BackupFileOperationGuard = MutexGuard<'static, ()>;

fn lock_backup_file_operations() -> Result<BackupFileOperationGuard, AppError> {
    BACKUP_FILE_OPERATION_LOCK
        .lock()
        .map_err(|e| AppError::Database(format!("Backup file operation lock failed: {e}")))
}

/// Accept only the PRAGMAs emitted by `dump_sql`. Other PRAGMAs can redirect
/// temporary files or bypass schema integrity checks.
const IMPORT_ALLOWED_PRAGMAS: &[&str] = &["foreign_keys", "user_version"];

/// Reject imported SQL operations that could affect files outside the staging database.
///
/// The export header is only a prefix check. An attacker can append statements
/// after it, and `ATTACH DATABASE` can create a file before schema validation
/// rejects the import. Apply this authorizer while staging external SQL.
///
/// Text scanning misses commented, mixed-case, or multiline `ATTACH` and
/// `VACUUM INTO`. The SQLite authorizer sees the parsed operation at prepare time.
///
/// The imported SQL already controls the contents of this disposable database.
/// Blocking `DELETE`, `DROP`, or `UPDATE` would not add protection, but it could
/// reject a valid backup with an unfamiliar object. The file boundary matters.
///
/// SQLite reports `ATTACH`, `VACUUM INTO`, and bare `VACUUM` as `Attach`.
/// File-backed virtual tables can read or write arbitrary paths. Reject unknown
/// action codes so future cross-file operations remain blocked by default.
fn import_authorizer(context: rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization {
    use rusqlite::hooks::{AuthAction, Authorization};

    let escapes_temp_db = match context.action {
        AuthAction::Attach { .. } | AuthAction::Detach { .. } => true,
        AuthAction::CreateVtable { .. } | AuthAction::DropVtable { .. } => true,
        AuthAction::Unknown { .. } => true,
        AuthAction::Pragma { pragma_name, .. } => !IMPORT_ALLOWED_PRAGMAS
            .iter()
            .any(|allowed| pragma_name.eq_ignore_ascii_case(allowed)),
        _ => false,
    };

    if escapes_temp_db {
        // SQLite will only reply "not authorized". Without logging, there is no way to know which statement was blocked.
        log::warn!(
            "SQL import rejected out-of-bounds statement: {:?}",
            context.action
        );
        Authorization::Deny
    } else {
        Authorization::Allow
    }
}

/// A database backup entry for the UI
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub filename: String,
    pub size_bytes: u64,
    pub created_at: String, // ISO 8601
}

impl Database {
    /// Export as SQLite-compatible SQL text (in-memory string, full export)
    #[cfg(test)]
    pub fn export_sql_string(&self) -> Result<String, AppError> {
        let snapshot = self.snapshot_to_memory()?;
        Self::dump_sql(&snapshot, &[])
    }

    /// Export as SQLite-compatible SQL text
    #[cfg(test)]
    pub fn export_sql(&self, target_path: &Path) -> Result<(), AppError> {
        let dump = self.export_sql_string()?;

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }

        crate::config::atomic_write(target_path, dump.as_bytes())
    }

    /// Import from SQL file, return the generated backup ID (empty string if no backup)
    pub fn import_sql(&self, source_path: &Path) -> Result<String, AppError> {
        if !source_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "SQL file does not exist: {}",
                source_path.display()
            )));
        }

        let sql_raw = fs::read_to_string(source_path).map_err(|e| AppError::io(source_path, e))?;
        let sql_content = sql_raw.trim_start_matches('\u{feff}');
        self.import_sql_string(sql_content)
    }

    /// Import from SQL string, returns the generated backup ID (empty string if no backup)
    pub fn import_sql_string(&self, sql_raw: &str) -> Result<String, AppError> {
        self.import_sql_string_inner(sql_raw)
    }

    fn import_sql_string_inner(&self, sql_raw: &str) -> Result<String, AppError> {
        self.import_sql_string_inner_with_hook(sql_raw, || Ok(()))
    }

    fn import_sql_string_inner_with_hook<F>(
        &self,
        sql_raw: &str,
        on_staging_ready: F,
    ) -> Result<String, AppError>
    where
        F: FnOnce() -> Result<(), AppError>,
    {
        let sql_content = sql_raw.trim_start_matches('\u{feff}');
        Self::validate_copilot_bridge_atlas_sql_export(sql_content)?;

        // Execute the import in the temporary database to ensure that failure does not pollute the main database
        let temp_file = NamedTempFile::new().map_err(|e| AppError::IoContext {
            context: "Failed to create temporary database file".to_string(),
            source: e,
        })?;
        let temp_path = temp_file.path().to_path_buf();
        let temp_conn =
            Connection::open(&temp_path).map_err(|e| AppError::Database(e.to_string()))?;
        // SQLite Backup copies the source database header into the destination.
        // Configure the empty staging database before creating any tables so a
        // SQL import cannot downgrade the main DB from incremental vacuum to NONE.
        temp_conn
            .execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
            .map_err(|e| {
                AppError::Database(format!("Setting staging database auto_vacuum failed: {e}"))
            })?;

        // Remove the authorizer after running external SQL. Schema validation
        // below uses trusted application statements.
        temp_conn.authorizer(Some(import_authorizer));
        let batch_result = temp_conn.execute_batch(sql_content);
        temp_conn.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
        batch_result
            .map_err(|e| AppError::Database(format!("Failed to execute SQL import: {e}")))?;
        if !temp_conn.is_autocommit() {
            let _ = temp_conn.execute_batch("ROLLBACK;");
            return Err(AppError::Message(
                "The SQL backup transaction is incomplete; the file may be truncated.".into(),
            ));
        }

        // Validate the imported schema before touching the live database.
        Self::validate_imported_schema(&temp_conn)?;
        on_staging_ready()?;

        let backup_file_guard = lock_backup_file_operations()?;
        // Keep one main-DB guard across the safety snapshot, local-table read,
        // and final replacement so neither the rollback point nor preserved
        // device-local rows can miss writes that arrived during staging.
        let backup_path = {
            let mut main_conn = lock_conn!(self.conn);
            let backup_path =
                Self::backup_database_file_from_conn(&backup_file_guard, &main_conn, &[])?;
            let backup = Backup::new(&temp_conn, &mut main_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            Self::complete_backup(&backup, "Replace primary database")?;
            backup_path
        };

        let backup_id = backup_path
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();

        Ok(backup_id)
    }

    /// Create memory snapshots to avoid holding database locks for long periods of time
    #[cfg(test)]
    pub(crate) fn snapshot_to_memory(&self) -> Result<Connection, AppError> {
        let conn = lock_conn!(self.conn);
        let mut snapshot =
            Connection::open_in_memory().map_err(|e| AppError::Database(e.to_string()))?;

        {
            let backup =
                Backup::new(&conn, &mut snapshot).map_err(|e| AppError::Database(e.to_string()))?;
            Self::complete_backup(&backup, "Create an in-memory database snapshot")?;
        }

        Ok(snapshot)
    }

    fn complete_backup(backup: &Backup<'_, '_>, context: &str) -> Result<(), AppError> {
        let result = backup
            .step(-1)
            .map_err(|e| AppError::Database(format!("{context} failed: {e}")))?;
        match result {
            StepResult::Done => Ok(()),
            StepResult::More | StepResult::Busy | StepResult::Locked => Err(AppError::Database(
                format!("{context} is not completed: SQLite Backup returns {result:?}"),
            )),
            _ => Err(AppError::Database(format!(
                "{context} incomplete: SQLite Backup returned unknown status"
            ))),
        }
    }

    fn validate_copilot_bridge_atlas_sql_export(sql: &str) -> Result<(), AppError> {
        let trimmed = sql.trim_start();
        if trimmed.lines().next() == Some(COPILOT_BRIDGE_ATLAS_SQL_EXPORT_HEADER) {
            return Ok(());
        }

        Err(AppError::Message(
            "Only SQL backups exported by Copilot Bridge Atlas are supported.".into(),
        ))
    }

    /// Periodic backup: create a new backup if the latest one is older than the configured interval
    pub(crate) fn periodic_backup_if_needed(&self) -> Result<(), AppError> {
        let interval_hours = crate::settings::effective_backup_interval_hours();
        if interval_hours > 0 {
            let backup_file_guard = lock_backup_file_operations()?;
            let backup_dir = get_app_config_dir().join("backups");
            if !backup_dir.exists() {
                self.backup_database_file_locked(&backup_file_guard)?;
            } else {
                let latest = fs::read_dir(&backup_dir).ok().and_then(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map(|ext| ext == "db").unwrap_or(false))
                        .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
                        .max()
                });

                let interval_secs = u64::from(interval_hours) * 3600;
                let needs_backup = match latest {
                    None => true,
                    Some(last_modified) => {
                        last_modified.elapsed().unwrap_or_default()
                            > std::time::Duration::from_secs(interval_secs)
                    }
                };

                if needs_backup {
                    log::info!(
                        "Periodic backup: latest backup is older than {interval_hours} hours, creating new backup"
                    );
                    self.backup_database_file_locked(&backup_file_guard)?;
                }
            }
        }

        // Periodic maintenance is always enabled, regardless of auto-backup settings.
        let mut reclaimed_rows = 0u64;
        match self.rollup_and_prune(30) {
            Ok(deleted) => {
                reclaimed_rows += deleted;
            }
            Err(e) => {
                log::warn!("Periodic rollup_and_prune failed: {e}");
            }
        }
        if reclaimed_rows > 0 {
            let conn = lock_conn!(self.conn);
            if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum;") {
                log::warn!("Periodic incremental vacuum failed: {e}");
            }
        }

        Ok(())
    }

    /// Generate a consistent snapshot backup and return its path, or None if the main database is absent.
    pub(crate) fn backup_database_file(&self) -> Result<Option<PathBuf>, AppError> {
        let backup_file_guard = lock_backup_file_operations()?;
        self.backup_database_file_locked(&backup_file_guard)
    }

    fn backup_database_file_locked(
        &self,
        backup_file_guard: &BackupFileOperationGuard,
    ) -> Result<Option<PathBuf>, AppError> {
        let conn = lock_conn!(self.conn);
        Self::backup_database_file_from_conn(backup_file_guard, &conn, &[])
    }

    /// Create a safety backup from a connection whose caller already owns both
    /// the backup-file operation guard and the appropriate database guard.
    fn backup_database_file_from_conn(
        backup_file_guard: &BackupFileOperationGuard,
        source_conn: &Connection,
        protected_paths: &[&Path],
    ) -> Result<Option<PathBuf>, AppError> {
        Self::backup_database_file_from_conn_with_hook(
            backup_file_guard,
            source_conn,
            protected_paths,
            |_, _| Ok(()),
        )
    }

    fn backup_database_file_from_conn_with_hook<F>(
        _backup_file_guard: &BackupFileOperationGuard,
        source_conn: &Connection,
        protected_paths: &[&Path],
        before_publish: F,
    ) -> Result<Option<PathBuf>, AppError>
    where
        F: FnOnce(&Path, &Path) -> Result<(), AppError>,
    {
        let db_path = get_app_config_dir().join("copilot-bridge-atlas.db");
        if !db_path.exists() {
            return Ok(None);
        }

        let backup_dir = db_path
            .parent()
            .ok_or_else(|| AppError::Config("Invalid database path".to_string()))?
            .join("backups");

        fs::create_dir_all(&backup_dir).map_err(|e| AppError::io(&backup_dir, e))?;

        let base_id = format!("db_backup_{}", Local::now().format("%Y%m%d_%H%M%S"));
        let mut next_suffix = 0;
        let mut backup_path =
            Self::next_available_backup_path(&backup_dir, &base_id, &mut next_suffix);

        // Build and validate the backup under a non-.db temporary name. Backup
        // discovery and retention only see the final path after the complete
        // SQLite image has been atomically published.
        let mut temp_path = Builder::new()
            .prefix(".copilot-bridge-atlas-backup-")
            .suffix(".tmp")
            .tempfile_in(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .into_temp_path();
        let temp_db_path: &Path = temp_path.as_ref();
        let mut dest_conn =
            Connection::open(temp_db_path).map_err(|e| AppError::Database(e.to_string()))?;
        let backup = Backup::new(source_conn, &mut dest_conn)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Self::complete_backup(&backup, "Create a secure backup of your database")?;
        drop(backup);
        Self::validate_sqlite_integrity(&dest_conn)?;
        dest_conn.close().map_err(|(_, e)| {
            AppError::Database(format!("Failed to close database security backup: {e}"))
        })?;
        before_publish(temp_db_path, &backup_path)?;

        loop {
            match temp_path.persist_noclobber(&backup_path) {
                Ok(()) => break,
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    temp_path = error.path;
                    backup_path =
                        Self::next_available_backup_path(&backup_dir, &base_id, &mut next_suffix);
                }
                Err(error) => return Err(AppError::io(&backup_path, error.error)),
            }
        }

        // The newly created safety backup must never be the cleanup victim.
        // During restore, the selected source is protected as well. If the
        // configured retention is too small to keep both, temporarily exceed
        // it instead of deleting either side of the recovery operation.
        let mut cleanup_protected = Vec::with_capacity(protected_paths.len() + 1);
        cleanup_protected.push(backup_path.as_path());
        cleanup_protected.extend_from_slice(protected_paths);
        Self::cleanup_db_backups(&backup_dir, &cleanup_protected)?;
        Ok(Some(backup_path))
    }

    fn next_available_backup_path(
        backup_dir: &Path,
        base_id: &str,
        next_suffix: &mut usize,
    ) -> PathBuf {
        loop {
            let backup_id = if *next_suffix == 0 {
                base_id.to_string()
            } else {
                format!("{base_id}_{}", *next_suffix)
            };
            *next_suffix += 1;
            let backup_path = backup_dir.join(format!("{backup_id}.db"));
            if !backup_path.exists() {
                return backup_path;
            }
        }
    }

    fn same_existing_backup_path(left: &Path, right: &Path) -> bool {
        match (fs::canonicalize(left), fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => left == right,
        }
    }

    /// Clean up old database backups and keep the latest N
    fn cleanup_db_backups(dir: &Path, protected_paths: &[&Path]) -> Result<(), AppError> {
        let retain = crate::settings::effective_backup_retain_count();
        let entries = match fs::read_dir(dir) {
            Ok(iter) => iter
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .map(|ext| ext == "db")
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>(),
            Err(_) => return Ok(()),
        };

        if entries.len() <= retain {
            return Ok(());
        }

        let remove_count = entries.len().saturating_sub(retain);
        let mut sorted = entries;
        sorted.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());

        let mut removed = 0;
        for entry in sorted {
            if removed >= remove_count {
                break;
            }
            let path = entry.path();
            if protected_paths
                .iter()
                .any(|protected| Self::same_existing_backup_path(&path, protected))
            {
                continue;
            }

            if let Err(err) = fs::remove_file(&path) {
                log::warn!(
                    "Failed to delete old database backup {}: {}",
                    path.display(),
                    err
                );
            } else {
                removed += 1;
            }
        }
        Ok(())
    }

    fn validate_sqlite_integrity(conn: &Connection) -> Result<(), AppError> {
        let mut stmt = conn
            .prepare("PRAGMA quick_check;")
            .map_err(|e| AppError::Database(format!("Database integrity check failed: {e}")))?;
        let results = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| AppError::Database(format!("Database integrity check failed: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("Database integrity check failed: {e}")))?;

        if results.len() == 1 && results[0].eq_ignore_ascii_case("ok") {
            return Ok(());
        }

        Err(AppError::Message(format!(
            "Database backup integrity check failed: {}",
            results.join("; ")
        )))
    }

    /// Validate that the external SQL created a recognizable Copilot Bridge Atlas schema.
    ///
    /// Require the current version and tables before accepting an import.
    fn validate_imported_schema(conn: &Connection) -> Result<(), AppError> {
        let version = Self::get_user_version(conn)?;
        if version != super::SCHEMA_VERSION {
            return Err(AppError::Database(format!(
                "Unsupported database schema {version}; Atlas 6 requires schema {}.",
                super::SCHEMA_VERSION
            )));
        }
        const REQUIRED_TABLES: &[&str] = &[
            "providers",
            "settings",
            "proxy_config",
            "proxy_request_logs",
            "proxy_request_diagnostics",
            "usage_daily_rollups",
            "model_pricing",
        ];

        let mut missing = Vec::new();
        for table in REQUIRED_TABLES {
            if !Self::table_exists(conn, table)? {
                missing.push(*table);
            }
        }
        if !missing.is_empty() {
            let names = missing.join(", ");
            return Err(AppError::Message(format!(
                "The imported SQL is missing required Copilot Bridge Atlas tables: {names}"
            )));
        }
        Ok(())
    }

    /// Export database as SQL text in the internal serialization tests.
    #[cfg(test)]
    fn dump_sql(conn: &Connection, skip_tables: &[&str]) -> Result<String, AppError> {
        let mut output = String::new();
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let user_version: i64 = conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap_or(0);

        output.push_str(&format!(
            "-- Copilot Bridge Atlas SQLite export\n-- Generated at: {timestamp}\n-- user_version: {user_version}\n"
        ));
        output.push_str("PRAGMA foreign_keys=OFF;\n");
        output.push_str(&format!("PRAGMA user_version={user_version};\n"));
        output.push_str("BEGIN TRANSACTION;\n");

        // export schema
        let mut stmt = conn
            .prepare(
                "SELECT type, name, tbl_name, sql
                 FROM sqlite_master
                 WHERE sql NOT NULL AND type IN ('table','index','trigger','view')
                 ORDER BY type='table' DESC, name",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut tables = Vec::new();
        let mut triggers = Vec::new();
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(e.to_string()))?;
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let obj_type: String = row.get(0).map_err(|e| AppError::Database(e.to_string()))?;
            let name: String = row.get(1).map_err(|e| AppError::Database(e.to_string()))?;
            let sql: String = row.get(3).map_err(|e| AppError::Database(e.to_string()))?;

            // Skip SQLite internal objects (such as sqlite_sequence)
            if name.starts_with("sqlite_") {
                continue;
            }

            if obj_type == "trigger" {
                triggers.push(sql);
                continue;
            }

            output.push_str(&sql);
            output.push_str(";\n");
            if obj_type == "table" {
                tables.push(name);
            }
        }

        // Export data
        for table in tables {
            if skip_tables.iter().any(|t| *t == table) {
                continue;
            }
            let columns = Self::get_table_columns(conn, &table)?;
            if columns.is_empty() {
                continue;
            }

            // One INSERT per line is the source of slow import: the recovery side needs to separate each statement
            // Parsing/preparation/finishing, measured 21 seconds for 20,000 lines (same slow as on the memory bank, the explanation is
            // Pure CPU not I/O). The same data is <100ms after merging into multiple rows of VALUES.
            // SQLite supports multi-row VALUES starting from 3.7.11 (2012), and the import side is universal
            // execute_batch, can read both old and new formats - no worries about backward compatibility.
            let quoted_table = Self::quote_identifier(&table);
            let quoted_columns = columns
                .iter()
                .map(|column| Self::quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ");
            let insert_prefix = format!("INSERT INTO {quoted_table} ({quoted_columns}) VALUES ");

            let mut stmt = conn
                .prepare(&format!("SELECT {quoted_columns} FROM {quoted_table}"))
                .map_err(|e| AppError::Database(e.to_string()))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut pending_rows = 0usize;
            let mut batch = String::new();
            while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
                let mut values = Vec::with_capacity(columns.len());
                for idx in 0..columns.len() {
                    let value = row
                        .get_ref(idx)
                        .map_err(|e| AppError::Database(e.to_string()))?;
                    values.push(Self::format_sql_value(value)?);
                }

                let row_sql = format!("({})", values.join(", "));
                let separator_bytes = usize::from(pending_rows > 0);
                if pending_rows > 0
                    && batch.len() + separator_bytes + row_sql.len() + 2 > INSERT_BATCH_MAX_BYTES
                {
                    batch.push_str(";\n");
                    output.push_str(&batch);
                    pending_rows = 0;
                }

                if pending_rows == 0 {
                    batch.clear();
                    batch.push_str(&insert_prefix);
                } else {
                    batch.push(',');
                }
                batch.push_str(&row_sql);
                pending_rows += 1;

                if pending_rows >= INSERT_BATCH_MAX_ROWS {
                    batch.push_str(";\n");
                    output.push_str(&batch);
                    pending_rows = 0;
                }
            }
            if pending_rows > 0 {
                batch.push_str(";\n");
                output.push_str(&batch);
            }
        }

        Self::dump_sqlite_sequences(conn, skip_tables, &mut output)?;

        // Triggers must be created after loading table data so they cannot
        // change dump rows or abandon the remainder of a multi-row INSERT.
        for sql in triggers {
            output.push_str(&sql);
            output.push_str(";\n");
        }

        output.push_str("COMMIT;\nPRAGMA foreign_keys=ON;\n");
        Ok(output)
    }

    #[cfg(test)]
    fn dump_sqlite_sequences(
        conn: &Connection,
        skip_tables: &[&str],
        output: &mut String,
    ) -> Result<(), AppError> {
        if !Self::table_exists(conn, "sqlite_sequence")? {
            return Ok(());
        }

        let mut stmt = conn
            .prepare("SELECT name, seq FROM sqlite_sequence ORDER BY name")
            .map_err(|e| {
                AppError::Database(format!("Failed to read AUTOINCREMENT sequence: {e}"))
            })?;
        let mut rows = stmt.query([]).map_err(|e| {
            AppError::Database(format!("Query for AUTOINCREMENT sequence failed: {e}"))
        })?;
        let mut values = Vec::new();
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let table: String = row.get(0).map_err(|e| {
                AppError::Database(format!("Failed to parse AUTOINCREMENT table name: {e}"))
            })?;
            if skip_tables.iter().any(|skipped| *skipped == table) {
                continue;
            }
            let sequence = row.get_ref(1).map_err(|e| {
                AppError::Database(format!("Failed to parse table {table} sequence: {e}"))
            })?;
            values.push(format!(
                "({}, {})",
                Self::format_sql_value(ValueRef::Text(table.as_bytes()))?,
                Self::format_sql_value(sequence)?
            ));
        }

        // Data INSERTs update sqlite_sequence to MAX(rowid), which loses a
        // deleted high-water mark. Replace those derived values with the exact
        // source metadata after all user-table rows have been loaded.
        output.push_str("DELETE FROM sqlite_sequence;\n");
        if !values.is_empty() {
            output.push_str("INSERT INTO sqlite_sequence (name, seq) VALUES ");
            output.push_str(&values.join(","));
            output.push_str(";\n");
        }
        Ok(())
    }

    #[cfg(test)]
    fn quote_identifier(identifier: &str) -> String {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }

    /// Get a list of table column names
    #[cfg(test)]
    fn get_table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, AppError> {
        let quoted_table = Self::quote_identifier(table);
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({quoted_table})"))
            .map_err(|e| AppError::Database(e.to_string()))?;
        let iter = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut columns = Vec::new();
        for col in iter {
            columns.push(col.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(columns)
    }

    /// Format SQL values
    #[cfg(test)]
    fn format_sql_value(value: ValueRef<'_>) -> Result<String, AppError> {
        match value {
            ValueRef::Null => Ok("NULL".to_string()),
            ValueRef::Integer(i) => Ok(i.to_string()),
            ValueRef::Real(f) => Ok(Self::format_sql_real(f)),
            ValueRef::Text(t) => match std::str::from_utf8(t) {
                // SQLite's SQL parser treats NUL as the end of the statement.
                // Keep readable literals for normal UTF-8, and use a hex cast
                // whenever a TEXT value cannot safely appear in SQL source.
                Ok(text) if !text.contains('\0') => {
                    let escaped = text.replace('\'', "''");
                    Ok(format!("'{escaped}'"))
                }
                _ => Ok(format!("CAST({} AS TEXT)", Self::format_sql_blob(t))),
            },
            ValueRef::Blob(bytes) => Ok(Self::format_sql_blob(bytes)),
        }
    }

    #[cfg(test)]
    fn format_sql_real(value: f64) -> String {
        if value.is_nan() {
            // SQLite normalizes bound NaN values to NULL as well.
            return "NULL".to_string();
        }
        if value.is_infinite() {
            return if value.is_sign_negative() {
                "-9.0e999".to_string()
            } else {
                "9.0e999".to_string()
            };
        }
        if value == 0.0 && value.is_sign_negative() {
            return "-0.0".to_string();
        }

        let mut literal = value.to_string();
        if !literal.contains(['.', 'e', 'E']) {
            // Without a decimal point/exponent SQLite stores integer-valued
            // REALs as INTEGER in columns without REAL affinity (e.g. STRICT ANY).
            literal.push_str(".0");
        }
        literal
    }

    #[cfg(test)]
    fn format_sql_blob(bytes: &[u8]) -> String {
        let mut s = String::from("X'");
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(&mut s, "{b:02X}");
        }
        s.push('\'');
        s
    }

    /// List all database backup files, sorted by creation time (newest first)
    pub fn list_backups() -> Result<Vec<BackupEntry>, AppError> {
        let _backup_file_guard = lock_backup_file_operations()?;
        let backup_dir = get_app_config_dir().join("backups");
        if !backup_dir.exists() {
            return Ok(vec![]);
        }

        let mut entries: Vec<BackupEntry> = fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "db").unwrap_or(false))
            .filter_map(|e| {
                let metadata = e.metadata().ok()?;
                let filename = e.file_name().to_string_lossy().to_string();
                let size_bytes = metadata.len();
                let created_at = metadata
                    .modified()
                    .ok()
                    .map(|t| {
                        let dt: chrono::DateTime<Utc> = t.into();
                        dt.to_rfc3339()
                    })
                    .unwrap_or_default();
                Some(BackupEntry {
                    filename,
                    size_bytes,
                    created_at,
                })
            })
            .collect();

        // Sort by created_at descending (newest first)
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(entries)
    }

    /// Restore database from a backup file. Returns the safety backup ID.
    pub fn restore_from_backup(&self, filename: &str) -> Result<String, AppError> {
        self.restore_from_backup_with_hook(filename, |_| Ok(()))
    }

    fn restore_from_backup_with_hook<F>(
        &self,
        filename: &str,
        before_replace: F,
    ) -> Result<String, AppError>
    where
        F: FnOnce(Option<&Path>) -> Result<(), AppError>,
    {
        // Security: validate filename to prevent path traversal
        if filename.contains("..")
            || filename.contains('/')
            || filename.contains('\\')
            || !filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        let backup_file_guard = lock_backup_file_operations()?;
        let backup_dir = get_app_config_dir().join("backups");
        let backup_path = backup_dir.join(filename);

        if !backup_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {filename}"
            )));
        }

        // Open read-only before creating the safety backup. `Connection::open`
        // would recreate a source removed by retention cleanup as an empty DB.
        let source_conn = Connection::open_with_flags(
            &backup_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Stage and fully validate the selected file before touching the live
        // connection. A corrupt or incompatible backup must
        // leave the current database unchanged.
        let temp_file = NamedTempFile::new().map_err(|e| AppError::IoContext {
            context: "Failed to create database recovery temporary file".to_string(),
            source: e,
        })?;
        let mut staging_conn =
            Connection::open(temp_file.path()).map_err(|e| AppError::Database(e.to_string()))?;
        {
            let backup = Backup::new(&source_conn, &mut staging_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            Self::complete_backup(&backup, "Read database backup")?;
        }
        drop(source_conn);

        Self::validate_sqlite_integrity(&staging_conn)?;
        Self::validate_imported_schema(&staging_conn)?;
        Self::ensure_incremental_auto_vacuum_on_conn(&staging_conn)?;
        Self::ensure_model_pricing_seeded_on_conn(&staging_conn)?;
        Self::validate_sqlite_integrity(&staging_conn)?;

        // Keep one main-DB guard across the safety snapshot and final apply so
        // the safety file exactly represents the state being replaced.
        let safety_backup = {
            let mut main_conn = lock_conn!(self.conn);
            let safety_backup = Self::backup_database_file_from_conn(
                &backup_file_guard,
                &main_conn,
                &[backup_path.as_path()],
            )?;
            before_replace(safety_backup.as_deref())?;
            let backup = Backup::new(&staging_conn, &mut main_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            Self::complete_backup(&backup, "Restore master database")?;
            safety_backup
        };
        let safety_id = safety_backup
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();

        log::info!("Database restored from backup: {filename}, safety backup: {safety_id}");
        Ok(safety_id)
    }

    /// Rename a backup file. Returns the new filename.
    pub fn rename_backup(old_filename: &str, new_name: &str) -> Result<String, AppError> {
        // Validate old filename (path traversal + .db suffix)
        if old_filename.contains("..")
            || old_filename.contains('/')
            || old_filename.contains('\\')
            || !old_filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        // Clean new name
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return Err(AppError::InvalidInput(
                "New name cannot be empty".to_string(),
            ));
        }

        // Length limit (without .db suffix)
        let name_part = trimmed.strip_suffix(".db").unwrap_or(trimmed);
        if name_part.len() > 100 {
            return Err(AppError::InvalidInput(
                "Name too long (max 100 characters)".to_string(),
            ));
        }

        // Prevent path traversal in new name
        if name_part.contains("..")
            || name_part.contains('/')
            || name_part.contains('\\')
            || name_part.contains('\0')
        {
            return Err(AppError::InvalidInput(
                "Invalid characters in new name".to_string(),
            ));
        }

        let new_filename = format!("{name_part}.db");

        let _backup_file_guard = lock_backup_file_operations()?;
        let backup_dir = get_app_config_dir().join("backups");
        let old_path = backup_dir.join(old_filename);
        let new_path = backup_dir.join(&new_filename);

        if !old_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {old_filename}"
            )));
        }

        if new_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "A backup named '{new_filename}' already exists"
            )));
        }

        fs::rename(&old_path, &new_path).map_err(|e| AppError::io(&old_path, e))?;
        log::info!("Renamed backup: {old_filename} -> {new_filename}");
        Ok(new_filename)
    }

    /// Delete a backup file permanently.
    pub fn delete_backup(filename: &str) -> Result<(), AppError> {
        // Validate filename (path traversal + .db suffix)
        if filename.contains("..")
            || filename.contains('/')
            || filename.contains('\\')
            || !filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        let _backup_file_guard = lock_backup_file_operations()?;
        let backup_path = get_app_config_dir().join("backups").join(filename);
        if !backup_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {filename}"
            )));
        }

        fs::remove_file(&backup_path).map_err(|e| AppError::io(&backup_path, e))?;
        log::info!("Deleted backup: {filename}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{lock_backup_file_operations, Database};
    use crate::error::AppError;
    use crate::settings::{get_settings, update_settings, AppSettings};
    use rusqlite::Connection;
    use serial_test::serial;

    struct TestHomeGuard {
        previous_test_home: Option<std::ffi::OsString>,
        temp_dir: tempfile::TempDir,
    }

    impl TestHomeGuard {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().expect("create isolated test home");
            let previous_test_home = std::env::var_os("COPILOT_BRIDGE_ATLAS_TEST_HOME");
            std::env::set_var("COPILOT_BRIDGE_ATLAS_TEST_HOME", temp_dir.path());
            // Prevent the Windows legacy-HOME fallback without mutating HOME:
            // an existing default DB keeps get_app_config_dir() anchored under
            // COPILOT_BRIDGE_ATLAS_TEST_HOME and makes import exercise its safety backup.
            let config_dir = temp_dir.path().join(".copilot-bridge-atlas");
            std::fs::create_dir_all(&config_dir).expect("create isolated config directory");
            std::fs::File::create(config_dir.join("copilot-bridge-atlas.db"))
                .expect("create isolated database sentinel");
            let guard = Self {
                previous_test_home,
                temp_dir,
            };
            let resolved = crate::config::get_app_config_dir();
            assert!(
                resolved.starts_with(guard.temp_dir.path()),
                "isolated test home resolved outside its temp directory: {}",
                resolved.display()
            );
            guard
        }

        fn path(&self) -> &std::path::Path {
            self.temp_dir.path()
        }
    }

    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            match self.previous_test_home.as_ref() {
                Some(previous) => std::env::set_var("COPILOT_BRIDGE_ATLAS_TEST_HOME", previous),
                None => std::env::remove_var("COPILOT_BRIDGE_ATLAS_TEST_HOME"),
            }
        }
    }

    struct SettingsGuard {
        previous: AppSettings,
    }

    impl SettingsGuard {
        fn with_backup_retain_count(retain: u32) -> Self {
            let previous = get_settings();
            let mut next = previous.clone();
            next.backup_retain_count = Some(retain);
            update_settings(next).expect("set backup retention for test");
            Self { previous }
        }
    }

    impl Drop for SettingsGuard {
        fn drop(&mut self) {
            let _ = update_settings(self.previous.clone());
        }
    }

    #[test]
    #[serial]
    fn import_rejects_cross_file_statements_and_leaves_no_file_behind() -> Result<(), AppError> {
        let test_home = TestHomeGuard::new();
        // `VACUUM INTO` is the one most easily missed by keyword scanning solutions: it does not contain the word "ATTACH".
        // But it falls into `AuthAction::Attach` (actual test) like ATTACH, so the same rule blocks both.
        let cases: [(&str, &str); 2] = [
            ("attach", "ATTACH DATABASE '{path}' AS evil;"),
            ("vacuum-into", "VACUUM INTO '{path}';"),
        ];

        for (label, template) in cases {
            let target = test_home
                .path()
                .join(format!("copilot-bridge-atlas-authorizer-{label}.sqlite"));

            // Legal export header + out-of-bounds statement. The header verification is only better than the prefix, and this input passed it.
            // The one who really stops it must be the authorizer.
            let malicious = format!(
                "{}\n{}\n",
                super::COPILOT_BRIDGE_ATLAS_SQL_EXPORT_HEADER,
                template.replace("{path}", &target.to_string_lossy().replace('\'', "''"))
            );

            let db = Database::memory()?;
            let result = db.import_sql_string(&malicious);

            let error = result.expect_err("Out-of-bounds SQL must be rejected");
            assert!(
                error.to_string().to_ascii_lowercase().contains("authoriz"),
                "{label} must be rejected by authorizer, actual error: {error}"
            );
            // Reporting an error alone is not enough: a file can be created before staging schema validation.
            // If the guard fails, even if the entire import fails, the file will already be on the disk.
            assert!(
                !target.exists(),
                "Rejected {label} must not leave files on disk: {}",
                target.display()
            );
        }
        Ok(())
    }

    #[test]
    #[serial]
    fn import_still_accepts_a_genuine_export() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        // The whitelist is tightened, and there must be a return line of defense to prove that it did not accidentally damage its own export format——
        // If this test is red, it means that dump_sql has written statements that are not covered by the whitelist.
        let source = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(source.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('p1', 'codex', 'Provider One', '{}', '{}')",
                [],
            )?;
        }
        let exported = source.export_sql_string()?;

        let target = Database::memory()?;
        target.import_sql_string(&exported)?;

        let conn = crate::database::lock_conn!(target.conn);
        let name: String = conn.query_row(
            "SELECT name FROM providers WHERE id = 'p1' AND app_type = 'codex'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(name, "Provider One");
        Ok(())
    }

    #[test]
    #[serial]
    fn import_accepts_genuine_export_without_providers() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let source = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(source.conn);
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))?;
            assert_eq!(count, 0);
            assert!(!Database::table_exists(&conn, "stream_check_logs")?);
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('empty-export-marker', 'kept')",
                [],
            )?;
        }
        let sql = source.export_sql_string()?;

        let target = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(target.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('target-sentinel', 'codex', 'Must Be Cleared', '{}', '{}')",
                [],
            )?;
        }
        target.import_sql_string(&sql)?;

        let conn = crate::database::lock_conn!(target.conn);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))?;
        assert_eq!(count, 0);
        assert!(!Database::table_exists(&conn, "stream_check_logs")?);
        let marker: String = conn.query_row(
            "SELECT value FROM settings WHERE key = 'empty-export-marker'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(marker, "kept");
        Ok(())
    }

    #[test]
    #[serial]
    fn import_rejects_header_only_sql_and_keeps_existing_database() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let target = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(target.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('sentinel', 'codex', 'Existing Provider', '{}', '{}')",
                [],
            )?;
        }

        let header_only = format!(
            "{}\nPRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\nCOMMIT;\n",
            super::COPILOT_BRIDGE_ATLAS_SQL_EXPORT_HEADER
        );
        let error = target
            .import_sql_string(&header_only)
            .expect_err("Files missing the original schema must be rejected");
        assert!(error.to_string().contains("Atlas 6 requires schema"));

        let conn = crate::database::lock_conn!(target.conn);
        let provider: (i64, String) = conn.query_row(
            "SELECT COUNT(*), MIN(name) FROM providers WHERE id = 'sentinel'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(provider, (1, "Existing Provider".into()));
        Ok(())
    }

    #[test]
    #[serial]
    fn sql_import_preserves_incremental_auto_vacuum() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let source = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(source.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('vacuum-provider', 'codex', 'Vacuum Provider', '{}', '{}')",
                [],
            )?;
        }
        let sql = source.export_sql_string()?;

        let target = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(target.conn);
            assert_eq!(Database::get_auto_vacuum_mode(&conn)?, 2);
        }

        target.import_sql_string(&sql)?;

        let conn = crate::database::lock_conn!(target.conn);
        assert_eq!(
            Database::get_auto_vacuum_mode(&conn)?,
            2,
            "SQL import must not downgrade the main database's INCREMENTAL auto_vacuum to NONE"
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn sql_file_api_round_trips_existing_export_behavior() -> Result<(), AppError> {
        let test_home = TestHomeGuard::new();
        let source = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(source.conn);
            conn.execute_batch(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('file-provider', 'codex', 'File Provider', '{}', '{}');
                 INSERT INTO proxy_request_logs (
                     request_id, provider_id, app_type, model,
                     input_tokens, output_tokens, total_cost_usd,
                     latency_ms, status_code, created_at
                 ) VALUES ('file-request', 'file-provider', 'codex', 'gpt-6-astra', 5, 3, '0', 10, 200, 1);",
            )?;
        }

        let backup_path = test_home.path().join("round-trip.sql");
        source.export_sql(&backup_path)?;

        let target = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(target.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('target-sentinel', 'codex', 'Must Be Replaced', '{}', '{}')",
                [],
            )?;
        }
        target.import_sql(&backup_path)?;

        let conn = crate::database::lock_conn!(target.conn);
        let providers = conn
            .prepare("SELECT id FROM providers ORDER BY id")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(providers, vec!["file-provider"]);
        let request_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM proxy_request_logs WHERE request_id = 'file-request')",
            [],
            |row| row.get(0),
        )?;
        assert!(request_exists, "File API must fully restore exported data");
        Ok(())
    }

    #[test]
    #[serial]
    fn failed_sql_import_keeps_the_existing_database_unchanged() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let target = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(target.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('sentinel', 'codex', 'Existing Provider', '{}', '{}')",
                [],
            )?;
        }

        let invalid_sql = format!(
            "{}\nBEGIN TRANSACTION;\nCREATE TABLE partial (id INTEGER);\nTHIS IS NOT SQL;\n",
            super::COPILOT_BRIDGE_ATLAS_SQL_EXPORT_HEADER
        );
        assert!(target.import_sql_string(&invalid_sql).is_err());

        let conn = crate::database::lock_conn!(target.conn);
        let provider: (i64, String, String) = conn.query_row(
            "SELECT COUNT(*), MIN(id), MIN(name) FROM providers",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(provider, (1, "sentinel".into(), "Existing Provider".into()));
        let partial_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'partial')",
            [],
            |row| row.get(0),
        )?;
        assert!(
            !partial_exists,
            "Temporary objects from failed imports must not enter the main database"
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn import_rejects_truncated_open_transaction_and_keeps_live_database() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let source = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(source.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('remote-provider', 'codex', 'Remote Provider', '{}', '{}')",
                [],
            )?;
        }
        let exported = source.export_sql_string()?;
        let truncated = exported
            .strip_suffix("COMMIT;\nPRAGMA foreign_keys=ON;\n")
            .expect("Copilot Bridge Atlas export should end with a committed transaction");

        let target = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(target.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('live-provider', 'codex', 'Live Provider', '{}', '{}')",
                [],
            )?;
        }

        let error = target
            .import_sql_string(truncated)
            .expect_err("an export truncated before COMMIT must be rejected");
        assert!(
            error.to_string().contains("incomplete"),
            "unexpected error: {error}"
        );

        let conn = crate::database::lock_conn!(target.conn);
        let providers = conn
            .prepare("SELECT id FROM providers ORDER BY id")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(providers, vec!["live-provider"]);
        Ok(())
    }

    #[test]
    #[serial]
    fn dump_sql_batches_rows_into_multi_row_inserts() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        // One INSERT per line is the source of slow import (parsed one by one on the recovery side, measured 21s for 20,000 rows).
        // This test nails the batch format: 450 rows must be combined into ceil(450/200) = 3 statements.
        // Once you return to line-by-line export, this area will immediately turn red.
        let db = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(db.conn);
            for i in 0..450 {
                conn.execute(
                    "INSERT INTO providers (id, app_type, name, settings_config, meta)
                     VALUES (?1, 'codex', 'p', '{}', '{}')",
                    [format!("p{i}")],
                )?;
            }
        }

        let sql = db.export_sql_string()?;
        let insert_count = sql.matches("INSERT INTO \"providers\"").count();
        assert_eq!(
            insert_count, 3,
            "450 rows should be combined into 3 multi-row INSERTs (200 rows per batch), actual {insert_count} rows"
        );

        let target = Database::memory()?;
        target.import_sql_string(&sql)?;
        let conn = crate::database::lock_conn!(target.conn);
        let row_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))?;
        assert_eq!(
            row_count, 450,
            "Batch boundaries must not contain missing or duplicate rows"
        );
        for boundary in [0, 199, 200, 399, 400, 449] {
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM providers WHERE id = ?1)",
                [format!("p{boundary}")],
                |row| row.get(0),
            )?;
            assert!(
                exists,
                "Batch boundary row p{boundary} must be restored in full"
            );
        }
        Ok(())
    }

    #[test]
    fn dump_sql_splits_large_rows_by_statement_bytes() -> Result<(), AppError> {
        let source = Connection::open_in_memory()?;
        source.execute(
            "CREATE TABLE large_rows (id INTEGER PRIMARY KEY, payload TEXT NOT NULL)",
            [],
        )?;

        // Each row fits below the byte cap, while any pair exceeds it.
        let payload = "x".repeat(super::INSERT_BATCH_MAX_BYTES / 2 + 1024);
        for id in 1..=3 {
            source.execute(
                "INSERT INTO large_rows (id, payload) VALUES (?1, ?2)",
                rusqlite::params![id, payload],
            )?;
        }

        let sql = Database::dump_sql(&source, &[])?;
        let inserts = sql
            .lines()
            .filter(|line| line.starts_with("INSERT INTO \"large_rows\""))
            .collect::<Vec<_>>();
        assert_eq!(
            inserts.len(),
            3,
            "Very large fields should be batched in advance according to the number of SQL bytes"
        );
        assert!(
            inserts
                .iter()
                .all(|statement| statement.len() <= super::INSERT_BATCH_MAX_BYTES),
            "Each independently sized INSERT should remain within the byte limit"
        );

        let target = Connection::open_in_memory()?;
        target.execute_batch(&sql)?;
        let (count, min_len, max_len): (i64, i64, i64) = target.query_row(
            "SELECT COUNT(*), MIN(length(payload)), MAX(length(payload)) FROM large_rows",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(count, 3);
        assert_eq!(min_len, payload.len() as i64);
        assert_eq!(max_len, payload.len() as i64);
        Ok(())
    }

    #[test]
    fn dump_sql_round_trips_generated_columns_and_quoted_identifiers() -> Result<(), AppError> {
        let source = Connection::open_in_memory()?;
        source.execute_batch(
            r#"
            CREATE TABLE "generated""values" (
                "a" TEXT NOT NULL,
                "computed" TEXT GENERATED ALWAYS AS ("a" || '-generated') STORED,
                "b""tail" TEXT NOT NULL
            );
            INSERT INTO "generated""values" ("a", "b""tail")
            VALUES ('source', 'ordinary-tail');
            "#,
        )?;

        let sql = Database::dump_sql(&source, &[])?;
        assert!(sql.contains("INSERT INTO \"generated\"\"values\" (\"a\", \"b\"\"tail\") VALUES"));

        let target = Connection::open_in_memory()?;
        target.execute_batch(&sql)?;
        let values: (String, String, String) = target.query_row(
            "SELECT \"a\", \"computed\", \"b\"\"tail\" FROM \"generated\"\"values\"",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(
            values,
            (
                "source".to_string(),
                "source-generated".to_string(),
                "ordinary-tail".to_string()
            )
        );
        Ok(())
    }

    #[test]
    fn dump_sql_preserves_text_bytes_and_real_storage_class() -> Result<(), AppError> {
        let source = Connection::open_in_memory()?;
        source.execute_batch(
            "CREATE TABLE scalar_values (
                 label TEXT PRIMARY KEY,
                 value ANY
             ) STRICT;
             INSERT INTO scalar_values VALUES ('nul-text', CAST(X'610062' AS TEXT));
             INSERT INTO scalar_values VALUES ('invalid-text', CAST(X'80FF' AS TEXT));",
        )?;
        for (label, value) in [
            ("real-one", 1.0),
            ("negative-zero", -0.0),
            ("positive-infinity", f64::INFINITY),
            ("negative-infinity", f64::NEG_INFINITY),
        ] {
            source.execute(
                "INSERT INTO scalar_values (label, value) VALUES (?1, ?2)",
                rusqlite::params![label, value],
            )?;
        }

        let sql = Database::dump_sql(&source, &[])?;
        assert!(
            sql.contains("CAST(X'610062' AS TEXT)") && sql.contains("CAST(X'80FF' AS TEXT)"),
            "TEXT with NUL or illegal UTF-8 must use hexadecimal expression"
        );

        let target = Connection::open_in_memory()?;
        target.execute_batch(&sql)?;

        for (label, expected_hex) in [("nul-text", "610062"), ("invalid-text", "80FF")] {
            let (storage_class, bytes): (String, String) = target.query_row(
                "SELECT typeof(value), hex(value) FROM scalar_values WHERE label = ?1",
                [label],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert_eq!(storage_class, "text");
            assert_eq!(bytes, expected_hex);
        }

        let real_value = |label: &str| -> Result<(String, f64), rusqlite::Error> {
            target.query_row(
                "SELECT typeof(value), value FROM scalar_values WHERE label = ?1",
                [label],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        };
        let (storage_class, one) = real_value("real-one")?;
        assert_eq!(storage_class, "real");
        assert_eq!(one, 1.0);

        let (storage_class, negative_zero) = real_value("negative-zero")?;
        assert_eq!(storage_class, "real");
        assert_eq!(negative_zero, 0.0);
        assert!(negative_zero.is_sign_negative());

        let (storage_class, positive_infinity) = real_value("positive-infinity")?;
        assert_eq!(storage_class, "real");
        assert_eq!(positive_infinity, f64::INFINITY);

        let (storage_class, negative_infinity) = real_value("negative-infinity")?;
        assert_eq!(storage_class, "real");
        assert_eq!(negative_infinity, f64::NEG_INFINITY);
        Ok(())
    }

    #[test]
    fn dump_sql_preserves_autoincrement_high_water_marks() -> Result<(), AppError> {
        let source = Connection::open_in_memory()?;
        source.execute_batch(
            "CREATE TABLE autoincrement_rows (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 value TEXT NOT NULL
             );
             INSERT INTO autoincrement_rows (value) VALUES ('one'), ('two'), ('deleted-high');
             DELETE FROM autoincrement_rows WHERE id = 3;",
        )?;

        let sql = Database::dump_sql(&source, &[])?;
        let target = Connection::open_in_memory()?;
        target.execute_batch(&sql)?;

        let sequence: i64 = target.query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'autoincrement_rows'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(
            sequence, 3,
            "The highest removed ID should still remain in the sequence high water mark"
        );
        target.execute(
            "INSERT INTO autoincrement_rows (value) VALUES ('after-restore')",
            [],
        )?;
        assert_eq!(target.last_insert_rowid(), 4);
        Ok(())
    }

    #[test]
    fn dump_sql_loads_rows_before_creating_triggers() -> Result<(), AppError> {
        let source = Connection::open_in_memory()?;
        source.execute_batch(
            "CREATE TABLE triggered_rows (seq INTEGER PRIMARY KEY);
             INSERT INTO triggered_rows VALUES (1), (2), (3);
             CREATE TRIGGER ignore_second_row
             BEFORE INSERT ON triggered_rows
             WHEN NEW.seq = 2
             BEGIN
                 SELECT RAISE(IGNORE);
             END;",
        )?;

        let sql = Database::dump_sql(&source, &[])?;
        let data_pos = sql.find("INSERT INTO \"triggered_rows\"").unwrap();
        let trigger_pos = sql.find("CREATE TRIGGER ignore_second_row").unwrap();
        assert!(
            data_pos < trigger_pos,
            "The trigger must be created after data recovery is complete"
        );

        let target = Connection::open_in_memory()?;
        target.execute_batch(&sql)?;
        let rows = target
            .prepare("SELECT seq FROM triggered_rows ORDER BY seq")?
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(rows, vec![1, 2, 3]);
        let trigger_exists: bool = target.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = 'ignore_second_row')",
            [],
            |row| row.get(0),
        )?;
        assert!(
            trigger_exists,
            "The trigger itself must still be restored with the backup"
        );
        Ok(())
    }

    #[test]
    fn dump_sql_preserves_indexes_and_views() -> Result<(), AppError> {
        let source = Connection::open_in_memory()?;
        source.execute_batch(
            "CREATE TABLE indexed_rows (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
             CREATE UNIQUE INDEX indexed_rows_value_idx ON indexed_rows(value);
             CREATE VIEW indexed_rows_view AS
                 SELECT id, value FROM indexed_rows WHERE value LIKE 'kept%';
             CREATE TRIGGER a_insert_indexed_rows_view
             INSTEAD OF INSERT ON indexed_rows_view
             BEGIN
                 INSERT INTO indexed_rows (id, value) VALUES (NEW.id, NEW.value);
             END;
             INSERT INTO indexed_rows VALUES (1, 'kept-value'), (2, 'hidden-value');",
        )?;

        let sql = Database::dump_sql(&source, &[])?;
        let target = Connection::open_in_memory()?;
        target.execute_batch(&sql)?;

        for (object_type, object_name) in [
            ("index", "indexed_rows_value_idx"),
            ("view", "indexed_rows_view"),
            ("trigger", "a_insert_indexed_rows_view"),
        ] {
            let exists: bool = target.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2
                )",
                [object_type, object_name],
                |row| row.get(0),
            )?;
            assert!(
                exists,
                "{object_type} {object_name} must be restored with SQL dump"
            );
        }

        target.execute(
            "INSERT INTO indexed_rows_view (id, value) VALUES (3, 'kept-via-trigger')",
            [],
        )?;
        let view_rows = target
            .prepare("SELECT id, value FROM indexed_rows_view ORDER BY id")?
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            view_rows,
            vec![
                (1, "kept-value".to_string()),
                (3, "kept-via-trigger".to_string()),
            ]
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn multi_row_dump_round_trips_special_values() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        // Newlines, quotes, commas, emoji, BLOBs, and NULL must survive batched SQL export.
        let source = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(source.conn);
            conn.execute("ALTER TABLE providers ADD COLUMN imported_note TEXT", [])?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('special', 'codex', ?1, ?2, '{}')",
                rusqlite::params![
                    "O'Brien,\n second line \" quoted\" 😀",
                    "{\"key\": \"it's, ok\"}"
                ],
            )?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('with-blob', 'codex', 'blob', X'00FF10', '{}')",
                [],
            )?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta, imported_note)
                 VALUES ('with-null', 'codex', 'nullcat', '{}', '{}', NULL)",
                [],
            )?;
        }

        let sql = source.export_sql_string()?;
        let target = Database::memory()?;
        target.import_sql_string(&sql)?;

        let conn = crate::database::lock_conn!(target.conn);
        let name: String = conn.query_row(
            "SELECT name FROM providers WHERE id = 'special'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(name, "O'Brien,\n second line \" quoted\" 😀");
        let cfg: String = conn.query_row(
            "SELECT settings_config FROM providers WHERE id = 'special'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(cfg, "{\"key\": \"it's, ok\"}");

        let blob_type: String = conn.query_row(
            "SELECT typeof(settings_config) FROM providers WHERE id = 'with-blob'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(
            blob_type, "blob",
            "BLOB storage type must be preserved after round trip"
        );
        let blob: Vec<u8> = conn.query_row(
            "SELECT settings_config FROM providers WHERE id = 'with-blob'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(blob, vec![0x00, 0xFF, 0x10]);

        let imported_note: Option<String> = conn.query_row(
            "SELECT imported_note FROM providers WHERE id = 'with-null'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(imported_note, None, "NULL must survive export and import");
        Ok(())
    }

    #[test]
    #[serial]
    fn failed_backup_publish_leaves_no_visible_or_temporary_file() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let db = Database::init()?;
        let backup_dir = crate::config::get_app_config_dir().join("backups");
        std::fs::create_dir_all(&backup_dir).map_err(|e| AppError::io(&backup_dir, e))?;
        let mut files_before = std::fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>();
        files_before.sort();
        let mut visible_before = Database::list_backups()?
            .into_iter()
            .map(|entry| entry.filename)
            .collect::<Vec<_>>();
        visible_before.sort();

        let error = {
            let backup_file_guard = lock_backup_file_operations()?;
            let conn = crate::database::lock_conn!(db.conn);
            Database::backup_database_file_from_conn_with_hook(
                &backup_file_guard,
                &conn,
                &[],
                |temp_path, target_path| {
                    assert!(
                        temp_path.exists(),
                        "completed backup should exist before publish"
                    );
                    assert_ne!(
                        temp_path.extension().and_then(|ext| ext.to_str()),
                        Some("db"),
                        "staging files must stay invisible to backup discovery"
                    );
                    assert!(!target_path.exists());
                    Err(AppError::Config("simulated publish failure".to_string()))
                },
            )
        }
        .expect_err("publish failure must be returned");
        assert!(error.to_string().contains("simulated publish failure"));
        let mut visible_after = Database::list_backups()?
            .into_iter()
            .map(|entry| entry.filename)
            .collect::<Vec<_>>();
        visible_after.sort();
        assert_eq!(visible_after, visible_before);
        let mut files_after = std::fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>();
        files_after.sort();
        assert_eq!(
            files_after, files_before,
            "failed publish must not leave either a visible backup or a temporary file"
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn backup_publish_retries_a_noclobber_name_collision() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let _settings = SettingsGuard::with_backup_retain_count(10);
        let db = Database::init()?;
        let mut claimed_path = None;

        let published_path = {
            let backup_file_guard = lock_backup_file_operations()?;
            let conn = crate::database::lock_conn!(db.conn);
            Database::backup_database_file_from_conn_with_hook(
                &backup_file_guard,
                &conn,
                &[],
                |_, target_path| {
                    claimed_path = Some(target_path.to_path_buf());
                    std::fs::write(target_path, b"claimed by another process")
                        .map_err(|e| AppError::io(target_path, e))?;
                    Ok(())
                },
            )?
            .expect("file-backed database should create a backup")
        };

        let claimed_path = claimed_path.expect("publish hook should receive the first target");
        assert_ne!(published_path, claimed_path);
        assert_eq!(
            std::fs::read(&claimed_path).map_err(|e| AppError::io(&claimed_path, e))?,
            b"claimed by another process"
        );
        let published_conn = Connection::open_with_flags(
            &published_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        Database::validate_sqlite_integrity(&published_conn)?;

        let backup_dir = crate::config::get_app_config_dir().join("backups");
        let temporary_files = std::fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".copilot-bridge-atlas-backup-")
            })
            .count();
        assert_eq!(
            temporary_files, 0,
            "publish retry must consume the temp file"
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn concurrent_backup_renames_never_overwrite_the_shared_target() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let _settings = SettingsGuard::with_backup_retain_count(10);
        let db = Database::init()?;
        let mut source_filenames = Vec::new();
        for provider_id in ["first-source", "second-source"] {
            {
                let conn = crate::database::lock_conn!(db.conn);
                conn.execute("DELETE FROM providers", [])?;
                conn.execute(
                    "INSERT INTO providers (id, app_type, name, settings_config, meta)
                     VALUES (?1, 'codex', ?1, '{}', '{}')",
                    [provider_id],
                )?;
            }
            let source_path = db
                .backup_database_file()?
                .expect("file-backed database should create a backup");
            source_filenames.push(
                source_path
                    .file_name()
                    .expect("backup should have a filename")
                    .to_string_lossy()
                    .into_owned(),
            );
        }

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let handles = source_filenames
            .iter()
            .cloned()
            .map(|source_filename| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Database::rename_backup(&source_filename, "shared-target")
                        .map_err(|e| e.to_string())
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| AppError::Config("rename thread panicked".to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

        let backup_dir = crate::config::get_app_config_dir().join("backups");
        let target_path = backup_dir.join("shared-target.db");
        let remaining_source = source_filenames
            .iter()
            .map(|filename| backup_dir.join(filename))
            .find(|path| path.exists())
            .expect("the losing source must remain after the target collision");
        let mut provider_ids = [&target_path, &remaining_source]
            .into_iter()
            .map(|path| -> Result<String, AppError> {
                let conn =
                    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                conn.query_row("SELECT id FROM providers", [], |row| row.get(0))
                    .map_err(AppError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        provider_ids.sort();
        assert_eq!(provider_ids, vec!["first-source", "second-source"]);
        Ok(())
    }

    #[test]
    #[serial]
    fn usage_columns_round_trip_through_database_and_sql_backups() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let _settings = SettingsGuard::with_backup_retain_count(10);
        let db = Database::init()?;
        assert_eq!(
            db.get_usage_table_columns()?,
            crate::settings::UsageTableColumns::default()
        );
        db.set_usage_table_columns("requestLogs", vec!["time".into(), "cost".into()])?;
        let expected =
            db.set_usage_table_columns("modelStats", vec!["model".into(), "requests".into()])?;
        let grouping =
            serde_json::from_value(serde_json::json!({"interval":3,"unit":"hour"})).unwrap();
        let date_range = crate::settings::UsageDateRange {
            preset: "7d".into(),
            ..Default::default()
        };
        db.set_usage_trend_grouping(grouping)?;
        db.set_usage_date_range(date_range.clone())?;
        assert_eq!(expected.request_logs.as_ref().unwrap(), &["time", "cost"]);
        let backup = db.backup_database_file()?.unwrap();
        let exported = db.export_sql_string()?;
        db.set_usage_table_columns("requestLogs", vec!["status".into()])?;
        db.set_usage_trend_grouping(Default::default())?;
        db.set_usage_date_range(Default::default())?;
        db.restore_from_backup(backup.file_name().unwrap().to_str().unwrap())?;
        assert_eq!(db.get_usage_table_columns()?, expected);
        assert_eq!(db.get_usage_trend_grouping()?, grouping);
        assert_eq!(db.get_usage_date_range()?, date_range);
        db.set_usage_table_columns("modelStats", vec!["cost".into()])?;
        db.set_usage_trend_grouping(Default::default())?;
        db.set_usage_date_range(Default::default())?;
        db.import_sql_string(&exported)?;
        assert_eq!(db.get_usage_table_columns()?, expected);
        assert_eq!(db.get_usage_trend_grouping()?, grouping);
        assert_eq!(db.get_usage_date_range()?, date_range);
        assert!(db.set_usage_table_columns("unknown", vec![]).is_err());
        assert_eq!(db.get_usage_table_columns()?, expected);
        Ok(())
    }

    #[test]
    #[serial]
    fn restore_with_retain_one_keeps_source_and_exact_safety_snapshot() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let _settings = SettingsGuard::with_backup_retain_count(1);
        let db = Database::init()?;

        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute("DELETE FROM providers", [])?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('restore-source', 'codex', 'Restore Source', '{}', '{}')",
                [],
            )?;
        }
        let source_path = db
            .backup_database_file()?
            .expect("file-backed database should create a backup");
        let source_filename = source_path
            .file_name()
            .expect("backup should have a filename")
            .to_string_lossy()
            .into_owned();
        let backup_dir = crate::config::get_app_config_dir().join("backups");
        let stale_path = backup_dir.join("stale-unprotected.db");
        std::fs::write(&stale_path, b"stale").map_err(|e| AppError::io(&stale_path, e))?;

        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute("DELETE FROM providers", [])?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('live-before-restore', 'codex', 'Live Before Restore', '{}', '{}')",
                [],
            )?;
        }

        let safety_id = db.restore_from_backup(&source_filename)?;
        let safety_path = backup_dir.join(format!("{safety_id}.db"));
        assert!(
            source_path.exists(),
            "selected restore source must be retained"
        );
        assert!(
            safety_path.exists(),
            "pre-restore safety backup must be retained"
        );
        assert!(
            !stale_path.exists(),
            "retention should still remove an unprotected stale backup"
        );

        let backup_count = std::fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "db"))
            .count();
        assert_eq!(
            backup_count, 2,
            "retain=1 may be exceeded temporarily to protect both recovery endpoints"
        );

        let live_provider: String = {
            let conn = crate::database::lock_conn!(db.conn);
            conn.query_row("SELECT id FROM providers", [], |row| row.get(0))?
        };
        assert_eq!(live_provider, "restore-source");

        let safety_conn =
            Connection::open_with_flags(&safety_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let safety_provider: String =
            safety_conn.query_row("SELECT id FROM providers", [], |row| row.get(0))?;
        assert_eq!(
            safety_provider, "live-before-restore",
            "safety backup must exactly represent the live state being replaced"
        );
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    #[serial]
    fn restore_protects_case_variant_source_path_from_retention() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let _settings = SettingsGuard::with_backup_retain_count(1);
        let db = Database::init()?;
        let source_path = db
            .backup_database_file()?
            .expect("file-backed database should create a backup");
        let source_filename = source_path
            .file_name()
            .expect("backup should have a filename")
            .to_string_lossy()
            .into_owned();
        let case_variant = format!(
            "{}.db",
            source_filename
                .strip_suffix(".db")
                .expect("generated backup should use a .db suffix")
                .to_ascii_uppercase()
        );

        db.restore_from_backup(&case_variant)?;
        assert!(
            source_path.exists(),
            "retention must recognize a case-variant path as the selected source"
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn restore_blocks_backup_deletion_until_live_replacement_finishes() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let db = Database::init()?;
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute("DELETE FROM providers", [])?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('restore-source', 'codex', 'Restore Source', '{}', '{}')",
                [],
            )?;
        }
        let source_path = db
            .backup_database_file()?
            .expect("file-backed database should create a backup");
        let source_filename = source_path
            .file_name()
            .expect("backup should have a filename")
            .to_string_lossy()
            .into_owned();
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute("DELETE FROM providers", [])?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('live-before-restore', 'codex', 'Live Before Restore', '{}', '{}')",
                [],
            )?;
        }

        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let mut delete_handle = None;
        let mut observed_safety_filename = None;
        let safety_id = db.restore_from_backup_with_hook(&source_filename, |safety_path| {
            let safety_path = safety_path.ok_or_else(|| {
                AppError::Config("restore should create a safety backup".to_string())
            })?;
            let safety_filename = safety_path
                .file_name()
                .ok_or_else(|| AppError::Config("safety backup has no filename".to_string()))?
                .to_string_lossy()
                .into_owned();
            observed_safety_filename = Some(safety_filename.clone());
            delete_handle = Some(std::thread::spawn(move || {
                let _ = attempt_tx.send(());
                let result = Database::delete_backup(&safety_filename).map_err(|e| e.to_string());
                let _ = result_tx.send(result);
            }));

            attempt_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .map_err(|e| AppError::Config(format!("delete thread did not start: {e}")))?;
            match result_rx.recv_timeout(std::time::Duration::from_millis(150)) {
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(()),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(AppError::Config(
                    "delete thread disconnected before restore completed".to_string(),
                )),
                Ok(result) => Err(AppError::Config(format!(
                    "backup deletion completed before live replacement: {result:?}"
                ))),
            }
        })?;

        let delete_result = result_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|e| AppError::Config(format!("delete did not resume after restore: {e}")))?;
        delete_result.map_err(AppError::Config)?;
        delete_handle
            .expect("delete thread should be created")
            .join()
            .map_err(|_| AppError::Config("delete thread panicked".to_string()))?;

        let expected_safety_filename = format!("{safety_id}.db");
        assert_eq!(
            observed_safety_filename.as_deref(),
            Some(expected_safety_filename.as_str())
        );
        let live_provider: String = {
            let conn = crate::database::lock_conn!(db.conn);
            conn.query_row("SELECT id FROM providers", [], |row| row.get(0))?
        };
        assert_eq!(live_provider, "restore-source");
        Ok(())
    }

    #[test]
    #[serial]
    fn restore_rejects_corrupt_db_before_touching_live_database() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let db = Database::init()?;
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute("DELETE FROM providers", [])?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('live-provider', 'codex', 'Live Provider', '{}', '{}')",
                [],
            )?;
        }

        let backup_dir = crate::config::get_app_config_dir().join("backups");
        std::fs::create_dir_all(&backup_dir).map_err(|e| AppError::io(&backup_dir, e))?;
        let corrupt_path = backup_dir.join("corrupt.db");
        std::fs::write(&corrupt_path, b"not a sqlite database")
            .map_err(|e| AppError::io(&corrupt_path, e))?;
        let mut backups_before = Database::list_backups()?
            .into_iter()
            .map(|entry| entry.filename)
            .collect::<Vec<_>>();
        backups_before.sort();

        db.restore_from_backup("corrupt.db")
            .expect_err("corrupt backup must be rejected");

        let live_provider: String = {
            let conn = crate::database::lock_conn!(db.conn);
            conn.query_row("SELECT id FROM providers", [], |row| row.get(0))?
        };
        assert_eq!(live_provider, "live-provider");
        let mut backups_after = Database::list_backups()?
            .into_iter()
            .map(|entry| entry.filename)
            .collect::<Vec<_>>();
        backups_after.sort();
        assert_eq!(
            backups_after, backups_before,
            "failed staging must not create a safety backup"
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn restore_rejects_future_schema_before_touching_live_database() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();
        let db = Database::init()?;

        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute("DELETE FROM providers", [])?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('future-source', 'codex', 'Future Source', '{}', '{}')",
                [],
            )?;
        }
        let source_path = db
            .backup_database_file()?
            .expect("file-backed database should create a backup");
        let source_filename = source_path
            .file_name()
            .expect("backup should have a filename")
            .to_string_lossy()
            .into_owned();
        {
            let source_conn = Connection::open(&source_path)?;
            source_conn.execute_batch(&format!(
                "PRAGMA user_version = {};",
                crate::database::SCHEMA_VERSION + 1
            ))?;
        }

        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute("DELETE FROM providers", [])?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('live-provider', 'codex', 'Live Provider', '{}', '{}')",
                [],
            )?;
        }

        let backup_dir = crate::config::get_app_config_dir().join("backups");
        let backup_count_before = std::fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "db"))
            .count();

        let error = db
            .restore_from_backup(&source_filename)
            .expect_err("future-schema backup must be rejected");
        assert_eq!(
            error.to_string(),
            format!(
                "Database error: Unsupported database schema {}; Atlas 6 requires schema {}.",
                crate::database::SCHEMA_VERSION + 1,
                crate::database::SCHEMA_VERSION,
            ),
        );

        let live_provider: String = {
            let conn = crate::database::lock_conn!(db.conn);
            conn.query_row("SELECT id FROM providers", [], |row| row.get(0))?
        };
        assert_eq!(
            live_provider, "live-provider",
            "failed staging validation must not replace the live database"
        );

        let backup_count_after = std::fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "db"))
            .count();
        assert_eq!(
            backup_count_after, backup_count_before,
            "staging failure should occur before creating a redundant safety backup"
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn periodic_maintenance_runs_even_when_auto_backup_disabled() -> Result<(), AppError> {
        let _test_home = TestHomeGuard::new();

        let settings = AppSettings {
            backup_interval_hours: Some(0),
            ..AppSettings::default()
        };
        update_settings(settings).expect("disable auto backup");

        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        let old_ts = now - 40 * 86400;

        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES ('old-req', 'p1', 'codex', 'gpt-6-astra', 100, 50, '0.01', 100, 200, ?1)",
                [old_ts],
            )?;
        }

        db.periodic_backup_if_needed()?;

        let (remaining_request_logs, rollups): (i64, i64) = {
            let conn = crate::database::lock_conn!(db.conn);
            let remaining_request_logs =
                conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                    row.get(0)
                })?;
            let rollups =
                conn.query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |row| {
                    row.get(0)
                })?;
            (remaining_request_logs, rollups)
        };

        assert_eq!(
            remaining_request_logs, 0,
            "old request logs should still be pruned when auto backup is disabled"
        );
        assert_eq!(rollups, 1, "old request logs should be rolled up");

        Ok(())
    }

    /// Performance benchmarks (not regression tests): measured with row counts close to heavy proxy users
    /// The time consumption and product size of the three paths of export/local file import/synchronous import.
    ///
    /// Manual operation: `cargo test --lib perf_backup -- --ignored --nocapture`
    #[test]
    #[ignore = "perf harness, run explicitly"]
    #[serial]
    fn perf_backup_export_import_paths() -> Result<(), AppError> {
        use std::time::Instant;

        const LOG_ROWS: usize = 20_000;
        const ROLLUP_ROWS: usize = 1_000;

        let _test_home = TestHomeGuard::new();

        fn populate(db: &Database, log_rows: usize, rollup_rows: usize) -> Result<(), AppError> {
            let mut conn = crate::database::lock_conn!(db.conn);
            let tx = conn.transaction()?;
            for i in 0..50 {
                tx.execute(
                    "INSERT INTO providers (id, app_type, name, settings_config, meta)
                     VALUES (?1, 'codex', ?2, '{}', '{}')",
                    rusqlite::params![format!("p{i}"), format!("Provider {i}")],
                )?;
            }
            for i in 0..log_rows {
                tx.execute(
                    "INSERT INTO proxy_request_logs (
                        request_id, provider_id, app_type, model,
                        input_tokens, output_tokens, total_cost_usd,
                        latency_ms, status_code, created_at
                    ) VALUES (?1, 'p1', 'codex', 'gpt-6-astra', 100, 50, '0.01', 120, 200, 1000)",
                    [format!("req-{i}")],
                )?;
            }
            for i in 0..rollup_rows {
                // (date, app_type, provider_id, model, request_model, pricing_model)
                // There is a UNIQUE constraint on the date, the date must be unique row by row.
                let date = format!(
                    "{:04}-{:02}-{:02}",
                    2025 + i / 336,
                    i / 28 % 12 + 1,
                    i % 28 + 1
                );
                tx.execute(
                    "INSERT INTO usage_daily_rollups (
                        date, app_type, provider_id, model, request_count, success_count,
                        input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                        total_cost_usd, avg_latency_ms
                    ) VALUES (?1, 'codex', 'p1', 'gpt-6-astra', 7, 7, 700, 350, 0, 0, '0.07', 120)",
                    [date],
                )?;
            }
            tx.commit()?;
            Ok(())
        }

        let source = Database::memory()?;
        populate(&source, LOG_ROWS, ROLLUP_ROWS)?;

        let t = Instant::now();
        let full_sql = source.export_sql_string()?;
        println!(
            "export_sql_string (full): {:?}, {} bytes",
            t.elapsed(),
            full_sql.len()
        );

        let t = Instant::now();
        let import_target = Database::memory()?;
        import_target.import_sql_string(&full_sql)?;
        println!("import_sql_string (local file path): {:?}", t.elapsed());
        {
            let conn = crate::database::lock_conn!(import_target.conn);
            let counts: (i64, i64, i64) = conn.query_row(
                "SELECT
                    (SELECT COUNT(*) FROM providers),
                    (SELECT COUNT(*) FROM proxy_request_logs),
                    (SELECT COUNT(*) FROM usage_daily_rollups)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            assert_eq!(counts, (50, LOG_ROWS as i64, ROLLUP_ROWS as i64));
        }

        Ok(())
    }
}
