//! 使用统计相关命令

use crate::error::AppError;
use crate::services::model_pricing::ModelPricingInfo;
use crate::services::usage_stats::*;
use crate::store::AppState;
use tauri::State;

/// 获取使用量汇总
#[tauri::command]
pub fn get_usage_summary(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<UsageSummary, AppError> {
    crate::copilot_bridge::require_codex(app_type.as_deref().unwrap_or("codex"))?;
    state.db.get_usage_summary(
        start_date,
        end_date,
        Some("codex"),
        provider_name.as_deref(),
        model.as_deref(),
    )
}

/// 获取每日趋势
#[tauri::command]
pub fn get_usage_trends(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
    grouping: Option<TrendGrouping>,
) -> Result<UsageTrends, AppError> {
    crate::copilot_bridge::require_codex(app_type.as_deref().unwrap_or("codex"))?;
    state.db.get_grouped_usage_trends(
        start_date,
        end_date,
        Some("codex"),
        provider_name.as_deref(),
        model.as_deref(),
        grouping.unwrap_or_default(),
    )
}

/// 获取 Provider 统计
#[tauri::command]
pub fn get_provider_stats(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<ProviderStats>, AppError> {
    crate::copilot_bridge::require_codex(app_type.as_deref().unwrap_or("codex"))?;
    state.db.get_provider_stats(
        start_date,
        end_date,
        Some("codex"),
        provider_name.as_deref(),
        model.as_deref(),
    )
}

/// 获取模型统计
#[tauri::command]
pub fn get_model_stats(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<ModelStats>, AppError> {
    crate::copilot_bridge::require_codex(app_type.as_deref().unwrap_or("codex"))?;
    state.db.get_model_stats(
        start_date,
        end_date,
        Some("codex"),
        provider_name.as_deref(),
        model.as_deref(),
    )
}

/// Get models with token usage but no matching local price.
#[tauri::command]
pub fn get_unpriced_model_usage(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<UnpricedModelUsage>, AppError> {
    crate::copilot_bridge::require_codex(app_type.as_deref().unwrap_or("codex"))?;
    state.db.get_unpriced_model_usage(
        start_date,
        end_date,
        Some("codex"),
        provider_name.as_deref(),
        model.as_deref(),
    )
}

/// 获取请求日志列表
#[tauri::command]
pub fn get_request_logs(
    state: State<'_, AppState>,
    mut filters: LogFilters,
    page: u32,
    page_size: u32,
) -> Result<PaginatedLogs, AppError> {
    filters.app_type = Some("codex".into());
    state.db.get_request_logs(&filters, page, page_size)
}

/// 获取模型定价列表
#[tauri::command]
pub fn get_model_pricing(state: State<'_, AppState>) -> Result<Vec<ModelPricingInfo>, AppError> {
    state.db.ensure_model_pricing_seeded()?;
    crate::services::model_pricing::sync_local_model_pricing(&state.db)?;

    let db = state.db.clone();
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
}

/// 更新模型定价
#[tauri::command]
pub fn update_model_pricing(
    state: State<'_, AppState>,
    model_id: String,
    display_name: String,
    input_cost: String,
    output_cost: String,
    cache_read_cost: String,
    cache_creation_cost: String,
    long_context: Option<crate::services::model_pricing::LongContextPricing>,
) -> Result<(), AppError> {
    crate::services::model_pricing::update_model_pricing(
        &state.db,
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
}

/// 删除模型定价
#[tauri::command]
pub fn delete_model_pricing(state: State<'_, AppState>, model_id: String) -> Result<(), AppError> {
    crate::services::model_pricing::delete_model_pricing(&state.db, &model_id)?;
    log::info!("已删除模型定价: {model_id}");
    Ok(())
}

/// Reset all model prices to the values bundled with this application.
#[tauri::command]
pub fn reset_model_pricing_to_defaults(state: State<'_, AppState>) -> Result<(), AppError> {
    crate::services::model_pricing::reset_model_pricing_to_defaults(&state.db)
}
