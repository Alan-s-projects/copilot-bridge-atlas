//! Use statistics related commands

use crate::database::{Database, RequestDiagnosticDetail};
use crate::error::AppError;
use crate::services::model_pricing::ModelPricingInfo;
use crate::services::usage_stats::*;
use crate::store::AppState;
use tauri::State;

async fn blocking_usage<T: Send + 'static>(
    state: State<'_, AppState>,
    operation: impl FnOnce(&Database) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || operation(&db))
        .await
        .map_err(|error| AppError::Message(format!("Usage operation failed: {error}")))?
}

/// Get usage summary
#[tauri::command]
pub async fn get_usage_summary(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
    client_name: Option<String>,
) -> Result<UsageSummary, AppError> {
    blocking_usage(state, move |db| {
        crate::copilot_bridge::require_codex(app_type.as_deref().unwrap_or("codex"))?;
        db.get_usage_summary_filtered(
            start_date,
            end_date,
            Some("codex"),
            provider_name.as_deref(),
            model.as_deref(),
            client_name.as_deref(),
        )
    })
    .await
}

/// Get daily trends
#[tauri::command]
pub async fn get_usage_trends(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
    client_name: Option<String>,
    grouping: Option<TrendGrouping>,
) -> Result<UsageTrends, AppError> {
    blocking_usage(state, move |db| {
        crate::copilot_bridge::require_codex(app_type.as_deref().unwrap_or("codex"))?;
        db.get_grouped_usage_trends_filtered(
            start_date,
            end_date,
            Some("codex"),
            provider_name.as_deref(),
            model.as_deref(),
            client_name.as_deref(),
            grouping.unwrap_or_default(),
        )
    })
    .await
}

/// Get model statistics
#[tauri::command]
pub async fn get_model_stats(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
    client_name: Option<String>,
) -> Result<Vec<ModelStats>, AppError> {
    blocking_usage(state, move |db| {
        crate::copilot_bridge::require_codex(app_type.as_deref().unwrap_or("codex"))?;
        db.get_model_stats_filtered(
            start_date,
            end_date,
            Some("codex"),
            provider_name.as_deref(),
            model.as_deref(),
            client_name.as_deref(),
        )
    })
    .await
}

/// Get models with token usage but no matching local price.
#[tauri::command]
pub async fn get_unpriced_model_usage(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
    client_name: Option<String>,
) -> Result<Vec<UnpricedModelUsage>, AppError> {
    blocking_usage(state, move |db| {
        crate::copilot_bridge::require_codex(app_type.as_deref().unwrap_or("codex"))?;
        db.get_unpriced_model_usage_filtered(
            start_date,
            end_date,
            Some("codex"),
            provider_name.as_deref(),
            model.as_deref(),
            client_name.as_deref(),
        )
    })
    .await
}

/// Distinct calling apps retained in request or rollup history.
#[tauri::command]
pub async fn get_usage_clients(state: State<'_, AppState>) -> Result<Vec<String>, AppError> {
    blocking_usage(state, move |db| db.get_usage_clients()).await
}

/// Get request log list
#[tauri::command]
pub async fn get_request_logs(
    state: State<'_, AppState>,
    mut filters: LogFilters,
    page: u32,
    page_size: u32,
) -> Result<PaginatedLogs, AppError> {
    blocking_usage(state, move |db| {
        filters.app_type = Some("codex".into());
        db.get_request_logs(&filters, page, page_size)
    })
    .await
}

/// Read a failed request's bounded headers and bodies only when its details are opened.
#[tauri::command]
pub async fn get_request_diagnostics(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Option<RequestDiagnosticDetail>, AppError> {
    blocking_usage(state, move |db| db.get_request_diagnostics(&request_id)).await
}

/// Get model pricing list
#[tauri::command]
pub async fn get_model_pricing(
    state: State<'_, AppState>,
) -> Result<Vec<ModelPricingInfo>, AppError> {
    blocking_usage(state, move |db| {
        let seeded_models = db.ensure_model_pricing_seeded()?;
        crate::services::model_pricing::sync_seeded_model_pricing(db, &seeded_models)?;

        let conn = crate::database::lock_conn!(db.conn);

        let mut stmt = conn.prepare(
            "SELECT model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million, long_context
         FROM model_pricing
         ORDER BY display_name",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(ModelPricingInfo {
                model_id: row.get(0)?,
                display_name: row.get(1)?,
                input_cost_per_million: row.get(2)?,
                output_cost_per_million: row.get(3)?,
                cache_read_cost_per_million: row.get(4)?,
                cache_creation_cost_per_million: row.get(5)?,
                long_context: row
                    .get::<_, Option<String>>(6)?
                    .map(|value| {
                        serde_json::from_str(&value).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                6,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        })
                    })
                    .transpose()?,
            })
        })?;

        let mut pricing = Vec::new();
        for row in rows {
            pricing.push(row?);
        }

        Ok(pricing)
    })
    .await
}

/// Update model pricing
#[tauri::command]
pub async fn update_model_pricing(
    state: State<'_, AppState>,
    model_id: String,
    display_name: String,
    input_cost: String,
    output_cost: String,
    cache_read_cost: String,
    cache_creation_cost: String,
    long_context: Option<crate::services::model_pricing::LongContextPricing>,
) -> Result<(), AppError> {
    blocking_usage(state, move |db| {
        crate::services::model_pricing::update_model_pricing(
            db,
            ModelPricingInfo {
                long_context,
                model_id,
                display_name,
                input_cost_per_million: input_cost,
                output_cost_per_million: output_cost,
                cache_read_cost_per_million: cache_read_cost,
                cache_creation_cost_per_million: cache_creation_cost,
            },
        )?;
        Ok(())
    })
    .await
}

/// Delete model pricing
#[tauri::command]
pub async fn delete_model_pricing(
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), AppError> {
    blocking_usage(state, move |db| {
        crate::services::model_pricing::delete_model_pricing(db, &model_id)?;
        log::info!("Deleted model pricing: {model_id}");
        Ok(())
    })
    .await
}

/// Reset all model prices to the values bundled with this application.
#[tauri::command]
pub async fn reset_model_pricing_to_defaults(state: State<'_, AppState>) -> Result<(), AppError> {
    blocking_usage(state, move |db| {
        crate::services::model_pricing::reset_model_pricing_to_defaults(db)
    })
    .await
}
