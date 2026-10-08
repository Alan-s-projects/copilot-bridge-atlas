//! SQLite storage for the Copilot provider, proxy, usage history, and backups.

pub(crate) mod backup;
mod dao;
mod schema;

pub(crate) use dao::request_diagnostics::RequestDiagnosticDetail;

#[cfg(test)]
mod tests;

// DAO types exported for external use
use crate::config::get_app_config_dir;
use crate::error::AppError;
use rusqlite::Connection;
use std::sync::Mutex;

// DAO methods are provided through impl Database, no additional export is required

/// Atlas 6 storage format.
pub(crate) const SCHEMA_VERSION: i32 = 1;
pub(crate) const APPLICATION_ID: i32 = 0x4154_4c36; // ATL6

/// Safely acquire Mutex locks and avoid unwrap panics
macro_rules! lock_conn {
    ($mutex:expr) => {
        $mutex
            .lock()
            .map_err(|e| AppError::Database(format!("Mutex lock failed: {}", e)))?
    };
}

// Export macros for use by submodules
pub(crate) use lock_conn;

/// Database connection encapsulation
///
/// Wrap a Connection with a Mutex to support sharing in multi-threaded environments such as Tauri State.
/// rusqlite::Connection itself is not Sync, so this layer of packaging is needed.
pub struct Database {
    pub(crate) conn: Mutex<Connection>,
}

impl Database {
    /// Initialize database connection and create table
    ///
    /// Database files are located at `~/.copilot-bridge-atlas/copilot-bridge-atlas.db`
    pub fn init() -> Result<Self, AppError> {
        let db_path = get_app_config_dir().join("copilot-bridge-atlas.db");

        // Make sure the parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }

        let conn = Connection::open(&db_path).map_err(|e| AppError::Database(e.to_string()))?;

        // Enable foreign key constraints
        conn.execute("PRAGMA foreign_keys = ON;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        if Self::get_user_version(&conn)? == 0 && !Self::has_user_tables(&conn)? {
            // Configure a new database before creating tables.
            conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        let db = Self {
            conn: Mutex::new(conn),
        };

        db.initialize_schema()?;
        if let Err(e) = db.ensure_incremental_auto_vacuum() {
            log::warn!("Failed to ensure incremental auto-vacuum: {e}");
        }
        let seeded_models = db.ensure_model_pricing_seeded()?;
        if let Err(e) =
            crate::services::model_pricing::sync_seeded_model_pricing(&db, &seeded_models)
        {
            log::warn!("Failed to sync local model pricing file: {e}");
        }

        // Startup cleanup: prune old logs and reclaim space
        if let Err(e) = db.rollup_and_prune(30) {
            log::warn!("Startup rollup_and_prune failed: {e}");
        }
        // Reclaim disk space after cleanup
        {
            let conn = lock_conn!(db.conn);
            if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum;") {
                log::warn!("Startup incremental vacuum failed: {e}");
            }
        }

        Ok(db)
    }

    /// Create an in-memory database (for testing)
    #[cfg(test)]
    pub fn memory() -> Result<Self, AppError> {
        let conn = Connection::open_in_memory().map_err(|e| AppError::Database(e.to_string()))?;

        // Enable foreign key constraints
        conn.execute("PRAGMA foreign_keys = ON;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.initialize_schema()?;
        db.ensure_model_pricing_seeded()?;

        Ok(db)
    }

    pub(crate) fn get_auto_vacuum_mode(conn: &Connection) -> Result<i32, AppError> {
        conn.query_row("PRAGMA auto_vacuum;", [], |row| row.get(0))
            .map_err(|e| AppError::Database(format!("Failed to read auto_vacuum: {e}")))
    }

    fn has_user_tables(conn: &Connection) -> Result<bool, AppError> {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(format!("Failed to read table quantity: {e}")))?;
        Ok(count > 0)
    }

    pub(crate) fn ensure_incremental_auto_vacuum_on_conn(
        conn: &Connection,
    ) -> Result<bool, AppError> {
        let mode = Self::get_auto_vacuum_mode(conn)?;
        if mode == 2 {
            return Ok(false);
        }

        let has_tables = Self::has_user_tables(conn)?;
        conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
            .map_err(|e| AppError::Database(format!("Failed to set auto_vacuum: {e}")))?;

        if !has_tables {
            return Ok(false);
        }

        conn.execute("VACUUM;", [])
            .map_err(|e| AppError::Database(format!("Failed to execute VACUUM: {e}")))?;
        conn.execute("PRAGMA foreign_keys = ON;", [])
            .map_err(|e| AppError::Database(format!("Restore foreign_keys failed: {e}")))?;
        Ok(true)
    }

    pub(crate) fn ensure_incremental_auto_vacuum(&self) -> Result<bool, AppError> {
        let mode = {
            let conn = lock_conn!(self.conn);
            Self::get_auto_vacuum_mode(&conn)?
        };
        if mode == 2 {
            return Ok(false);
        }

        let has_tables = {
            let conn = lock_conn!(self.conn);
            Self::has_user_tables(&conn)?
        };
        if has_tables {
            log::info!(
                "Detected auto_vacuum={mode}, rebuilding database to enable incremental vacuum"
            );
            self.backup_database_file()?;
        }

        let rebuilt = {
            let conn = lock_conn!(self.conn);
            Self::ensure_incremental_auto_vacuum_on_conn(&conn)?
        };

        if rebuilt {
            log::info!("Incremental auto-vacuum enabled after database rebuild");
        } else {
            log::info!("Incremental auto-vacuum configured for new database");
        }

        Ok(rebuilt)
    }
}
