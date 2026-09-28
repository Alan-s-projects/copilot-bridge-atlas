use crate::database::{lock_conn, Database};
use crate::{AppError, Provider};
use indexmap::IndexMap;
use rusqlite::{params, OptionalExtension};
use serde::de::DeserializeOwned;

fn json_column<T: DeserializeOwned>(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<T> {
    let value: String = row.get(index)?;
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn provider_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Provider> {
    Ok(Provider {
        id: row.get(0)?,
        name: row.get(1)?,
        settings_config: json_column(row, 2)?,
        meta: json_column(row, 3)?,
    })
}

impl Database {
    pub fn get_all_providers(
        &self,
        app_type: &str,
    ) -> Result<IndexMap<String, Provider>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn
            .prepare(
                "SELECT id, name, settings_config, meta FROM providers
             WHERE app_type = ?1 ORDER BY is_current DESC, id",
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        let providers = statement
            .query_map([app_type], provider_from_row)
            .map_err(|error| AppError::Database(error.to_string()))?
            .map(|row| row.map(|provider| (provider.id.clone(), provider)))
            .collect::<Result<IndexMap<_, _>, _>>()
            .map_err(|error| AppError::Database(error.to_string()))?;
        Ok(providers)
    }

    pub fn get_current_provider(&self, app_type: &str) -> Result<Option<String>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT id FROM providers WHERE app_type = ?1 AND is_current = 1 LIMIT 1",
            [app_type],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| AppError::Database(error.to_string()))
    }

    pub fn get_provider_by_id(
        &self,
        id: &str,
        app_type: &str,
    ) -> Result<Option<Provider>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT id, name, settings_config, meta FROM providers WHERE id = ?1 AND app_type = ?2",
            params![id, app_type],
            provider_from_row,
        )
        .optional()
        .map_err(|error| AppError::Database(error.to_string()))
    }

    pub fn save_provider(&self, app_type: &str, provider: &Provider) -> Result<(), AppError> {
        let mut conn = lock_conn!(self.conn);
        let transaction = conn
            .transaction()
            .map_err(|error| AppError::Database(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id, app_type) DO UPDATE SET
                 name = excluded.name,
                 settings_config = excluded.settings_config,
                 meta = excluded.meta",
                params![
                    provider.id,
                    app_type,
                    provider.name,
                    serde_json::to_string(&provider.settings_config)
                        .map_err(|error| AppError::Database(error.to_string()))?,
                    serde_json::to_string(&provider.meta.as_ref().cloned().unwrap_or_default())
                        .map_err(|error| AppError::Database(error.to_string()))?,
                ],
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| AppError::Database(error.to_string()))
    }

    pub fn set_current_provider(&self, app_type: &str, id: &str) -> Result<(), AppError> {
        let mut conn = lock_conn!(self.conn);
        let transaction = conn
            .transaction()
            .map_err(|error| AppError::Database(error.to_string()))?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM providers WHERE id = ?1 AND app_type = ?2)",
                params![id, app_type],
                |row| row.get(0),
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        if !exists {
            return Err(AppError::InvalidInput("Provider not found".into()));
        }
        transaction
            .execute(
                "UPDATE providers SET is_current = (id = ?2) WHERE app_type = ?1",
                params![app_type, id],
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| AppError::Database(error.to_string()))
    }
}
