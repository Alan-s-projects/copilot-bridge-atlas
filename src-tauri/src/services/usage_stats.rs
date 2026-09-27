//! 使用统计服务
//!
//! 提供使用量数据的聚合查询功能

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::services::sql_helpers::{
    fresh_input_sql, output_generation_ms_sql, INPUT_TOKEN_SEMANTICS_FRESH,
    INPUT_TOKEN_SEMANTICS_TOTAL,
};
#[cfg(test)]
use chrono::NaiveDate;
use chrono::{Local, TimeZone, Timelike};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

mod trends;
pub use trends::{TrendGrouping, UsageTrends};

/// 使用量汇总
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub total_requests: u64,
    pub total_cost: String,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub success_rate: f32,
    #[serde(default)]
    pub avg_latency_ms: f64,
    /// Time-weighted output rate; old rollups fall back to request latency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens_per_second: Option<f64>,
    /// Fresh input + output + cache writes + cache reads, retained for API compatibility.
    pub real_total_tokens: u64,
    /// cache_read / (input + cache_creation + cache_read). Range 0.0–1.0.
    /// Reported as a fraction; multiply by 100 in UI for percentage display.
    pub cache_hit_rate: f64,
}

/// Helper: compute (real_total, hit_rate) from the four token counters.
/// All inputs must already be cache-normalized (i.e. input excludes cache).
fn derive_real_total_and_hit_rate(
    fresh_input: u64,
    output: u64,
    cache_creation: u64,
    cache_read: u64,
) -> (u64, f64) {
    let real_total = fresh_input + output + cache_creation + cache_read;
    let cacheable_input = fresh_input + cache_creation + cache_read;
    let hit_rate = if cacheable_input > 0 {
        cache_read as f64 / cacheable_input as f64
    } else {
        0.0
    };
    (real_total, hit_rate)
}

/// 每日统计
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyStats {
    pub date: String,
    pub request_count: u64,
    pub total_cost: String,
    pub total_tokens: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cache_read_tokens: u64,
}

/// Provider 统计
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStats {
    pub provider_id: String,
    pub provider_name: String,
    pub request_count: u64,
    pub total_tokens: u64,
    pub total_cost: String,
    pub success_rate: f32,
    pub avg_latency_ms: u64,
}

/// 模型统计
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStats {
    pub model: String,
    pub request_count: u64,
    pub total_tokens: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cost: String,
    pub avg_cost_per_request: String,
}

/// Token usage for models that have no matching entry in the local price table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnpricedModelUsage {
    pub model: String,
    pub request_count: u64,
    pub fresh_input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_hit_rate: f64,
}

/// 请求日志过滤器
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFilters {
    pub app_type: Option<String>,
    pub provider_name: Option<String>,
    pub model: Option<String>,
    pub status_code: Option<u16>,
    pub start_date: Option<i64>,
    pub end_date: Option<i64>,
}

/// 分页请求日志响应
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedLogs {
    pub data: Vec<RequestLogDetail>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
}

/// 请求日志详情
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogDetail {
    pub request_id: String,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    pub app_type: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_reasoning_effort: Option<String>,
    pub cost_multiplier: String,
    pub input_tokens: u32,
    /// Cache-normalized input using the stored row's token semantics.
    #[serde(default)]
    pub fresh_input_tokens: u64,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    /// Internal storage semantics; omitted from the UI/API payload.
    #[serde(skip)]
    pub input_token_semantics: i64,
    pub input_cost_usd: String,
    pub output_cost_usd: String,
    pub cache_read_cost_usd: String,
    pub cache_creation_cost_usd: String,
    pub total_cost_usd: String,
    pub is_streaming: bool,
    pub latency_ms: u64,
    pub first_token_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub status_code: u16,
    pub error_message: Option<String>,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_source: Option<String>,
    /// 写入时实际用于计价的模型名。None = v11 前的历史行，"" = 未计价的错误行。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_tier: Option<String>,
}

/// Map the 30-column request-log projection.
///
/// The SELECT must return columns in this order:
/// `request_id, provider_id, provider_name, app_type, model, request_model,
///  cost_multiplier, input_tokens, output_tokens, cache_read_tokens,
///  cache_creation_tokens, input_cost_usd, output_cost_usd, cache_read_cost_usd,
///  cache_creation_cost_usd, total_cost_usd, is_streaming, latency_ms,
///  first_token_ms, duration_ms, status_code, error_message, created_at,
///  data_source, pricing_model, input_token_semantics,
///  requested_reasoning_effort, applied_reasoning_effort, fresh_input_tokens, pricing_tier`
///
/// 不需要 provider_name 时（如 backfill）SELECT `NULL AS provider_name` 占位即可。
fn row_to_request_log_detail(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestLogDetail> {
    Ok(RequestLogDetail {
        request_id: row.get(0)?,
        provider_id: row.get(1)?,
        provider_name: row.get(2)?,
        app_type: row.get(3)?,
        model: row.get(4)?,
        request_model: row.get(5)?,
        cost_multiplier: row
            .get::<_, Option<String>>(6)?
            .unwrap_or_else(|| "1".to_string()),
        input_tokens: row.get::<_, i64>(7)? as u32,
        output_tokens: row.get::<_, i64>(8)? as u32,
        cache_read_tokens: row.get::<_, i64>(9)? as u32,
        cache_creation_tokens: row.get::<_, i64>(10)? as u32,
        input_cost_usd: row.get(11)?,
        output_cost_usd: row.get(12)?,
        cache_read_cost_usd: row.get(13)?,
        cache_creation_cost_usd: row.get(14)?,
        total_cost_usd: row.get(15)?,
        is_streaming: row.get::<_, i64>(16)? != 0,
        latency_ms: row.get::<_, i64>(17)? as u64,
        first_token_ms: row.get::<_, Option<i64>>(18)?.map(|v| v as u64),
        duration_ms: row.get::<_, Option<i64>>(19)?.map(|v| v as u64),
        status_code: row.get::<_, i64>(20)? as u16,
        error_message: row.get(21)?,
        created_at: row.get(22)?,
        data_source: row.get(23)?,
        pricing_model: row.get(24)?,
        input_token_semantics: row.get::<_, i64>(25)?,
        requested_reasoning_effort: row.get(26)?,
        applied_reasoning_effort: row.get(27)?,
        fresh_input_tokens: row.get::<_, i64>(28)? as u64,
        pricing_tier: row.get(29)?,
    })
}

/// Keep historical proxy rows identifiable after their provider is removed.
fn provider_name_coalesce(log_alias: &str, provider_alias: &str) -> String {
    format!("COALESCE({provider_alias}.name, {log_alias}.provider_id)")
}

/// SQL 片段：把指定别名的 `data_source` 包成 COALESCE，NULL 视作 'proxy'。
///
/// 防御 schema v9 之前可能写入的 NULL data_source 行（见
/// `tests::create_legacy_nullable_logs_table`）。所有用到 data_source 的查询
/// 都应通过此 helper 生成片段，避免遗漏。
fn data_source_expr(log_alias: &str) -> String {
    format!("COALESCE({log_alias}.data_source, 'proxy')")
}

/// SQL 片段：把日志/汇总行 LEFT JOIN 到 providers 表以取得供应商名称。
/// `proxy_request_logs` 与 `usage_daily_rollups` 的 (provider_id, app_type)
/// 形状相同，两者皆可作为 `log_alias`。providers 主键即 (id, app_type)，
/// 连接至多 1:1，不会放大行数。
fn providers_join(log_alias: &str, provider_alias: &str) -> String {
    format!(
        "LEFT JOIN providers {provider_alias} \
         ON {log_alias}.provider_id = {provider_alias}.id \
         AND {log_alias}.app_type = {provider_alias}.app_type"
    )
}

/// SQL 标量表达式：行的「有效计价模型」—— pricing_model 非空优先，NULL/'' 回落
/// model。这是 `get_model_stats` 的分组键，也是 Dashboard 模型筛选的匹配口径：
/// 筛选值来自模型统计列表，两边必须用同一表达式才能选得中。
fn effective_model_sql(alias: &str) -> String {
    format!("COALESCE(NULLIF({alias}.pricing_model, ''), {alias}.model)")
}

/// 把 Dashboard 顶部的 Provider/模型筛选追加到查询条件。
///
/// Provider 按展示名精确匹配（复用 [`provider_name_coalesce`]，会话占位行的
/// 可读名如 "_codex_session" 也能选中）；模型按 [`effective_model_sql`] 匹配。
/// 注意：传入 `provider_name` 时调用方必须把 [`providers_join`] 拼进 FROM，
/// 否则 `{provider_alias}.name` 无法解析。
fn push_provider_model_filters(
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::ToSql>>,
    log_alias: &str,
    provider_alias: &str,
    provider_name: Option<&str>,
    model: Option<&str>,
) {
    if let Some(name) = provider_name {
        conditions.push(format!(
            "{} = ?",
            provider_name_coalesce(log_alias, provider_alias)
        ));
        params.push(Box::new(name.to_string()));
    }
    if let Some(m) = model {
        conditions.push(format!("{} = ?", effective_model_sql(log_alias)));
        params.push(Box::new(m.to_string()));
    }
}

pub(crate) fn effective_usage_log_filter(log_alias: &str) -> String {
    // Imported conversation totals are not requests handled by Atlas. Keep the
    // historical rows on disk, but never mix them into proxy counts or cache rates.
    format!(
        "{log_alias}.app_type = 'codex' AND {} = 'proxy'",
        data_source_expr(log_alias)
    )
}

fn effective_usage_rollup_filter(alias: &str) -> String {
    // Legacy rollups predate a data_source column. Session importers used these
    // reserved provider IDs; real proxy rows retain their actual provider ID.
    format!(
        "{alias}.app_type = 'codex' AND {alias}.provider_id NOT IN ('_session', '_codex_session')"
    )
}

#[derive(Debug, Clone, Default)]
struct RollupDateBounds {
    start: Option<String>,
    end: Option<String>,
    is_empty: bool,
}

fn local_datetime_from_timestamp(ts: i64) -> Result<chrono::DateTime<Local>, AppError> {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .ok_or_else(|| AppError::Database(format!("无法解析本地时间戳: {ts}")))
}

fn compute_rollup_date_bounds(
    start_ts: Option<i64>,
    end_ts: Option<i64>,
) -> Result<RollupDateBounds, AppError> {
    let start = match start_ts {
        Some(ts) => {
            let local = local_datetime_from_timestamp(ts)?;
            let day = local.date_naive();
            if local.time().num_seconds_from_midnight() == 0 {
                Some(day.format("%Y-%m-%d").to_string())
            } else {
                day.succ_opt()
                    .map(|next| next.format("%Y-%m-%d").to_string())
            }
        }
        None => None,
    };

    let end = match end_ts {
        Some(ts) => {
            let local = local_datetime_from_timestamp(ts)?;
            let day = local.date_naive();
            if local.time().hour() == 23 && local.time().minute() == 59 {
                Some(day.format("%Y-%m-%d").to_string())
            } else {
                day.pred_opt()
                    .map(|prev| prev.format("%Y-%m-%d").to_string())
            }
        }
        None => None,
    };

    let is_empty = matches!((&start, &end), (Some(start), Some(end)) if start > end);

    Ok(RollupDateBounds {
        start,
        end,
        is_empty,
    })
}

fn push_rollup_date_filters(
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::ToSql>>,
    column: &str,
    bounds: &RollupDateBounds,
) {
    if bounds.is_empty {
        conditions.push("1 = 0".to_string());
        return;
    }

    if let Some(start) = &bounds.start {
        conditions.push(format!("{column} >= ?"));
        params.push(Box::new(start.clone()));
    }

    if let Some(end) = &bounds.end {
        conditions.push(format!("{column} <= ?"));
        params.push(Box::new(end.clone()));
    }
}

#[cfg(test)]
fn local_day_start_rfc3339(day: NaiveDate) -> String {
    let local_midnight = day
        .and_hms_opt(0, 0, 0)
        .and_then(|naive| match Local.from_local_datetime(&naive) {
            chrono::LocalResult::Single(dt) => Some(dt),
            chrono::LocalResult::Ambiguous(earliest, _) => Some(earliest),
            chrono::LocalResult::None => None,
        })
        .unwrap_or_else(Local::now);

    local_midnight.to_rfc3339()
}

impl Database {
    /// 获取使用量汇总
    pub fn get_usage_summary(
        &self,
        start_date: Option<i64>,
        end_date: Option<i64>,
        app_type: Option<&str>,
        provider_name: Option<&str>,
        model: Option<&str>,
    ) -> Result<UsageSummary, AppError> {
        let conn = lock_conn!(self.conn);

        // Build detail WHERE clause
        let mut conditions = vec![effective_usage_log_filter("l")];
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(start) = start_date {
            conditions.push("l.created_at >= ?".to_string());
            params_vec.push(Box::new(start));
        }
        if let Some(end) = end_date {
            conditions.push("l.created_at <= ?".to_string());
            params_vec.push(Box::new(end));
        }
        if let Some(at) = app_type {
            conditions.push("l.app_type = ?".to_string());
            params_vec.push(Box::new(at.to_string()));
        }
        push_provider_model_filters(
            &mut conditions,
            &mut params_vec,
            "l",
            "p",
            provider_name,
            model,
        );

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };
        let detail_join = if provider_name.is_some() {
            providers_join("l", "p")
        } else {
            String::new()
        };

        // Only include rolled-up rows for full local days that are fully covered by the range.
        let mut rollup_conditions = vec![effective_usage_rollup_filter("r")];
        let mut rollup_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let rollup_bounds = compute_rollup_date_bounds(start_date, end_date)?;

        push_rollup_date_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r.date",
            &rollup_bounds,
        );
        if let Some(at) = app_type {
            rollup_conditions.push("r.app_type = ?".to_string());
            rollup_params.push(Box::new(at.to_string()));
        }
        push_provider_model_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r",
            "p2",
            provider_name,
            model,
        );

        let rollup_where = if rollup_conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", rollup_conditions.join(" AND "))
        };
        let rollup_join = if provider_name.is_some() {
            providers_join("r", "p2")
        } else {
            String::new()
        };

        let fresh_input_detail = fresh_input_sql("l");
        let fresh_input_rollup = fresh_input_sql("r");
        let generation_ms = output_generation_ms_sql("l");
        let sql = format!(
            "SELECT
                COALESCE(d.total_requests, 0) + COALESCE(r.total_requests, 0),
                COALESCE(d.total_cost, 0) + COALESCE(r.total_cost, 0),
                COALESCE(d.total_input_tokens, 0) + COALESCE(r.total_input_tokens, 0),
                COALESCE(d.total_output_tokens, 0) + COALESCE(r.total_output_tokens, 0),
                COALESCE(d.total_cache_creation_tokens, 0) + COALESCE(r.total_cache_creation_tokens, 0),
                COALESCE(d.total_cache_read_tokens, 0) + COALESCE(r.total_cache_read_tokens, 0),
                COALESCE(d.success_count, 0) + COALESCE(r.success_count, 0),
                COALESCE(d.total_latency_ms, 0) + COALESCE(r.total_latency_ms, 0),
                COALESCE(d.timed_output_tokens, 0) + COALESCE(r.timed_output_tokens, 0),
                COALESCE(d.generation_ms, 0) + COALESCE(r.generation_ms, 0)
            FROM
                (SELECT
                    COUNT(*) as total_requests,
                    COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as total_cost,
                    COALESCE(SUM({fresh_input_detail}), 0) as total_input_tokens,
                    COALESCE(SUM(l.output_tokens), 0) as total_output_tokens,
                    COALESCE(SUM(l.cache_creation_tokens), 0) as total_cache_creation_tokens,
                    COALESCE(SUM(l.cache_read_tokens), 0) as total_cache_read_tokens,
                    COALESCE(SUM(CASE WHEN l.status_code >= 200 AND l.status_code < 300 THEN 1 ELSE 0 END), 0) as success_count,
                    COALESCE(SUM(CAST(l.latency_ms AS REAL)), 0) as total_latency_ms,
                    COALESCE(SUM(CASE WHEN l.output_tokens > 0 AND ({generation_ms}) > 0
                        THEN l.output_tokens ELSE 0 END), 0) as timed_output_tokens,
                    COALESCE(SUM(CASE WHEN l.output_tokens > 0 AND ({generation_ms}) > 0
                        THEN CAST(({generation_ms}) AS REAL) ELSE 0 END), 0) as generation_ms
                 FROM proxy_request_logs l {detail_join} {where_clause}) d,
                (SELECT
                    COALESCE(SUM(r.request_count), 0) as total_requests,
                    COALESCE(SUM(CAST(r.total_cost_usd AS REAL)), 0) as total_cost,
                    COALESCE(SUM({fresh_input_rollup}), 0) as total_input_tokens,
                    COALESCE(SUM(r.output_tokens), 0) as total_output_tokens,
                    COALESCE(SUM(r.cache_creation_tokens), 0) as total_cache_creation_tokens,
                    COALESCE(SUM(r.cache_read_tokens), 0) as total_cache_read_tokens,
                    COALESCE(SUM(r.success_count), 0) as success_count,
                    COALESCE(SUM(CAST(r.avg_latency_ms AS REAL) * r.request_count), 0) as total_latency_ms,
                    COALESCE(SUM(CASE WHEN r.output_tokens > 0 AND r.avg_latency_ms > 0 AND r.request_count > 0
                        THEN r.output_tokens ELSE 0 END), 0) as timed_output_tokens,
                    COALESCE(SUM(CASE WHEN r.output_tokens > 0 AND r.avg_latency_ms > 0 AND r.request_count > 0
                        THEN CAST(r.avg_latency_ms AS REAL) * r.request_count ELSE 0 END), 0) as generation_ms
                 FROM usage_daily_rollups r {rollup_join} {rollup_where}) r"
        );

        // Combine params: detail params first, then rollup params
        let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = params_vec;
        all_params.extend(rollup_params);
        let param_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

        let result = conn.query_row(&sql, param_refs.as_slice(), |row| {
            let total_requests: i64 = row.get(0)?;
            let total_cost: f64 = row.get(1)?;
            let total_input_tokens: i64 = row.get(2)?;
            let total_output_tokens: i64 = row.get(3)?;
            let total_cache_creation_tokens: i64 = row.get(4)?;
            let total_cache_read_tokens: i64 = row.get(5)?;
            let success_count: i64 = row.get(6)?;
            let total_latency_ms: f64 = row.get(7)?;
            let timed_output_tokens: i64 = row.get(8)?;
            let generation_ms: f64 = row.get(9)?;
            let output_tokens_per_second = (timed_output_tokens > 0 && generation_ms > 0.0)
                .then(|| timed_output_tokens as f64 * 1000.0 / generation_ms);
            let avg_latency_ms = if total_requests > 0 {
                total_latency_ms / total_requests as f64
            } else {
                0.0
            };

            let success_rate = if total_requests > 0 {
                (success_count as f32 / total_requests as f32) * 100.0
            } else {
                0.0
            };

            let (real_total_tokens, cache_hit_rate) = derive_real_total_and_hit_rate(
                total_input_tokens as u64,
                total_output_tokens as u64,
                total_cache_creation_tokens as u64,
                total_cache_read_tokens as u64,
            );

            Ok(UsageSummary {
                total_requests: total_requests as u64,
                total_cost: format!("{total_cost:.6}"),
                total_input_tokens: total_input_tokens as u64,
                total_output_tokens: total_output_tokens as u64,
                total_cache_creation_tokens: total_cache_creation_tokens as u64,
                total_cache_read_tokens: total_cache_read_tokens as u64,
                success_rate,
                avg_latency_ms,
                output_tokens_per_second,
                real_total_tokens,
                cache_hit_rate,
            })
        })?;

        Ok(result)
    }

    /// Legacy aggregation retained as a regression oracle for recorded totals.
    #[cfg(test)]
    pub fn get_daily_trends(
        &self,
        start_date: Option<i64>,
        end_date: Option<i64>,
        app_type: Option<&str>,
        provider_name: Option<&str>,
        model: Option<&str>,
    ) -> Result<Vec<DailyStats>, AppError> {
        let conn = lock_conn!(self.conn);

        let end_ts = end_date.unwrap_or_else(|| Local::now().timestamp());
        let mut start_ts = start_date.unwrap_or_else(|| end_ts - 24 * 60 * 60);

        if start_ts >= end_ts {
            start_ts = end_ts - 24 * 60 * 60;
        }

        let duration = end_ts - start_ts;
        if duration <= 24 * 60 * 60 {
            let bucket_seconds: i64 = 60 * 60;
            let mut bucket_count: i64 = if duration <= 0 {
                1
            } else {
                (duration + bucket_seconds - 1) / bucket_seconds
            };

            if bucket_count < 1 {
                bucket_count = 1;
            }

            let mut extra_conditions: Vec<String> = Vec::new();
            let mut extra_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(at) = app_type {
                extra_conditions.push("l.app_type = ?".to_string());
                extra_params.push(Box::new(at.to_string()));
            }
            push_provider_model_filters(
                &mut extra_conditions,
                &mut extra_params,
                "l",
                "p",
                provider_name,
                model,
            );
            let extra_filter = extra_conditions
                .iter()
                .map(|c| format!("AND {c}"))
                .collect::<Vec<_>>()
                .join(" ");
            let detail_join = if provider_name.is_some() {
                providers_join("l", "p")
            } else {
                String::new()
            };

            let effective_filter = effective_usage_log_filter("l");
            let fresh_input = fresh_input_sql("l");
            // The range includes end_ts. On an exact hour boundary, fold that
            // second into the last bucket before GROUP BY so it cannot replace
            // the rest of that hour when the query results are collected.
            let sql = format!(
                "SELECT
                    MIN(CAST((l.created_at - ?1) / ?3 AS INTEGER), ?4) as bucket_idx,
                    COUNT(*) as request_count,
                    COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as total_cost,
                    COALESCE(SUM({fresh_input} + l.output_tokens), 0) as total_tokens,
                    COALESCE(SUM({fresh_input}), 0) as total_input_tokens,
                    COALESCE(SUM(l.output_tokens), 0) as total_output_tokens,
                    COALESCE(SUM(l.cache_creation_tokens), 0) as total_cache_creation_tokens,
                    COALESCE(SUM(l.cache_read_tokens), 0) as total_cache_read_tokens
                FROM proxy_request_logs l {detail_join}
                WHERE l.created_at >= ?1 AND l.created_at <= ?2
                  AND {effective_filter} {extra_filter}
                GROUP BY bucket_idx
                ORDER BY bucket_idx ASC"
            );

            let mut stmt = conn.prepare(&sql)?;
            let row_mapper = |row: &rusqlite::Row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    DailyStats {
                        date: String::new(),
                        request_count: row.get::<_, i64>(1)? as u64,
                        total_cost: format!("{:.6}", row.get::<_, f64>(2)?),
                        total_tokens: row.get::<_, i64>(3)? as u64,
                        total_input_tokens: row.get::<_, i64>(4)? as u64,
                        total_output_tokens: row.get::<_, i64>(5)? as u64,
                        total_cache_creation_tokens: row.get::<_, i64>(6)? as u64,
                        total_cache_read_tokens: row.get::<_, i64>(7)? as u64,
                    },
                ))
            };

            let mut map: HashMap<i64, DailyStats> = HashMap::new();

            let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![
                Box::new(start_ts),
                Box::new(end_ts),
                Box::new(bucket_seconds),
                Box::new(bucket_count - 1),
            ];
            all_params.extend(extra_params);
            let param_refs: Vec<&dyn rusqlite::ToSql> =
                all_params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), row_mapper)?;
            for row in rows {
                let (bucket_idx, stat) = row?;
                map.insert(bucket_idx, stat);
            }

            let mut stats = Vec::with_capacity(bucket_count as usize);
            for i in 0..bucket_count {
                let bucket_start_ts = start_ts + i * bucket_seconds;
                let bucket_start = local_datetime_from_timestamp(bucket_start_ts)?;
                let date = bucket_start.to_rfc3339();

                if let Some(mut stat) = map.remove(&i) {
                    stat.date = date;
                    stats.push(stat);
                } else {
                    stats.push(DailyStats {
                        date,
                        request_count: 0,
                        total_cost: "0.000000".to_string(),
                        total_tokens: 0,
                        total_input_tokens: 0,
                        total_output_tokens: 0,
                        total_cache_creation_tokens: 0,
                        total_cache_read_tokens: 0,
                    });
                }
            }

            return Ok(stats);
        }

        let start_day = local_datetime_from_timestamp(start_ts)?.date_naive();
        let end_day = local_datetime_from_timestamp(end_ts)?.date_naive();
        let bucket_count = (end_day.signed_duration_since(start_day).num_days() + 1) as usize;

        let mut extra_conditions: Vec<String> = Vec::new();
        let mut extra_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(at) = app_type {
            extra_conditions.push("l.app_type = ?".to_string());
            extra_params.push(Box::new(at.to_string()));
        }
        push_provider_model_filters(
            &mut extra_conditions,
            &mut extra_params,
            "l",
            "p",
            provider_name,
            model,
        );
        let extra_filter = extra_conditions
            .iter()
            .map(|c| format!("AND {c}"))
            .collect::<Vec<_>>()
            .join(" ");
        let detail_join = if provider_name.is_some() {
            providers_join("l", "p")
        } else {
            String::new()
        };

        let effective_filter = effective_usage_log_filter("l");
        let fresh_input = fresh_input_sql("l");
        let detail_sql = format!(
            "SELECT
                date(l.created_at, 'unixepoch', 'localtime') as bucket_date,
                COUNT(*) as request_count,
                COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as total_cost,
                COALESCE(SUM({fresh_input} + l.output_tokens), 0) as total_tokens,
                COALESCE(SUM({fresh_input}), 0) as total_input_tokens,
                COALESCE(SUM(l.output_tokens), 0) as total_output_tokens,
                COALESCE(SUM(l.cache_creation_tokens), 0) as total_cache_creation_tokens,
                COALESCE(SUM(l.cache_read_tokens), 0) as total_cache_read_tokens
            FROM proxy_request_logs l {detail_join}
            WHERE l.created_at >= ?1 AND l.created_at <= ?2
              AND {effective_filter} {extra_filter}
            GROUP BY bucket_date
            ORDER BY bucket_date ASC"
        );

        let mut detail_stmt = conn.prepare(&detail_sql)?;
        let detail_row_mapper = |row: &rusqlite::Row| {
            Ok((
                row.get::<_, String>(0)?,
                DailyStats {
                    date: String::new(),
                    request_count: row.get::<_, i64>(1)? as u64,
                    total_cost: format!("{:.6}", row.get::<_, f64>(2)?),
                    total_tokens: row.get::<_, i64>(3)? as u64,
                    total_input_tokens: row.get::<_, i64>(4)? as u64,
                    total_output_tokens: row.get::<_, i64>(5)? as u64,
                    total_cache_creation_tokens: row.get::<_, i64>(6)? as u64,
                    total_cache_read_tokens: row.get::<_, i64>(7)? as u64,
                },
            ))
        };

        let mut map: HashMap<NaiveDate, DailyStats> = HashMap::new();
        let mut detail_all_params: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(start_ts), Box::new(end_ts)];
        detail_all_params.extend(extra_params);
        let detail_param_refs: Vec<&dyn rusqlite::ToSql> =
            detail_all_params.iter().map(|p| p.as_ref()).collect();
        let detail_rows = detail_stmt.query_map(detail_param_refs.as_slice(), detail_row_mapper)?;

        for row in detail_rows {
            let (bucket_date, stat) = row?;
            let date = NaiveDate::parse_from_str(&bucket_date, "%Y-%m-%d")
                .map_err(|err| AppError::Database(format!("解析趋势日期失败: {err}")))?;
            map.insert(date, stat);
        }

        let rollup_bounds = compute_rollup_date_bounds(Some(start_ts), Some(end_ts))?;
        let mut rollup_conditions = vec![effective_usage_rollup_filter("r")];
        let mut rollup_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        push_rollup_date_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r.date",
            &rollup_bounds,
        );
        if let Some(at) = app_type {
            rollup_conditions.push("r.app_type = ?".to_string());
            rollup_params.push(Box::new(at.to_string()));
        }
        push_provider_model_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r",
            "p2",
            provider_name,
            model,
        );

        let rollup_where = if rollup_conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", rollup_conditions.join(" AND "))
        };
        let rollup_join = if provider_name.is_some() {
            providers_join("r", "p2")
        } else {
            String::new()
        };

        let fresh_input_rollup = fresh_input_sql("r");
        let rollup_sql = format!(
            "SELECT
                r.date,
                COALESCE(SUM(r.request_count), 0),
                COALESCE(SUM(CAST(r.total_cost_usd AS REAL)), 0),
                COALESCE(SUM({fresh_input_rollup} + r.output_tokens), 0),
                COALESCE(SUM({fresh_input_rollup}), 0),
                COALESCE(SUM(r.output_tokens), 0),
                COALESCE(SUM(r.cache_creation_tokens), 0),
                COALESCE(SUM(r.cache_read_tokens), 0)
            FROM usage_daily_rollups r {rollup_join}
            {rollup_where}
            GROUP BY r.date
            ORDER BY r.date ASC"
        );

        let mut rollup_stmt = conn.prepare(&rollup_sql)?;
        let rollup_row_mapper = |row: &rusqlite::Row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, f64>(2)?,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, i64>(5)? as u64,
                    row.get::<_, i64>(6)? as u64,
                    row.get::<_, i64>(7)? as u64,
                ),
            ))
        };
        let rollup_param_refs: Vec<&dyn rusqlite::ToSql> =
            rollup_params.iter().map(|param| param.as_ref()).collect();
        let rollup_rows = rollup_stmt.query_map(rollup_param_refs.as_slice(), rollup_row_mapper)?;

        for row in rollup_rows {
            let (bucket_date, (req, cost, tok, inp, out, cc, cr)) = row?;
            let date = NaiveDate::parse_from_str(&bucket_date, "%Y-%m-%d")
                .map_err(|err| AppError::Database(format!("解析 rollup 趋势日期失败: {err}")))?;
            let entry = map.entry(date).or_insert_with(|| DailyStats {
                date: String::new(),
                request_count: 0,
                total_cost: "0.000000".to_string(),
                total_tokens: 0,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_creation_tokens: 0,
                total_cache_read_tokens: 0,
            });
            entry.request_count += req;
            let existing_cost: f64 = entry.total_cost.parse().unwrap_or(0.0);
            entry.total_cost = format!("{:.6}", existing_cost + cost);
            entry.total_tokens += tok;
            entry.total_input_tokens += inp;
            entry.total_output_tokens += out;
            entry.total_cache_creation_tokens += cc;
            entry.total_cache_read_tokens += cr;
        }

        let mut stats = Vec::with_capacity(bucket_count);
        let mut current_day = start_day;
        for _ in 0..bucket_count {
            let date = local_day_start_rfc3339(current_day);

            if let Some(mut stat) = map.remove(&current_day) {
                stat.date = date;
                stats.push(stat);
            } else {
                stats.push(DailyStats {
                    date,
                    request_count: 0,
                    total_cost: "0.000000".to_string(),
                    total_tokens: 0,
                    total_input_tokens: 0,
                    total_output_tokens: 0,
                    total_cache_creation_tokens: 0,
                    total_cache_read_tokens: 0,
                });
            }

            current_day = current_day.succ_opt().unwrap_or(current_day);
        }

        Ok(stats)
    }

    /// 获取 Provider 统计
    pub fn get_provider_stats(
        &self,
        start_date: Option<i64>,
        end_date: Option<i64>,
        app_type: Option<&str>,
        provider_name: Option<&str>,
        model: Option<&str>,
    ) -> Result<Vec<ProviderStats>, AppError> {
        let conn = lock_conn!(self.conn);

        let mut detail_conditions = vec![effective_usage_log_filter("l")];
        let mut detail_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(start) = start_date {
            detail_conditions.push("l.created_at >= ?".to_string());
            detail_params.push(Box::new(start));
        }
        if let Some(end) = end_date {
            detail_conditions.push("l.created_at <= ?".to_string());
            detail_params.push(Box::new(end));
        }
        if let Some(at) = app_type {
            detail_conditions.push("l.app_type = ?".to_string());
            detail_params.push(Box::new(at.to_string()));
        }
        push_provider_model_filters(
            &mut detail_conditions,
            &mut detail_params,
            "l",
            "p",
            provider_name,
            model,
        );
        let detail_where = if detail_conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", detail_conditions.join(" AND "))
        };

        let mut rollup_conditions = vec![effective_usage_rollup_filter("r")];
        let mut rollup_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let rollup_bounds = compute_rollup_date_bounds(start_date, end_date)?;
        push_rollup_date_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r.date",
            &rollup_bounds,
        );
        if let Some(at) = app_type {
            rollup_conditions.push("r.app_type = ?".to_string());
            rollup_params.push(Box::new(at.to_string()));
        }
        push_provider_model_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r",
            "p2",
            provider_name,
            model,
        );
        let rollup_where = if rollup_conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", rollup_conditions.join(" AND "))
        };

        // UNION detail logs + rollup data, then aggregate
        let detail_pname = provider_name_coalesce("l", "p");
        let rollup_pname = provider_name_coalesce("r", "p2");
        let fresh_input_detail = fresh_input_sql("l");
        let fresh_input_rollup = fresh_input_sql("r");
        let sql = format!(
            "SELECT
                provider_id, app_type, provider_name,
                SUM(request_count) as request_count,
                SUM(total_tokens) as total_tokens,
                SUM(total_cost) as total_cost,
                SUM(success_count) as success_count,
                CASE WHEN SUM(request_count) > 0
                    THEN SUM(latency_sum) / SUM(request_count)
                    ELSE 0 END as avg_latency
            FROM (
                SELECT l.provider_id, l.app_type,
                    {detail_pname} as provider_name,
                    COUNT(*) as request_count,
                    COALESCE(SUM({fresh_input_detail} + l.output_tokens), 0) as total_tokens,
                    COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as total_cost,
                    COALESCE(SUM(CASE WHEN l.status_code >= 200 AND l.status_code < 300 THEN 1 ELSE 0 END), 0) as success_count,
                    COALESCE(SUM(l.latency_ms), 0) as latency_sum
                FROM proxy_request_logs l
                LEFT JOIN providers p ON l.provider_id = p.id AND l.app_type = p.app_type
                {detail_where}
                GROUP BY l.provider_id, l.app_type
                UNION ALL
                SELECT r.provider_id, r.app_type,
                    {rollup_pname} as provider_name,
                    COALESCE(SUM(r.request_count), 0),
                    COALESCE(SUM({fresh_input_rollup} + r.output_tokens), 0),
                    COALESCE(SUM(CAST(r.total_cost_usd AS REAL)), 0),
                    COALESCE(SUM(r.success_count), 0),
                    COALESCE(SUM(r.avg_latency_ms * r.request_count), 0)
                FROM usage_daily_rollups r
                LEFT JOIN providers p2 ON r.provider_id = p2.id AND r.app_type = p2.app_type
                {rollup_where}
                GROUP BY r.provider_id, r.app_type
            )
            GROUP BY provider_id, app_type
            ORDER BY total_cost DESC"
        );

        let mut stmt = conn.prepare(&sql)?;
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = detail_params;
        params.extend(rollup_params);
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let row_mapper = |row: &rusqlite::Row| {
            let request_count: i64 = row.get(3)?;
            let success_count: i64 = row.get(6)?;
            let success_rate = if request_count > 0 {
                (success_count as f32 / request_count as f32) * 100.0
            } else {
                0.0
            };

            Ok(ProviderStats {
                provider_id: row.get(0)?,
                provider_name: row.get(2)?,
                request_count: request_count as u64,
                total_tokens: row.get::<_, i64>(4)? as u64,
                total_cost: format!("{:.6}", row.get::<_, f64>(5)?),
                success_rate,
                avg_latency_ms: row.get::<_, f64>(7)? as u64,
            })
        };

        let rows = stmt.query_map(param_refs.as_slice(), row_mapper)?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row?);
        }

        Ok(stats)
    }

    /// 获取模型统计
    pub fn get_model_stats(
        &self,
        start_date: Option<i64>,
        end_date: Option<i64>,
        app_type: Option<&str>,
        provider_name: Option<&str>,
        model: Option<&str>,
    ) -> Result<Vec<ModelStats>, AppError> {
        let conn = lock_conn!(self.conn);

        let mut detail_conditions = vec![effective_usage_log_filter("l")];
        let mut detail_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(start) = start_date {
            detail_conditions.push("l.created_at >= ?".to_string());
            detail_params.push(Box::new(start));
        }
        if let Some(end) = end_date {
            detail_conditions.push("l.created_at <= ?".to_string());
            detail_params.push(Box::new(end));
        }
        if let Some(at) = app_type {
            detail_conditions.push("l.app_type = ?".to_string());
            detail_params.push(Box::new(at.to_string()));
        }
        push_provider_model_filters(
            &mut detail_conditions,
            &mut detail_params,
            "l",
            "p",
            provider_name,
            model,
        );
        let detail_where = if detail_conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", detail_conditions.join(" AND "))
        };
        let detail_join = if provider_name.is_some() {
            providers_join("l", "p")
        } else {
            String::new()
        };

        let mut rollup_conditions = vec![effective_usage_rollup_filter("r")];
        let mut rollup_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let rollup_bounds = compute_rollup_date_bounds(start_date, end_date)?;
        push_rollup_date_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r.date",
            &rollup_bounds,
        );
        if let Some(at) = app_type {
            rollup_conditions.push("r.app_type = ?".to_string());
            rollup_params.push(Box::new(at.to_string()));
        }
        push_provider_model_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r",
            "p2",
            provider_name,
            model,
        );
        let rollup_where = if rollup_conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", rollup_conditions.join(" AND "))
        };
        let rollup_join = if provider_name.is_some() {
            providers_join("r", "p2")
        } else {
            String::new()
        };

        // UNION detail logs + rollup data
        //
        // 分组键用「有效计价模型」：pricing_model 非空时优先（成本就是按它的
        // 定价算的，金额与定价表自洽），NULL/'' 回落 model。默认 response 计价
        // 模式下两者相同，行为不变；request 模式 + 路由接管下，钱挂在实际计价
        // 基准名下，而不是上游回显/客户端别名名下。
        let fresh_input_detail = fresh_input_sql("l");
        let fresh_input_rollup = fresh_input_sql("r");
        let detail_model = effective_model_sql("l");
        let rollup_model = effective_model_sql("r");
        let sql = format!(
            "SELECT
                model,
                SUM(request_count) as request_count,
                SUM(total_tokens) as total_tokens,
                SUM(total_cost) as total_cost,
                SUM(fresh_input) as fresh_input,
                SUM(output_tokens) as output_tokens,
                SUM(cache_read) as cache_read,
                SUM(cache_write) as cache_write
            FROM (
                SELECT {detail_model} as model,
                    COUNT(*) as request_count,
                    COALESCE(SUM({fresh_input_detail} + l.output_tokens), 0) as total_tokens,
                    COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as total_cost,
                    COALESCE(SUM({fresh_input_detail}), 0) as fresh_input,
                    COALESCE(SUM(l.output_tokens), 0) as output_tokens,
                    COALESCE(SUM(l.cache_read_tokens), 0) as cache_read,
                    COALESCE(SUM(l.cache_creation_tokens), 0) as cache_write
                FROM proxy_request_logs l
                {detail_join}
                {detail_where}
                GROUP BY {detail_model}
                UNION ALL
                SELECT {rollup_model},
                    COALESCE(SUM(r.request_count), 0),
                    COALESCE(SUM({fresh_input_rollup} + r.output_tokens), 0),
                    COALESCE(SUM(CAST(r.total_cost_usd AS REAL)), 0),
                    COALESCE(SUM({fresh_input_rollup}), 0),
                    COALESCE(SUM(r.output_tokens), 0),
                    COALESCE(SUM(r.cache_read_tokens), 0),
                    COALESCE(SUM(r.cache_creation_tokens), 0)
                FROM usage_daily_rollups r
                {rollup_join}
                {rollup_where}
                GROUP BY {rollup_model}
            )
            GROUP BY model
            ORDER BY total_cost DESC"
        );

        let mut stmt = conn.prepare(&sql)?;
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = detail_params;
        params.extend(rollup_params);
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let row_mapper = |row: &rusqlite::Row| {
            let request_count: i64 = row.get(1)?;
            let total_cost: f64 = row.get(3)?;
            let avg_cost = if request_count > 0 {
                total_cost / request_count as f64
            } else {
                0.0
            };

            Ok(ModelStats {
                model: row.get(0)?,
                request_count: request_count as u64,
                total_tokens: row.get::<_, i64>(2)? as u64,
                total_input_tokens: row.get::<_, i64>(4)? as u64,
                total_output_tokens: row.get::<_, i64>(5)? as u64,
                total_cache_read_tokens: row.get::<_, i64>(6)? as u64,
                total_cache_creation_tokens: row.get::<_, i64>(7)? as u64,
                total_cost: format!("{total_cost:.6}"),
                avg_cost_per_request: format!("{avg_cost:.6}"),
            })
        };

        let rows = stmt.query_map(param_refs.as_slice(), row_mapper)?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row?);
        }

        Ok(stats)
    }

    /// Find model usage in the selected range that cannot be priced.
    ///
    /// Price matching uses the same aliases as historical cost backfill, so
    /// dated and reasoning-suffixed model IDs do not create false warnings.
    pub fn get_unpriced_model_usage(
        &self,
        start_date: Option<i64>,
        end_date: Option<i64>,
        app_type: Option<&str>,
        provider_name: Option<&str>,
        model: Option<&str>,
    ) -> Result<Vec<UnpricedModelUsage>, AppError> {
        let conn = lock_conn!(self.conn);

        let mut detail_conditions = vec![effective_usage_log_filter("l")];
        let mut detail_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(start) = start_date {
            detail_conditions.push("l.created_at >= ?".into());
            detail_params.push(Box::new(start));
        }
        if let Some(end) = end_date {
            detail_conditions.push("l.created_at <= ?".into());
            detail_params.push(Box::new(end));
        }
        if let Some(app_type) = app_type {
            detail_conditions.push("l.app_type = ?".into());
            detail_params.push(Box::new(app_type.to_string()));
        }
        push_provider_model_filters(
            &mut detail_conditions,
            &mut detail_params,
            "l",
            "p",
            provider_name,
            model,
        );
        let detail_where = format!("WHERE {}", detail_conditions.join(" AND "));
        let detail_join = if provider_name.is_some() {
            providers_join("l", "p")
        } else {
            String::new()
        };

        let mut rollup_conditions = vec![effective_usage_rollup_filter("r")];
        let mut rollup_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let rollup_bounds = compute_rollup_date_bounds(start_date, end_date)?;
        push_rollup_date_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r.date",
            &rollup_bounds,
        );
        if let Some(app_type) = app_type {
            rollup_conditions.push("r.app_type = ?".into());
            rollup_params.push(Box::new(app_type.to_string()));
        }
        push_provider_model_filters(
            &mut rollup_conditions,
            &mut rollup_params,
            "r",
            "p2",
            provider_name,
            model,
        );
        let rollup_where = format!("WHERE {}", rollup_conditions.join(" AND "));
        let rollup_join = if provider_name.is_some() {
            providers_join("r", "p2")
        } else {
            String::new()
        };

        let fresh_input_detail = fresh_input_sql("l");
        let fresh_input_rollup = fresh_input_sql("r");
        let detail_model = effective_model_sql("l");
        let rollup_model = effective_model_sql("r");
        let sql = format!(
            "SELECT model,
                    SUM(request_count),
                    SUM(fresh_input_tokens),
                    SUM(output_tokens),
                    SUM(cache_creation_tokens),
                    SUM(cache_read_tokens)
             FROM (
                SELECT {detail_model} AS model,
                    COUNT(*) AS request_count,
                    COALESCE(SUM({fresh_input_detail}), 0) AS fresh_input_tokens,
                    COALESCE(SUM(l.output_tokens), 0) AS output_tokens,
                    COALESCE(SUM(l.cache_creation_tokens), 0) AS cache_creation_tokens,
                    COALESCE(SUM(l.cache_read_tokens), 0) AS cache_read_tokens
                FROM proxy_request_logs l
                {detail_join}
                {detail_where}
                GROUP BY {detail_model}
                UNION ALL
                SELECT {rollup_model},
                    COALESCE(SUM(r.request_count), 0),
                    COALESCE(SUM({fresh_input_rollup}), 0),
                    COALESCE(SUM(r.output_tokens), 0),
                    COALESCE(SUM(r.cache_creation_tokens), 0),
                    COALESCE(SUM(r.cache_read_tokens), 0)
                FROM usage_daily_rollups r
                {rollup_join}
                {rollup_where}
                GROUP BY {rollup_model}
             )
             GROUP BY model
             HAVING SUM(fresh_input_tokens + output_tokens
                        + cache_creation_tokens + cache_read_tokens) > 0
             ORDER BY model"
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = detail_params;
        params.extend(rollup_params);
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(param_refs.as_slice(), |row| {
            let fresh_input_tokens = row.get::<_, i64>(2)? as u64;
            let output_tokens = row.get::<_, i64>(3)? as u64;
            let cache_creation_tokens = row.get::<_, i64>(4)? as u64;
            let cache_read_tokens = row.get::<_, i64>(5)? as u64;
            let cacheable_input = fresh_input_tokens + cache_creation_tokens + cache_read_tokens;

            Ok(UnpricedModelUsage {
                model: row.get(0)?,
                request_count: row.get::<_, i64>(1)? as u64,
                fresh_input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                cache_hit_rate: if cacheable_input > 0 {
                    cache_read_tokens as f64 / cacheable_input as f64
                } else {
                    0.0
                },
            })
        })?;
        let usage = rows.collect::<Result<Vec<_>, _>>()?;
        drop(statement);

        let mut unpriced = Vec::new();
        for model_usage in usage {
            if is_supported_pricing_model(&model_usage.model)
                && find_model_pricing_row(&conn, &model_usage.model)?.is_none()
            {
                unpriced.push(model_usage);
            }
        }
        Ok(unpriced)
    }

    /// 获取请求日志列表（分页）
    pub fn get_request_logs(
        &self,
        filters: &LogFilters,
        page: u32,
        page_size: u32,
    ) -> Result<PaginatedLogs, AppError> {
        let conn = lock_conn!(self.conn);

        let mut conditions = vec![effective_usage_log_filter("l")];
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref app_type) = filters.app_type {
            conditions.push("l.app_type = ?".to_string());
            params.push(Box::new(app_type.clone()));
        }
        // 与 Dashboard 顶部下拉筛选同口径：Provider 按展示名精确匹配（会话占位
        // 行如 "_codex_session" 也能命中），模型按有效计价模型匹配。
        push_provider_model_filters(
            &mut conditions,
            &mut params,
            "l",
            "p",
            filters.provider_name.as_deref(),
            filters.model.as_deref(),
        );
        if let Some(status) = filters.status_code {
            conditions.push("l.status_code = ?".to_string());
            params.push(Box::new(status as i64));
        }
        if let Some(start) = filters.start_date {
            conditions.push("l.created_at >= ?".to_string());
            params.push(Box::new(start));
        }
        if let Some(end) = filters.end_date {
            conditions.push("l.created_at <= ?".to_string());
            params.push(Box::new(end));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        // 获取总数
        let count_sql = format!(
            "SELECT COUNT(*) FROM proxy_request_logs l
             LEFT JOIN providers p ON l.provider_id = p.id AND l.app_type = p.app_type
             {where_clause}"
        );
        let count_params: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let total: u32 = conn.query_row(&count_sql, count_params.as_slice(), |row| {
            row.get::<_, i64>(0).map(|v| v as u32)
        })?;

        // 获取数据
        let offset = page * page_size;
        params.push(Box::new(page_size as i64));
        params.push(Box::new(offset as i64));

        let logs_pname = provider_name_coalesce("l", "p");
        let fresh_input = fresh_input_sql("l");
        let sql = format!(
            "SELECT l.request_id, l.provider_id, {logs_pname} as provider_name, l.app_type, l.model,
                    l.request_model, l.cost_multiplier,
                    l.input_tokens, l.output_tokens, l.cache_read_tokens, l.cache_creation_tokens,
                    l.input_cost_usd, l.output_cost_usd, l.cache_read_cost_usd, l.cache_creation_cost_usd, l.total_cost_usd,
                    l.is_streaming, l.latency_ms, l.first_token_ms, l.duration_ms,
                    l.status_code, l.error_message, l.created_at, l.data_source, l.pricing_model,
                    l.input_token_semantics, l.requested_reasoning_effort, l.applied_reasoning_effort,
                    {fresh_input} AS fresh_input_tokens, l.pricing_tier
             FROM proxy_request_logs l
             LEFT JOIN providers p ON l.provider_id = p.id AND l.app_type = p.app_type
             {where_clause}
             ORDER BY l.created_at DESC
             LIMIT ? OFFSET ?"
        );

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(params_refs.as_slice(), row_to_request_log_detail)?;

        let mut logs = Vec::new();
        let mut pricing_cache = HashMap::new();

        for row in rows {
            let mut log = row?;
            Self::maybe_backfill_log_costs(&conn, &mut log, &mut pricing_cache)?;
            logs.push(log);
        }

        Ok(PaginatedLogs {
            data: logs,
            total,
            page,
            page_size,
        })
    }
}

#[derive(Clone)]
struct PricingInfo {
    tier: &'static str,
    input: rust_decimal::Decimal,
    output: rust_decimal::Decimal,
    cache_read: rust_decimal::Decimal,
    cache_creation: rust_decimal::Decimal,
}

impl Database {
    /// Recalculate stored zero-cost usage rows once pricing becomes available.
    pub(crate) fn backfill_missing_usage_costs(&self) -> Result<u64, AppError> {
        let conn = lock_conn!(self.conn);
        Self::backfill_missing_usage_costs_on_conn(&conn, None)
    }

    /// 仅回填指定 model_id 相关的零成本行；用于单条定价更新后的精准回填。
    pub(crate) fn backfill_missing_usage_costs_for_model(
        &self,
        model_id: &str,
    ) -> Result<u64, AppError> {
        let conn = lock_conn!(self.conn);
        Self::backfill_missing_usage_costs_on_conn(&conn, Some(model_id))
    }

    pub(crate) fn backfill_missing_usage_costs_on_conn(
        conn: &Connection,
        only_model_id: Option<&str>,
    ) -> Result<u64, AppError> {
        let fresh_input = fresh_input_sql("");
        let sql = format!(
            "SELECT request_id, provider_id, NULL AS provider_name, app_type, model, request_model,
                        cost_multiplier,
                        input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                        input_cost_usd, output_cost_usd, cache_read_cost_usd,
                        cache_creation_cost_usd, total_cost_usd, is_streaming, latency_ms,
                        first_token_ms, duration_ms, status_code, error_message, created_at,
                        data_source, pricing_model, input_token_semantics,
                        requested_reasoning_effort, applied_reasoning_effort,
                        {fresh_input} AS fresh_input_tokens, pricing_tier
             FROM proxy_request_logs
             WHERE app_type = 'codex' AND COALESCE(data_source, 'proxy') = 'proxy'
               AND CAST(total_cost_usd AS REAL) <= 0
               AND pricing_tier IS NULL
               AND (input_tokens > 0 OR output_tokens > 0
                    OR cache_read_tokens > 0 OR cache_creation_tokens > 0)"
        );

        let mut logs = {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], row_to_request_log_detail)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        // Use the same GPT date/reasoning aliases as price lookup when deciding
        // which missing costs a single model-price update can fill.
        if let Some(model_id) = only_model_id {
            let target = model_pricing_candidates(model_id);
            logs.retain(|log| log_pricing_scope_matches(log, &target));
        }

        if logs.is_empty() {
            return Ok(0);
        }

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| AppError::Database(format!("启动用量成本回填事务失败: {e}")))?;

        let mut updated = 0u64;
        let mut pricing_cache = HashMap::new();
        for log in &mut logs {
            if Self::maybe_backfill_log_costs(&tx, log, &mut pricing_cache)? {
                updated += 1;
            }
        }
        tx.commit()
            .map_err(|e| AppError::Database(format!("提交用量成本回填事务失败: {e}")))?;

        if updated > 0 {
            log::info!("已回填 {updated} 条缺失的用量成本");
        }

        Ok(updated)
    }

    /// 尝试为单条 log 回填成本字段。返回是否实际写入（true=已 UPDATE，false=跳过）。
    fn maybe_backfill_log_costs(
        conn: &Connection,
        log: &mut RequestLogDetail,
        pricing_cache: &mut HashMap<String, PricingInfo>,
    ) -> Result<bool, AppError> {
        let existing_cost = rust_decimal::Decimal::from_str(&log.total_cost_usd)
            .unwrap_or(rust_decimal::Decimal::ZERO);
        let has_cost = existing_cost > rust_decimal::Decimal::ZERO;
        let has_usage = log.input_tokens > 0
            || log.output_tokens > 0
            || log.cache_read_tokens > 0
            || log.cache_creation_tokens > 0;

        if has_cost || log.pricing_tier.is_some() || !has_usage {
            return Ok(false);
        }

        let pricing = match Self::get_log_model_pricing_cached(conn, pricing_cache, log)? {
            Some(info) => info,
            None => return Ok(false),
        };
        let multiplier =
            rust_decimal::Decimal::from_str(&log.cost_multiplier).unwrap_or_else(|e| {
                log::warn!(
                    "历史用量倍率解析失败 request_id={}: {} - {e}",
                    log.request_id,
                    log.cost_multiplier
                );
                rust_decimal::Decimal::ONE
            });

        let million = rust_decimal::Decimal::from(1_000_000u64);

        // Explicit fresh-input rows need no deduction. Legacy Codex totals
        // include cache reads; current totals also include cache creation.
        let billable_input_tokens = if log.input_token_semantics == INPUT_TOKEN_SEMANTICS_FRESH {
            log.input_tokens as u64
        } else if log.input_token_semantics == INPUT_TOKEN_SEMANTICS_TOTAL {
            (log.input_tokens as u64)
                .saturating_sub(log.cache_read_tokens as u64)
                .saturating_sub(log.cache_creation_tokens as u64)
        } else {
            // v12 and earlier: input included cache reads but excluded cache writes.
            (log.input_tokens as u64).saturating_sub(log.cache_read_tokens as u64)
        };
        let input_cost =
            rust_decimal::Decimal::from(billable_input_tokens) * pricing.input / million;
        let output_cost =
            rust_decimal::Decimal::from(log.output_tokens as u64) * pricing.output / million;
        let cache_read_cost = rust_decimal::Decimal::from(log.cache_read_tokens as u64)
            * pricing.cache_read
            / million;
        let cache_creation_cost = rust_decimal::Decimal::from(log.cache_creation_tokens as u64)
            * pricing.cache_creation
            / million;
        // 总成本 = 基础成本之和 × 倍率
        let base_total = input_cost + output_cost + cache_read_cost + cache_creation_cost;
        let total_cost = base_total * multiplier;

        log.input_cost_usd = format!("{input_cost:.6}");
        log.output_cost_usd = format!("{output_cost:.6}");
        log.cache_read_cost_usd = format!("{cache_read_cost:.6}");
        log.cache_creation_cost_usd = format!("{cache_creation_cost:.6}");
        log.total_cost_usd = format!("{total_cost:.6}");
        log.pricing_tier = Some(pricing.tier.to_string());

        conn.execute(
            "UPDATE proxy_request_logs
             SET input_cost_usd = ?1,
                 output_cost_usd = ?2,
                 cache_read_cost_usd = ?3,
                 cache_creation_cost_usd = ?4,
                 total_cost_usd = ?5,
                 pricing_tier = ?7
             WHERE request_id = ?6",
            params![
                log.input_cost_usd,
                log.output_cost_usd,
                log.cache_read_cost_usd,
                log.cache_creation_cost_usd,
                log.total_cost_usd,
                log.request_id,
                log.pricing_tier
            ],
        )
        .map_err(|e| AppError::Database(format!("更新请求成本失败: {e}")))?;

        Ok(true)
    }

    fn get_model_pricing_cached(
        conn: &Connection,
        cache: &mut HashMap<String, PricingInfo>,
        model: &str,
        total_input: u64,
    ) -> Result<Option<PricingInfo>, AppError> {
        let key = format!("{model}:{total_input}");
        if let Some(info) = cache.get(&key) {
            return Ok(Some(info.clone()));
        }

        let row = find_selected_model_pricing(conn, model, total_input)?;
        let Some(SelectedPricing {
            rates: (input, output, cache_read, cache_creation),
            tier,
        }) = row
        else {
            return Ok(None);
        };

        let pricing = PricingInfo {
            tier,
            input: rust_decimal::Decimal::from_str(&input)
                .map_err(|e| AppError::Database(format!("解析输入价格失败: {e}")))?,
            output: rust_decimal::Decimal::from_str(&output)
                .map_err(|e| AppError::Database(format!("解析输出价格失败: {e}")))?,
            cache_read: rust_decimal::Decimal::from_str(&cache_read)
                .map_err(|e| AppError::Database(format!("解析缓存读取价格失败: {e}")))?,
            cache_creation: rust_decimal::Decimal::from_str(&cache_creation)
                .map_err(|e| AppError::Database(format!("解析缓存写入价格失败: {e}")))?,
        };

        cache.insert(key, pricing.clone());
        Ok(Some(pricing))
    }

    fn get_log_model_pricing_cached(
        conn: &Connection,
        cache: &mut HashMap<String, PricingInfo>,
        log: &RequestLogDetail,
    ) -> Result<Option<PricingInfo>, AppError> {
        let total_input = log.fresh_input_tokens as u64
            + log.cache_read_tokens as u64
            + log.cache_creation_tokens as u64;
        // 写入时的计价基准已落库（v11+）：回填只按它重算，找不到就保持 0 成本
        // 等补价。不能换用 model/request_model 猜——路由接管 + request 计价模式下
        // 三者可能各不相同（model=上游回显、request_model=客户端别名、
        // pricing_model=实际出站模型），换基准会按错误价格永久固化。
        // 占位符（"" = 未计价错误行 / "unknown"）视同缺失，走历史行逻辑。
        if let Some(pricing_model) = log
            .pricing_model
            .as_deref()
            .filter(|pm| !is_placeholder_pricing_model(pm))
        {
            return Self::get_model_pricing_cached(conn, cache, pricing_model, total_input);
        }

        if let Some(pricing) = Self::get_model_pricing_cached(conn, cache, &log.model, total_input)?
        {
            return Ok(Some(pricing));
        }

        // 仅当 model 列是占位符（解析失败留下的 ""/"unknown" 等）时才回退到
        // request_model 定价。model 是真实模型名但缺定价时必须保持 0 成本等待
        // 补价：路由接管下 request_model 是客户端别名（如 gpt-6-astra），
        // 按别名回填会把真实上游模型的 tokens 按错误价格永久固化（行一旦有成本
        // 就不再进入回填范围）。
        if !is_placeholder_pricing_model(&log.model) {
            return Ok(None);
        }

        let Some(request_model) = log.request_model.as_deref() else {
            return Ok(None);
        };
        if request_model == log.model {
            return Ok(None);
        }

        Self::get_model_pricing_cached(conn, cache, request_model, total_input)
    }
}

pub(crate) fn find_model_pricing_row(
    conn: &Connection,
    model_id: &str,
) -> Result<Option<(String, String, String, String)>, AppError> {
    find_model_pricing_for_input(conn, model_id, 0)
}

pub(crate) fn find_model_pricing_for_input(
    conn: &Connection,
    model_id: &str,
    total_input: u64,
) -> Result<Option<(String, String, String, String)>, AppError> {
    Ok(find_selected_model_pricing(conn, model_id, total_input)?.map(|pricing| pricing.rates))
}

pub(crate) struct SelectedPricing {
    pub rates: (String, String, String, String),
    pub tier: &'static str,
}

pub(crate) fn find_selected_model_pricing(
    conn: &Connection,
    model_id: &str,
    total_input: u64,
) -> Result<Option<SelectedPricing>, AppError> {
    let candidates = model_pricing_candidates(model_id);
    if candidates.is_empty() {
        return Ok(None);
    }

    for candidate in &candidates {
        if let Some(row) = query_model_pricing_exact(conn, candidate)? {
            let tier: Option<String> = conn.query_row(
                "SELECT long_context FROM model_pricing WHERE model_id = ?1",
                [candidate],
                |row| row.get(0),
            )?;
            if let Some(tier) = tier {
                let tier: super::model_pricing::LongContextPricing = serde_json::from_str(&tier)
                    .map_err(|e| {
                        AppError::Database(format!("Invalid long-context pricing: {e}"))
                    })?;
                if total_input > tier.threshold_input_tokens {
                    return Ok(Some(SelectedPricing {
                        tier: super::model_pricing::LONG_CONTEXT_PRICING_TIER,
                        rates: (
                            tier.input_cost_per_million,
                            tier.output_cost_per_million,
                            tier.cache_read_cost_per_million,
                            tier.cache_creation_cost_per_million,
                        ),
                    }));
                }
            }
            return Ok(Some(SelectedPricing {
                rates: row,
                tier: super::model_pricing::DEFAULT_PRICING_TIER,
            }));
        }
    }

    Ok(None)
}

/// Match the same canonical aliases used by exact price lookup.
fn log_pricing_scope_matches(log: &RequestLogDetail, target_candidates: &[String]) -> bool {
    [
        Some(log.model.as_str()),
        log.request_model.as_deref(),
        log.pricing_model.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|field| {
        model_pricing_candidates(field)
            .iter()
            .any(|candidate| target_candidates.iter().any(|target| target == candidate))
    })
}

pub(crate) fn is_placeholder_pricing_model(model_id: &str) -> bool {
    let normalized = model_id.trim().to_ascii_lowercase();
    normalized.is_empty() || matches!(normalized.as_str(), "unknown" | "null" | "none")
}

pub(crate) fn is_supported_pricing_model(model_id: &str) -> bool {
    !model_pricing_candidates(model_id).is_empty()
}

fn query_model_pricing_exact(
    conn: &Connection,
    model_id: &str,
) -> Result<Option<(String, String, String, String)>, AppError> {
    conn.query_row(
        "SELECT input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
         FROM model_pricing
         WHERE model_id = ?1",
        [model_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )
    .optional()
    .map_err(|e| AppError::Database(format!("查询模型定价失败: {e}")))
}

fn model_pricing_candidates(model_id: &str) -> Vec<String> {
    let cleaned = clean_model_id_for_pricing(model_id);
    if is_placeholder_pricing_model(&cleaned)
        || !crate::proxy::providers::copilot_model_map::is_valid_model_id(&cleaned)
    {
        return Vec::new();
    }

    let exact = model_id.trim().to_ascii_lowercase();
    let mut candidates = if exact == cleaned {
        Vec::new()
    } else {
        vec![exact]
    };
    let mut queue = vec![cleaned];

    while let Some(candidate) = queue.pop() {
        if !push_unique_candidate(&mut candidates, candidate.clone()) {
            continue;
        }

        if let Some(stripped) = strip_model_date_suffix(&candidate) {
            queue.push(stripped);
        }
        // Only legacy GPT usage names encode the effort in the model ID.
        // A new vendor's "-high" model may be a distinct, differently priced SKU.
        if candidate.starts_with("gpt-") {
            if let Some(stripped) = strip_reasoning_effort_suffix(&candidate) {
                queue.push(stripped);
            }
        }
    }

    candidates
}

fn clean_model_id_for_pricing(model_id: &str) -> String {
    let normalized = model_id.trim().to_ascii_lowercase().replace('@', "-");
    normalized
        .strip_prefix("openai/")
        .filter(|id| id.starts_with("gpt-"))
        .unwrap_or(&normalized)
        .to_string()
}

fn push_unique_candidate(candidates: &mut Vec<String>, candidate: String) -> bool {
    if candidate.is_empty() || candidates.iter().any(|existing| existing == &candidate) {
        return false;
    }
    candidates.push(candidate);
    true
}

fn strip_model_date_suffix(model_id: &str) -> Option<String> {
    let bytes = model_id.as_bytes();
    if bytes.len() > 11 {
        let start = bytes.len() - 11;
        let suffix = &bytes[start..];
        let is_iso_date = suffix[0] == b'-'
            && suffix[1..5].iter().all(|b| b.is_ascii_digit())
            && suffix[5] == b'-'
            && suffix[6..8].iter().all(|b| b.is_ascii_digit())
            && suffix[8] == b'-'
            && suffix[9..11].iter().all(|b| b.is_ascii_digit());
        if is_iso_date {
            return Some(model_id[..start].to_string());
        }
    }

    let (base, suffix) = model_id.rsplit_once('-')?;
    if base.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // OpenAI date aliases also use YYYYMMDD.
    if suffix.len() == 8 {
        return Some(base.to_string());
    }
    None
}

fn strip_reasoning_effort_suffix(model_id: &str) -> Option<String> {
    for suffix in [
        "-none", "-minimal", "-low", "-medium", "-high", "-xhigh", "-max", "-ultra",
    ] {
        if let Some(stripped) = model_id.strip_suffix(suffix) {
            if !stripped.is_empty() {
                return Some(stripped.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_pricing_reconciles_request_model_summary_and_trend_costs() -> Result<(), AppError> {
        use crate::proxy::{
            types::ReasoningEffort,
            usage::{logger::UsageLogger, parser::TokenUsage},
        };
        use rust_decimal::Decimal;

        let db = Database::memory()?;
        let logger = UsageLogger::new(&db);
        let tiers = Database::bundled_long_context_prices()?;
        let mut requests = HashMap::new();
        let mut models = HashMap::new();
        let mut grand_total = Decimal::ZERO;
        let mut grand_tokens = [0_u64; 4];
        let million = Decimal::from(1_000_000);
        // Aggregate API strings intentionally round to six decimal places.
        let assert_aggregate = |actual: &str, expected: Decimal| {
            let actual = Decimal::from_str(actual).unwrap();
            assert!(
                (actual - expected).abs() <= Decimal::new(5, 7),
                "aggregate {actual} differs from expected {expected}"
            );
        };
        for [model, _, input, output, read, write] in Database::bundled_model_prices()? {
            let tier = tiers.get(&model);
            let threshold = tier
                .map(|tier| tier.threshold_input_tokens)
                .unwrap_or(272000) as u32;
            let mut model_total = Decimal::ZERO;
            let mut model_tokens = [0_u64; 4];
            let cases = [
                ("below", 100, threshold - 201, 100, 1_000_000, "1"),
                ("boundary", 100, threshold - 200, 100, 1_000_000, "1"),
                ("read-crosses", 100, threshold - 199, 100, 20, "1.5"),
                ("write-crosses", 100, 0, threshold - 99, 20, "1"),
                ("zero-multiplier", threshold + 1, 0, 0, 20, "0"),
                ("output-only", 0, 0, 0, 1_000_000, "1"),
            ];
            for (case, fresh, cached, written, output_tokens, multiplier) in cases {
                let total_input = fresh + cached + written;
                let long = tier.filter(|tier| u64::from(total_input) > tier.threshold_input_tokens);
                let (expected_tier, rates) = if let Some(tier) = long {
                    (
                        "long_context",
                        [
                            &tier.input_cost_per_million,
                            &tier.output_cost_per_million,
                            &tier.cache_read_cost_per_million,
                            &tier.cache_creation_cost_per_million,
                        ],
                    )
                } else {
                    ("default", [&input, &output, &read, &write])
                };
                // Independent arithmetic from the published rate fixture, not the production calculator.
                let tokens = [fresh, output_tokens, cached, written];
                let components = std::array::from_fn::<_, 4, _>(|index| {
                    Decimal::from(tokens[index]) * Decimal::from_str(rates[index]).unwrap()
                        / million
                });
                let total = components.iter().copied().sum::<Decimal>()
                    * Decimal::from_str(multiplier).unwrap();
                let id = format!("{model}:{case}");
                logger.log_with_calculation(
                    id.clone(),
                    "copilot".into(),
                    "codex".into(),
                    format!("{model}-upstream"),
                    format!("{model}@low"),
                    model.clone(),
                    TokenUsage {
                        input_tokens: total_input,
                        output_tokens,
                        cache_read_tokens: cached,
                        cache_creation_tokens: written,
                        model: None,
                        message_id: None,
                    },
                    Decimal::from_str(multiplier).unwrap(),
                    100,
                    None,
                    200,
                    None,
                    None,
                    false,
                    ReasoningEffort::default(),
                )?;
                requests.insert(id, (expected_tier, components, total));
                model_total += total;
                for (index, count) in tokens.into_iter().enumerate() {
                    model_tokens[index] += u64::from(count);
                    grand_tokens[index] += u64::from(count);
                }
            }
            grand_total += model_total;
            models.insert(model, (model_total, model_tokens, cases.len() as u64));
        }
        assert!(!models.is_empty());
        let logs = db.get_request_logs(&LogFilters::default(), 0, requests.len() as u32)?;
        assert_eq!(logs.total as usize, requests.len());
        for log in logs.data {
            let (tier, components, total) = &requests[&log.request_id];
            assert_eq!(
                log.pricing_tier.as_deref(),
                Some(*tier),
                "{}",
                log.request_id
            );
            assert_eq!(
                Decimal::from_str(&log.total_cost_usd).unwrap(),
                *total,
                "{}",
                log.request_id
            );
            for (actual, expected) in [
                &log.input_cost_usd,
                &log.output_cost_usd,
                &log.cache_read_cost_usd,
                &log.cache_creation_cost_usd,
            ]
            .into_iter()
            .zip(components)
            {
                assert_eq!(
                    Decimal::from_str(actual).unwrap(),
                    *expected,
                    "{}",
                    log.request_id
                );
            }
        }
        let stats = db.get_model_stats(None, None, Some("codex"), None, None)?;
        assert_eq!(stats.len(), models.len());
        for stat in stats {
            let (total, tokens, count) = models[&stat.model];
            assert_eq!(stat.request_count, count);
            assert_eq!(
                [
                    stat.total_input_tokens,
                    stat.total_output_tokens,
                    stat.total_cache_read_tokens,
                    stat.total_cache_creation_tokens
                ],
                tokens
            );
            assert_aggregate(&stat.total_cost, total);
            assert_aggregate(&stat.avg_cost_per_request, total / Decimal::from(count));
            let filtered =
                db.get_usage_summary(None, None, Some("codex"), None, Some(&stat.model))?;
            assert_eq!(filtered.total_requests, count);
            assert_aggregate(&filtered.total_cost, total);
        }
        let summary = db.get_usage_summary(None, None, Some("codex"), None, None)?;
        assert_eq!(summary.total_requests as usize, requests.len());
        assert_eq!(
            [
                summary.total_input_tokens,
                summary.total_output_tokens,
                summary.total_cache_read_tokens,
                summary.total_cache_creation_tokens
            ],
            grand_tokens
        );
        assert_aggregate(&summary.total_cost, grand_total);
        let providers = db.get_provider_stats(None, None, Some("codex"), None, None)?;
        assert_eq!(providers.len(), 1);
        assert_aggregate(&providers[0].total_cost, grand_total);
        let trends = db.get_daily_trends(None, None, Some("codex"), None, None)?;
        let trend_total: Decimal = trends
            .iter()
            .map(|day| Decimal::from_str(&day.total_cost).unwrap())
            .sum();
        assert_aggregate(&trend_total.to_string(), grand_total);
        Ok(())
    }

    #[test]
    fn long_context_rates_are_strictly_above_the_documented_input_threshold() -> Result<(), AppError>
    {
        let db = Database::memory()?;
        let conn = lock_conn!(db.conn);
        let tiers = Database::bundled_long_context_prices()?;
        assert_eq!(tiers.len(), 11);
        for (model, tier) in tiers {
            let base = find_model_pricing_row(&conn, &model)?.unwrap();
            for total in [tier.threshold_input_tokens - 1, tier.threshold_input_tokens] {
                assert_eq!(
                    find_model_pricing_for_input(&conn, &model, total)?.unwrap(),
                    base,
                    "{model}"
                );
            }
            let long =
                find_model_pricing_for_input(&conn, &model, tier.threshold_input_tokens + 1)?
                    .unwrap();
            assert_eq!(
                long,
                (
                    tier.input_cost_per_million,
                    tier.output_cost_per_million,
                    tier.cache_read_cost_per_million,
                    tier.cache_creation_cost_per_million
                ),
                "{model}"
            );
        }
        assert_eq!(
            find_model_pricing_for_input(&conn, "OpenAI/GPT-6-ASTRA@ULTRA", 272001)?,
            find_model_pricing_for_input(&conn, "gpt-6-astra", 272001)?,
        );
        assert_eq!(
            find_model_pricing_for_input(&conn, "gemini-3.8-flash", 1_000_000)?,
            find_model_pricing_row(&conn, "gemini-3.8-flash")?,
        );
        Ok(())
    }

    #[test]
    fn atlas_usage_excludes_imported_details_and_rollups_without_erasing_them(
    ) -> Result<(), AppError> {
        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        let yesterday = (chrono::Local::now().date_naive() - chrono::Days::new(1)).to_string();
        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs
                 (request_id, provider_id, app_type, model, input_tokens, output_tokens,
                  cache_read_tokens, cache_creation_tokens, input_token_semantics, status_code, latency_ms, created_at, data_source)
                 VALUES ('atlas-live', 'copilot', 'codex', 'gpt-6-astra', 1000, 20, 800, 100, 1, 200, 100, ?1, 'proxy'),
                        ('old-import', '_codex_session', 'codex', 'old-model', 100000, 5000, 90000, 0, 0, 200, 100, ?1, 'codex_session')",
                [now],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (date, app_type, provider_id, model, request_count, success_count, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms)
                 VALUES (?1, 'codex', '_codex_session', 'old-model', 999, 999, 5000000, 5000, 4000000, 0, '100', 100)",
                [&yesterday],
            )?;
        }
        let summary = db.get_usage_summary(None, None, Some("codex"), None, None)?;
        assert_eq!(summary.total_requests, 1);
        assert_eq!(summary.total_input_tokens, 100);
        assert_eq!(summary.real_total_tokens, 1020);
        assert!((summary.cache_hit_rate - 0.8).abs() < 0.0001);
        let models = db.get_model_stats(None, None, Some("codex"), None, None)?;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model, "gpt-6-astra");
        let providers = db.get_provider_stats(None, None, Some("codex"), None, None)?;
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].request_count, 1);
        let logs = db.get_request_logs(
            &LogFilters {
                app_type: Some("codex".into()),
                ..Default::default()
            },
            0,
            20,
        )?;
        assert_eq!(logs.total, 1);
        let conn = lock_conn!(db.conn);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM proxy_request_logs WHERE data_source = 'codex_session'",
                [],
                |row| row.get::<_, i64>(0)
            )?,
            1
        );
        assert_eq!(conn.query_row("SELECT request_count FROM usage_daily_rollups WHERE provider_id = '_codex_session'", [], |row| row.get::<_, i64>(0))?, 999);
        Ok(())
    }

    fn local_ts(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> i64 {
        match Local.with_ymd_and_hms(year, month, day, hour, minute, second) {
            chrono::LocalResult::Single(dt) => dt.timestamp(),
            chrono::LocalResult::Ambiguous(earliest, _) => earliest.timestamp(),
            chrono::LocalResult::None => panic!("valid local datetime"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_usage_log(
        conn: &Connection,
        request_id: &str,
        app_type: &str,
        provider_id: &str,
        model: &str,
        data_source: &str,
        created_at: i64,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        status_code: i64,
        total_cost_usd: &str,
    ) -> Result<(), AppError> {
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, provider_id, app_type, model, request_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd, latency_ms, status_code, created_at, data_source
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, '0', '0', '0', '0', ?, 100, ?, ?, ?)",
            params![
                request_id,
                provider_id,
                app_type,
                model,
                model,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                total_cost_usd,
                status_code,
                created_at,
                data_source
            ],
        )?;
        Ok(())
    }

    fn create_legacy_nullable_logs_table(conn: &Connection) -> Result<(), AppError> {
        conn.execute(
            "CREATE TABLE proxy_request_logs (
                request_id TEXT PRIMARY KEY,
                app_type TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                cache_read_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL,
                status_code INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                data_source TEXT
            )",
            [],
        )?;
        Ok(())
    }

    #[test]
    fn test_effective_filter_keeps_legacy_null_data_source_proxy_rows() -> Result<(), AppError> {
        let conn = Connection::open_in_memory()?;
        create_legacy_nullable_logs_table(&conn)?;
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, app_type, model, input_tokens, output_tokens,
                cache_read_tokens, cache_creation_tokens, status_code, created_at, data_source
            ) VALUES ('legacy-proxy', 'codex', 'gpt-5.5', 10, 2, 1, 0, 200, 1000, NULL)",
            [],
        )?;

        let filter = effective_usage_log_filter("l");
        let sql = format!("SELECT COUNT(*) FROM proxy_request_logs l WHERE {filter}");
        let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
        assert_eq!(count, 1);

        Ok(())
    }

    #[test]
    fn test_backfill_missing_usage_costs_uses_new_gpt_5_5_pricing() -> Result<(), AppError> {
        let db = Database::memory()?;

        {
            let conn = lock_conn!(db.conn);
            insert_usage_log(
                &conn,
                "codex-gpt-5-5-zero-cost",
                "codex",
                "copilot",
                "gpt-5.5",
                "proxy",
                1000,
                1_000_000,
                1_000_000,
                0,
                0,
                200,
                "0",
            )?;
        }

        assert_eq!(db.backfill_missing_usage_costs()?, 1);

        let conn = lock_conn!(db.conn);
        let (input_cost, output_cost, total_cost): (String, String, String) = conn.query_row(
            "SELECT input_cost_usd, output_cost_usd, total_cost_usd
             FROM proxy_request_logs WHERE request_id = 'codex-gpt-5-5-zero-cost'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(input_cost, "10.000000");
        assert_eq!(output_cost, "45.000000");
        assert_eq!(total_cost, "55.000000");

        Ok(())
    }

    #[test]
    fn test_backfill_gpt_pricing_after_upgrade() -> Result<(), AppError> {
        let db = Database::memory()?;
        let cases = [
            (
                "OpenAI/GPT-6-SOL@HIGH",
                "codex",
                3_000_000,
                ["4.000000", "15.000000", "0.400000", "5.000000", "24.400000"],
            ),
            (
                "gpt-6-luna",
                "codex",
                3_000_000,
                ["0.200000", "0.750000", "0.020000", "0.250000", "1.220000"],
            ),
            (
                "gpt-5.3-codex",
                "codex",
                3_000_000,
                ["1.750000", "14.000000", "0.175000", "0.000000", "15.925000"],
            ),
            // Models without a cache-write price do not gain an invented one.
            (
                "gpt-5.4-nano",
                "codex",
                3_000_000,
                ["0.200000", "1.250000", "0.020000", "0.000000", "1.470000"],
            ),
            // Date suffixes still resolve only to an explicitly priced model.
            (
                "gpt-5-mini-2026-07-18",
                "codex",
                3_000_000,
                ["0.250000", "2.000000", "0.025000", "0.000000", "2.275000"],
            ),
        ];
        {
            let conn = lock_conn!(db.conn);
            // Simulate an existing database with unpriced usage before the update.
            conn.execute(
                "DELETE FROM model_pricing WHERE model_id IN
                 ('gpt-6-sol', 'gpt-6-luna', 'gpt-5.3-codex',
                  'gpt-5.4-nano', 'gpt-5-mini')",
                [],
            )?;
            for (model, app, input, _) in &cases {
                insert_usage_log(
                    &conn, model, app, "p1", model, "proxy", 1000, *input, 1_000_000, 1_000_000,
                    1_000_000, 200, "0",
                )?;
            }
            conn.execute(
                "UPDATE proxy_request_logs SET input_token_semantics = ?1",
                [INPUT_TOKEN_SEMANTICS_TOTAL],
            )?;
        }
        assert_eq!(db.backfill_missing_usage_costs()?, 0);
        db.ensure_model_pricing_seeded()?;
        assert_eq!(db.backfill_missing_usage_costs()?, cases.len() as u64);

        let conn = lock_conn!(db.conn);
        for (model, _, _, expected) in cases {
            let costs: [String; 5] = conn.query_row(
                "SELECT input_cost_usd, output_cost_usd, cache_read_cost_usd,
                        cache_creation_cost_usd, total_cost_usd
                 FROM proxy_request_logs WHERE request_id = ?1",
                [model],
                |row| {
                    Ok([
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ])
                },
            )?;
            assert_eq!(costs, expected, "{model}");
        }
        Ok(())
    }

    #[test]
    fn request_cache_details_match_summary_and_preserve_recorded_costs() -> Result<(), AppError> {
        let db = Database::memory()?;
        {
            let conn = lock_conn!(db.conn);
            for (id, input, semantics) in [
                ("total", 1000, INPUT_TOKEN_SEMANTICS_TOTAL),
                ("legacy", 900, 0),
                ("fresh", 100, INPUT_TOKEN_SEMANTICS_FRESH),
            ] {
                insert_usage_log(
                    &conn,
                    id,
                    "codex",
                    "p1",
                    "gpt-6-astra",
                    "proxy",
                    1000,
                    input,
                    50,
                    800,
                    100,
                    200,
                    "9.123456",
                )?;
                conn.execute(
                    "UPDATE proxy_request_logs SET input_token_semantics = ?1 WHERE request_id = ?2",
                    params![semantics, id],
                )?;
            }
        }
        let logs = db.get_request_logs(&LogFilters::default(), 0, 20)?;
        assert_eq!(logs.total, 3);
        for log in &logs.data {
            assert_eq!(log.fresh_input_tokens, 100);
            assert_eq!(log.cache_read_tokens, 800);
            assert_eq!(log.cache_creation_tokens, 100);
            assert_eq!(log.total_cost_usd, "9.123456");
            let json = serde_json::to_value(log).unwrap();
            assert_eq!(json["freshInputTokens"], 100);
        }
        let summary = db.get_usage_summary(None, None, Some("codex"), None, None)?;
        assert_eq!(summary.total_input_tokens, 300);
        assert_eq!(summary.total_cache_read_tokens, 2400);
        assert_eq!(summary.total_cache_creation_tokens, 300);
        assert!((summary.cache_hit_rate - 0.8).abs() < 0.000001);
        assert_eq!(summary.total_cost, "27.370368");
        Ok(())
    }

    #[test]
    fn summary_output_tps_uses_generation_timing_and_respects_model_scope() -> Result<(), AppError>
    {
        let db = Database::memory()?;
        {
            let conn = lock_conn!(db.conn);
            for (id, model, output, latency, first, duration) in [
                ("ttft", "gpt-6-astra", 100, 2000, Some(1000), None),
                ("duration", "gpt-6-astra", 600, 9000, Some(2000), Some(3000)),
                ("fallback", "gpt-6-astra", 100, 2000, None, None),
                ("invalid", "gpt-6-astra", 100, 1000, Some(1500), None),
                ("empty", "gpt-6-astra", 0, 5000, None, None),
                ("other", "gpt-6-luna", 10, 1000, None, None),
            ] {
                insert_usage_log(
                    &conn, id, "codex", "p1", model, "proxy", 1000, 100, output, 0, 0, 200, "1",
                )?;
                conn.execute(
                    "UPDATE proxy_request_logs SET latency_ms=?1, first_token_ms=?2, duration_ms=?3 WHERE request_id=?4",
                    params![latency, first, duration, id],
                )?;
            }
        }
        let astra = db.get_usage_summary(None, None, Some("codex"), None, Some("gpt-6-astra"))?;
        assert!((astra.output_tokens_per_second.unwrap() - 800.0 / 6.0).abs() < 0.000001);
        let all = db.get_usage_summary(None, None, Some("codex"), None, None)?;
        assert!((all.output_tokens_per_second.unwrap() - 810.0 / 7.0).abs() < 0.000001);
        let missing = db.get_usage_summary(None, None, Some("codex"), None, Some("missing"))?;
        assert_eq!(missing.output_tokens_per_second, None);
        assert!(serde_json::to_value(missing)
            .unwrap()
            .get("outputTokensPerSecond")
            .is_none());
        Ok(())
    }

    #[test]
    fn summary_output_tps_uses_latency_when_only_daily_rollups_remain() -> Result<(), AppError> {
        let db = Database::memory()?;
        assert_eq!(
            db.get_usage_summary(None, None, None, None, None)?
                .output_tokens_per_second,
            None
        );
        {
            let conn = lock_conn!(db.conn);
            let date = (Local::now() - chrono::Duration::days(40))
                .format("%Y-%m-%d")
                .to_string();
            conn.execute(
                "INSERT INTO usage_daily_rollups (date, app_type, provider_id, model, request_count, success_count, output_tokens, avg_latency_ms)
                 VALUES (?1, 'codex', 'p1', 'gpt-6-astra', 2, 2, 400, 4000)",
                [date],
            )?;
        }
        let summary = db.get_usage_summary(None, None, None, None, None)?;
        assert_eq!(summary.output_tokens_per_second, Some(50.0));
        Ok(())
    }

    #[test]
    fn read_hit_rate_excludes_output_and_counts_cache_writes_as_misses() {
        for output in [0, 50, 100_000] {
            let (total, rate) = derive_real_total_and_hit_rate(100, output, 100, 800);
            assert_eq!(total, 1000 + output);
            assert!((rate - 0.8).abs() < 0.000001);
        }
        assert_eq!(derive_real_total_and_hit_rate(0, 50, 0, 0), (50, 0.0));
    }

    #[test]
    fn test_backfill_distinguishes_legacy_and_total_cache_semantics() -> Result<(), AppError> {
        let db = Database::memory()?;

        {
            let conn = lock_conn!(db.conn);
            // v12 mirror row: input = fresh + read; creation was reported separately.
            insert_usage_log(
                &conn,
                "legacy-cache-semantics",
                "codex",
                "p1",
                "gpt-5.5",
                "proxy",
                1000,
                800_000,
                0,
                600_000,
                200_000,
                200,
                "0",
            )?;
            // v13 proxy row: input = fresh + read + creation.
            insert_usage_log(
                &conn,
                "total-cache-semantics",
                "codex",
                "p1",
                "gpt-5.5",
                "proxy",
                1001,
                1_000_000,
                0,
                600_000,
                200_000,
                200,
                "0",
            )?;
            conn.execute(
                "UPDATE proxy_request_logs
                 SET input_token_semantics = ?1
                 WHERE request_id = 'total-cache-semantics'",
                [INPUT_TOKEN_SEMANTICS_TOTAL],
            )?;
            insert_usage_log(
                &conn,
                "fresh-cache-semantics",
                "codex",
                "p1",
                "gpt-5.5",
                "proxy",
                1002,
                200_000,
                0,
                600_000,
                200_000,
                200,
                "0",
            )?;
            conn.execute(
                "UPDATE proxy_request_logs SET input_token_semantics = ?1 WHERE request_id = 'fresh-cache-semantics'",
                [INPUT_TOKEN_SEMANTICS_FRESH],
            )?;
        }

        assert_eq!(db.backfill_missing_usage_costs()?, 3);

        let conn = lock_conn!(db.conn);
        let mut stmt = conn.prepare(
            "SELECT request_id, input_cost_usd
             FROM proxy_request_logs
             WHERE request_id IN ('fresh-cache-semantics', 'legacy-cache-semantics', 'total-cache-semantics')
             ORDER BY request_id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            rows,
            vec![
                ("fresh-cache-semantics".to_string(), "2.000000".to_string()),
                ("legacy-cache-semantics".to_string(), "2.000000".to_string()),
                ("total-cache-semantics".to_string(), "2.000000".to_string()),
            ]
        );

        Ok(())
    }

    #[test]
    fn test_backfill_missing_usage_costs_uses_stored_multiplier() -> Result<(), AppError> {
        let db = Database::memory()?;

        {
            let conn = lock_conn!(db.conn);
            insert_usage_log(
                &conn,
                "codex-gpt-5-5-multiplier",
                "codex",
                "copilot",
                "gpt-5.5",
                "proxy",
                1000,
                1_000_000,
                0,
                0,
                0,
                200,
                "0",
            )?;
            conn.execute(
                "UPDATE proxy_request_logs
                 SET cost_multiplier = '1.5'
                 WHERE request_id = 'codex-gpt-5-5-multiplier'",
                [],
            )?;
        }

        assert_eq!(db.backfill_missing_usage_costs()?, 1);

        let conn = lock_conn!(db.conn);
        let (input_cost, total_cost): (String, String) = conn.query_row(
            "SELECT input_cost_usd, total_cost_usd
             FROM proxy_request_logs WHERE request_id = 'codex-gpt-5-5-multiplier'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(input_cost, "10.000000");
        assert_eq!(total_cost, "15.000000");

        Ok(())
    }

    #[test]
    fn test_backfill_missing_usage_costs_falls_back_to_request_model() -> Result<(), AppError> {
        let db = Database::memory()?;

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                    total_cost_usd, latency_ms, status_code, created_at, data_source
                ) VALUES (
                    'codex-request-model-fallback', 'copilot', 'codex', 'unknown', 'gpt-5.5',
                    1000000, 0, 0, 0,
                    '0', '0', '0', '0',
                    '0', 100, 200, 1000, 'proxy'
                )",
                [],
            )?;
        }

        assert_eq!(db.backfill_missing_usage_costs()?, 1);

        let conn = lock_conn!(db.conn);
        let total_cost: String = conn.query_row(
            "SELECT total_cost_usd
             FROM proxy_request_logs WHERE request_id = 'codex-request-model-fallback'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(total_cost, "10.000000");

        Ok(())
    }

    #[test]
    fn test_backfill_skips_request_model_fallback_for_real_unpriced_model() -> Result<(), AppError>
    {
        let db = Database::memory()?;

        {
            let conn = lock_conn!(db.conn);
            // 路由接管场景：model 是上游回显的真实模型（缺定价），request_model
            // 是客户端别名（有定价）。回填不得按别名定价，必须保持 0 成本等待补价。
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                    total_cost_usd, latency_ms, status_code, created_at, data_source
                ) VALUES (
                    'takeover-unpriced-model', 'provider-1', 'codex',
                    'gpt-unpriced-upstream', 'gpt-6-astra',
                    1000000, 0, 0, 0,
                    '0', '0', '0', '0',
                    '0', 100, 200, 1000, 'proxy'
                )",
                [],
            )?;
        }

        // request_model（gpt-6-astra）有定价，但 model 是真实模型名：不得回退
        assert_eq!(db.backfill_missing_usage_costs()?, 0);

        {
            let conn = lock_conn!(db.conn);
            let total_cost: String = conn.query_row(
                "SELECT total_cost_usd
                 FROM proxy_request_logs WHERE request_id = 'takeover-unpriced-model'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(total_cost, "0");

            // 补上真实模型定价后，回填必须按真实模型价格修复（0 成本行未被污染固化）
            conn.execute(
                "INSERT INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
                 VALUES ('gpt-unpriced-upstream', 'GPT Unpriced Upstream', '0.6', '2.5')",
                [],
            )?;
        }

        assert_eq!(db.backfill_missing_usage_costs()?, 1);

        let conn = lock_conn!(db.conn);
        let total_cost: String = conn.query_row(
            "SELECT total_cost_usd
             FROM proxy_request_logs WHERE request_id = 'takeover-unpriced-model'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(total_cost, "0.600000");

        Ok(())
    }

    #[test]
    fn test_backfill_uses_persisted_pricing_model() -> Result<(), AppError> {
        let db = Database::memory()?;

        {
            let conn = lock_conn!(db.conn);
            // request 计价模式 + 接管：写入时锚定出站模型 gpt-future（当时缺价），
            // 但上游回显了别名 → model/request_model 都是 gpt-6-astra（有定价）。
            // 回填必须按落库的 pricing_model 重算，不得换用 model 列的别名价格。
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model, pricing_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                    total_cost_usd, latency_ms, status_code, created_at, data_source
                ) VALUES (
                    'persisted-pricing-model', 'provider-1', 'codex',
                    'gpt-6-astra', 'gpt-6-astra', 'gpt-future',
                    1000000, 0, 0, 0,
                    '0', '0', '0', '0',
                    '0', 100, 200, 1000, 'proxy'
                )",
                [],
            )?;
        }

        // pricing_model（gpt-future）缺价：不得回退到 model 列的别名价格
        assert_eq!(db.backfill_missing_usage_costs()?, 0);

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
                 VALUES ('gpt-future', 'GPT Future', '0.6', '2.5')",
                [],
            )?;
        }

        // 按 pricing_model 也能定位到该行（model/request_model 都不是 gpt-future）
        assert_eq!(db.backfill_missing_usage_costs_for_model("gpt-future")?, 1);

        let conn = lock_conn!(db.conn);
        let total_cost: String = conn.query_row(
            "SELECT total_cost_usd
             FROM proxy_request_logs WHERE request_id = 'persisted-pricing-model'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(total_cost, "0.600000");

        Ok(())
    }

    #[test]
    fn test_scoped_backfill_matches_raw_alias_rows() -> Result<(), AppError> {
        let db = Database::memory()?;

        {
            let conn = lock_conn!(db.conn);
            // 代理日志按上游原文落库：带路由前缀和 :free 后缀的别名形式。
            // 精准回填的筛选必须归一化后匹配，否则这类行要等全量回填才更新。
            insert_usage_log(
                &conn,
                "gpt-alias-zero-cost",
                "codex",
                "provider-1",
                "OpenAI/GPT-FUTURE-2026-09-25@HIGH",
                "proxy",
                1000,
                1_000_000,
                0,
                0,
                0,
                200,
                "0",
            )?;
        }

        // 定价缺失时不应回填
        assert_eq!(db.backfill_missing_usage_costs()?, 0);

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
                 VALUES ('gpt-future', 'GPT Future', '0.6', '2.5')",
                [],
            )?;
        }

        // 按归一化 ID 精准回填，应命中以原始别名落库的行
        assert_eq!(db.backfill_missing_usage_costs_for_model("gpt-future")?, 1);

        let conn = lock_conn!(db.conn);
        let total_cost: String = conn.query_row(
            "SELECT total_cost_usd
             FROM proxy_request_logs WHERE request_id = 'gpt-alias-zero-cost'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(total_cost, "0.600000");

        Ok(())
    }

    #[test]
    fn test_get_usage_summary() -> Result<(), AppError> {
        let db = Database::memory()?;

        // 插入测试数据
        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "req1",
                    "p1",
                    "codex",
                    "gpt-6-astra",
                    100,
                    50,
                    "0.01",
                    100,
                    200,
                    1000
                ],
            )?;
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "req2",
                    "p1",
                    "codex",
                    "gpt-6-astra",
                    200,
                    100,
                    "0.02",
                    150,
                    200,
                    2000
                ],
            )?;
        }

        let summary = db.get_usage_summary(None, None, None, None, None)?;
        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.success_rate, 100.0);
        assert_eq!(summary.avg_latency_ms, 125.0);

        Ok(())
    }

    #[test]
    fn summary_latency_weights_rollups_and_respects_range_and_model_filters() -> Result<(), AppError>
    {
        let db = Database::memory()?;
        let start = local_ts(2026, 9, 1, 0, 0, 0);
        let detail = local_ts(2026, 9, 3, 12, 0, 0);
        let end = local_ts(2026, 9, 3, 23, 59, 59);
        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    latency_ms, status_code, created_at, data_source
                ) VALUES
                    ('success', 'p1', 'codex', 'gpt-6-astra', 100, 200, ?1, 'proxy'),
                    ('failed', 'p1', 'codex', 'gpt-6-astra', 300, 500, ?1, 'proxy'),
                    ('other-model', 'p1', 'codex', 'gpt-6-luna', 9999, 200, ?1, 'proxy'),
                    ('imported', 'p1', 'codex', 'gpt-6-astra', 9999, 200, ?1, 'codex_session'),
                    ('outside', 'p1', 'codex', 'gpt-6-astra', 9999, 200, ?2, 'proxy')",
                params![detail, end + 1],
            )?;
            conn.execute_batch(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model, request_count,
                    success_count, avg_latency_ms
                ) VALUES
                    ('2026-09-02', 'codex', 'p1', 'gpt-6-astra', 3, 2, 700),
                    ('2026-08-31', 'codex', 'p1', 'gpt-6-astra', 100, 100, 9999),
                    ('2026-09-02', 'codex', '_codex_session', 'gpt-6-astra', 100, 100, 9999);",
            )?;
        }
        let summary = db.get_usage_summary(
            Some(start),
            Some(end),
            Some("codex"),
            None,
            Some("gpt-6-astra"),
        )?;
        assert_eq!(summary.total_requests, 5);
        assert!((summary.success_rate - 60.0).abs() < 0.001);
        assert_eq!(summary.avg_latency_ms, 500.0);
        assert_eq!(
            serde_json::to_value(&summary).unwrap()["avgLatencyMs"],
            500.0
        );

        let recent = db.get_usage_summary(
            Some(detail),
            Some(end),
            Some("codex"),
            None,
            Some("gpt-6-astra"),
        )?;
        assert_eq!(recent.total_requests, 2);
        assert_eq!(recent.avg_latency_ms, 200.0);

        let empty = db.get_usage_summary(
            Some(start),
            Some(end),
            Some("codex"),
            None,
            Some("gpt-no-records"),
        )?;
        assert_eq!(empty.total_requests, 0);
        assert_eq!(empty.avg_latency_ms, 0.0);
        Ok(())
    }

    #[test]
    fn test_get_usage_summary_excludes_partial_rollup_boundary_days() -> Result<(), AppError> {
        let db = Database::memory()?;
        let start = local_ts(2024, 1, 1, 12, 0, 0);
        let end = local_ts(2024, 1, 3, 12, 0, 0);

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-01-01",
                    "codex",
                    "p1",
                    "gpt-6-astra",
                    10,
                    10,
                    1000,
                    500,
                    0,
                    0,
                    "1.00",
                    100
                ],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-01-02",
                    "codex",
                    "p1",
                    "gpt-6-astra",
                    20,
                    19,
                    2000,
                    1000,
                    0,
                    0,
                    "2.00",
                    120
                ],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-01-03",
                    "codex",
                    "p1",
                    "gpt-6-astra",
                    30,
                    29,
                    3000,
                    1500,
                    0,
                    0,
                    "3.00",
                    140
                ],
            )?;
        }

        let summary = db.get_usage_summary(Some(start), Some(end), Some("codex"), None, None)?;
        assert_eq!(summary.total_requests, 20);
        assert_eq!(summary.total_input_tokens, 2000);
        assert_eq!(summary.total_output_tokens, 1000);
        assert_eq!(summary.avg_latency_ms, 120.0);

        Ok(())
    }

    #[test]
    fn test_provider_and_model_filters_cover_detail_and_rollup() -> Result<(), AppError> {
        let db = Database::memory()?;
        let detail_ts = local_ts(2026, 6, 10, 12, 0, 0);

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config) VALUES
                 ('prov-a', 'codex', 'Current Copilot', '{}'),
                 ('prov-b', 'codex', 'Previous Copilot', '{}')",
                [],
            )?;

            insert_usage_log(
                &conn,
                "a-1",
                "codex",
                "prov-a",
                "gpt-6-astra",
                "proxy",
                detail_ts,
                100,
                10,
                0,
                0,
                200,
                "1.0",
            )?;
            insert_usage_log(
                &conn,
                "b-1",
                "codex",
                "prov-b",
                "gpt-6-luna",
                "proxy",
                detail_ts,
                200,
                20,
                0,
                0,
                200,
                "2.0",
            )?;
            // 会话占位行：providers 表无此 id，展示名走 CASE 映射。
            insert_usage_log(
                &conn,
                "s-1",
                "codex",
                "_session",
                "gpt-6-astra",
                "session_log",
                detail_ts,
                999,
                99,
                0,
                0,
                200,
                "0.5",
            )?;
            // 计价模型与请求模型不同的行：模型筛选必须按有效计价模型命中。
            insert_usage_log(
                &conn,
                "a-2",
                "codex",
                "prov-a",
                "alias-model",
                "proxy",
                detail_ts,
                50,
                5,
                0,
                0,
                200,
                "0.3",
            )?;
            conn.execute(
                "UPDATE proxy_request_logs SET pricing_model = 'real-model' WHERE request_id = 'a-2'",
                [],
            )?;

            // rollup 历史日行：无范围过滤时全部计入。
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES
                ('2026-06-08', 'codex', 'prov-a', 'gpt-6-astra', 5, 5, 500, 50, 0, 0, '5.0', 100),
                ('2026-06-08', 'codex', 'prov-b', 'gpt-6-luna', 7, 7, 700, 70, 0, 0, '7.0', 100)",
                [],
            )?;
        }

        // ① 汇总按 Provider 展示名过滤：明细 + rollup 都命中。
        let packy = db.get_usage_summary(None, None, None, Some("Current Copilot"), None)?;
        assert_eq!(packy.total_requests, 7, "a-1 + a-2 + rollup 5");

        // ② 汇总按模型过滤（有效计价模型口径）。
        let previous = db.get_usage_summary(None, None, None, None, Some("gpt-6-luna"))?;
        assert_eq!(previous.total_requests, 8, "b-1 + rollup 7");

        // ③ pricing_model 优先于 model：alias-model 查不到，real-model 查得到。
        let by_alias = db.get_usage_summary(None, None, None, None, Some("alias-model"))?;
        assert_eq!(by_alias.total_requests, 0);
        let by_real = db.get_usage_summary(None, None, None, None, Some("real-model"))?;
        assert_eq!(by_real.total_requests, 1);

        // Imported sessions remain stored, but cannot inflate Atlas traffic.
        let session = db.get_usage_summary(None, None, None, Some("_codex_session"), None)?;
        assert_eq!(session.total_requests, 0);

        // ⑤ Provider 统计 + 模型过滤：只剩 Previous Copilot 一行。
        let provider_stats = db.get_provider_stats(None, None, None, None, Some("gpt-6-luna"))?;
        assert_eq!(provider_stats.len(), 1);
        assert_eq!(provider_stats[0].provider_name, "Previous Copilot");
        assert_eq!(provider_stats[0].request_count, 8);

        // ⑥ 模型统计 + Provider 过滤：只剩 Current Copilot 名下的模型。
        let model_stats = db.get_model_stats(None, None, None, Some("Current Copilot"), None)?;
        let models: Vec<&str> = model_stats.iter().map(|m| m.model.as_str()).collect();
        assert!(models.contains(&"gpt-6-astra"));
        assert!(models.contains(&"real-model"));
        assert!(!models.contains(&"gpt-6-luna"));

        // ⑧ 趋势（>24h 走天分桶 + rollup 分支）。
        let t_start = local_ts(2026, 6, 8, 0, 0, 0);
        let t_end = local_ts(2026, 6, 10, 23, 59, 0);
        let trends = db.get_daily_trends(
            Some(t_start),
            Some(t_end),
            None,
            Some("Current Copilot"),
            None,
        )?;
        let total_req: u64 = trends.iter().map(|d| d.request_count).sum();
        assert_eq!(total_req, 7, "明细 2 + rollup 5");

        // ⑨ 趋势 ≤24h 走小时分桶分支（?1/?2/?3 编号参数与追加过滤混用的路径），
        //    同时验证 Provider + 模型组合过滤。
        let h_start = local_ts(2026, 6, 10, 0, 0, 0);
        let h_end = local_ts(2026, 6, 10, 20, 0, 0);
        let hourly = db.get_daily_trends(
            Some(h_start),
            Some(h_end),
            None,
            Some("Current Copilot"),
            Some("gpt-6-astra"),
        )?;
        let hourly_req: u64 = hourly.iter().map(|d| d.request_count).sum();
        assert_eq!(hourly_req, 1, "仅 a-1 命中（a-2 计价模型不同）");

        // ⑩ 请求日志列表与下拉同口径：精确名 + 有效计价模型。
        let logs = db.get_request_logs(
            &LogFilters {
                provider_name: Some("Current Copilot".to_string()),
                model: Some("real-model".to_string()),
                ..Default::default()
            },
            0,
            10,
        )?;
        assert_eq!(logs.total, 1);
        assert_eq!(logs.data[0].request_id, "a-2");

        Ok(())
    }

    #[test]
    fn test_get_usage_summary_includes_end_day_rollup_for_minute_precision_end_time(
    ) -> Result<(), AppError> {
        let db = Database::memory()?;
        let start = local_ts(2024, 1, 1, 0, 0, 0);
        let end = local_ts(2024, 1, 2, 23, 59, 0);

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-01-01",
                    "codex",
                    "p1",
                    "gpt-6-astra",
                    10,
                    10,
                    1000,
                    500,
                    0,
                    0,
                    "1.00",
                    100
                ],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-01-02",
                    "codex",
                    "p1",
                    "gpt-6-astra",
                    20,
                    19,
                    2000,
                    1000,
                    0,
                    0,
                    "2.00",
                    120
                ],
            )?;
        }

        let summary = db.get_usage_summary(Some(start), Some(end), Some("codex"), None, None)?;
        assert_eq!(summary.total_requests, 30);
        assert_eq!(summary.total_input_tokens, 3000);
        assert_eq!(summary.total_output_tokens, 1500);

        Ok(())
    }

    #[test]
    fn model_token_details_combine_normalized_logs_and_rollups_by_billing_model(
    ) -> Result<(), AppError> {
        let db = Database::memory()?;
        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs
                 (request_id,provider_id,app_type,model,pricing_model,input_tokens,output_tokens,
                  cache_read_tokens,cache_creation_tokens,input_token_semantics,total_cost_usd,latency_ms,status_code,created_at)
                 VALUES ('total','copilot','codex','upstream','gpt-6-astra',1000,50,800,100,?1,'1',100,200,1000)",
                [INPUT_TOKEN_SEMANTICS_TOTAL],
            )?;
            conn.execute(
                "INSERT INTO proxy_request_logs
                 (request_id,provider_id,app_type,model,pricing_model,input_tokens,output_tokens,
                  cache_read_tokens,cache_creation_tokens,input_token_semantics,total_cost_usd,latency_ms,status_code,created_at)
                 VALUES ('fresh','copilot','codex','upstream','gpt-6-astra',200,20,100,0,?1,'1',100,200,1001)",
                [INPUT_TOKEN_SEMANTICS_FRESH],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups
                 (date,app_type,provider_id,model,pricing_model,request_count,success_count,
                  input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,input_token_semantics,total_cost_usd)
                 VALUES ('2026-09-25','codex','copilot','upstream','gpt-6-astra',3,3,300,30,900,100,?1,'2')",
                [INPUT_TOKEN_SEMANTICS_FRESH],
            )?;
        }
        let stats = db.get_model_stats(None, None, Some("codex"), None, Some("gpt-6-astra"))?;
        assert_eq!(stats.len(), 1);
        let stat = &stats[0];
        assert_eq!(stat.model, "gpt-6-astra");
        assert_eq!(stat.request_count, 5);
        assert_eq!(stat.total_input_tokens, 600);
        assert_eq!(stat.total_cache_read_tokens, 1800);
        assert_eq!(stat.total_cache_creation_tokens, 200);
        assert_eq!(stat.total_output_tokens, 100);
        assert_eq!(stat.total_cost, "4.000000");
        assert_eq!(stat.avg_cost_per_request, "0.800000");
        Ok(())
    }

    #[test]
    fn test_get_model_stats() -> Result<(), AppError> {
        let db = Database::memory()?;

        // 插入测试数据
        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "req1",
                    "p1",
                    "codex",
                    "gpt-6-astra",
                    100,
                    50,
                    "0.01",
                    100,
                    200,
                    1000
                ],
            )?;
        }

        let stats = db.get_model_stats(None, None, None, None, None)?;
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].model, "gpt-6-astra");
        assert_eq!(stats[0].request_count, 1);

        Ok(())
    }

    #[test]
    fn unpriced_model_usage_reports_missing_prices_with_token_totals() -> Result<(), AppError> {
        let db = Database::memory()?;
        let start = local_ts(2026, 4, 1, 12, 0, 0);
        let end = local_ts(2026, 4, 1, 18, 0, 0);

        {
            let conn = lock_conn!(db.conn);
            insert_usage_log(
                &conn,
                "priced-alias",
                "codex",
                "copilot",
                "openai/gpt-6-astra-2026-04-01-high",
                "proxy",
                start + 60,
                1000,
                50,
                200,
                100,
                200,
                "0",
            )?;
            insert_usage_log(
                &conn,
                "unpriced-model",
                "codex",
                "copilot",
                "gpt-7-future-2026-04-01-high",
                "proxy",
                start + 120,
                1000,
                50,
                200,
                100,
                200,
                "0",
            )?;
            conn.execute(
                "UPDATE proxy_request_logs SET input_token_semantics = ?1
                 WHERE request_id = 'unpriced-model'",
                [INPUT_TOKEN_SEMANTICS_TOTAL],
            )?;
            insert_usage_log(
                &conn,
                "outside-range",
                "codex",
                "copilot",
                "gpt-8-outside",
                "proxy",
                end + 1,
                100,
                10,
                0,
                0,
                200,
                "0",
            )?;
            insert_usage_log(
                &conn,
                "imported-model",
                "codex",
                "copilot",
                "gpt-7-imported",
                "codex_session",
                start + 180,
                100,
                10,
                0,
                0,
                200,
                "0",
            )?;
            insert_usage_log(
                &conn,
                "non-gpt-model",
                "codex",
                "copilot",
                "claude-retired",
                "proxy",
                start + 240,
                100,
                10,
                0,
                0,
                200,
                "0",
            )?;
            insert_usage_log(
                &conn,
                "no-token-usage",
                "codex",
                "copilot",
                "gpt-empty",
                "proxy",
                start + 300,
                0,
                0,
                0,
                0,
                200,
                "0",
            )?;
        }

        let unpriced =
            db.get_unpriced_model_usage(Some(start), Some(end), Some("codex"), None, None)?;
        assert_eq!(unpriced.len(), 2);
        assert!(unpriced.iter().any(|row| row.model == "claude-retired"));
        let future = unpriced
            .iter()
            .find(|row| row.model == "gpt-7-future-2026-04-01-high")
            .unwrap();
        assert_eq!(future.request_count, 1);
        assert_eq!(future.fresh_input_tokens, 700);
        assert_eq!(future.output_tokens, 50);
        assert_eq!(future.cache_read_tokens, 200);
        assert!((future.cache_hit_rate - 0.2).abs() < 0.0001);

        assert!(db
            .get_unpriced_model_usage(Some(end + 10), Some(end + 100), Some("codex"), None, None)?
            .is_empty());

        Ok(())
    }

    #[test]
    fn prices_multiple_vendors_without_guessing_prices_for_future_variants() -> Result<(), AppError>
    {
        let db = Database::memory()?;
        {
            let conn = lock_conn!(db.conn);
            for model in ["gemini-3.8-flash", "grok-4.7", "new-vendor/agent"] {
                insert_usage_log(
                    &conn, model, "codex", "copilot", model, "proxy", 1000, 1_000_000, 1_000_000,
                    0, 0, 200, "0",
                )?;
            }
            for unknown in [
                "grok-99",
                "new-vendor/agent",
                "openai/grok-4.7",
                "google/grok-4.7",
                "gemini-3.8-flash-high",
            ] {
                assert!(
                    find_model_pricing_row(&conn, unknown)?.is_none(),
                    "{unknown}"
                );
            }
        }
        assert_eq!(db.backfill_missing_usage_costs()?, 2);
        let unpriced = db.get_unpriced_model_usage(None, None, Some("codex"), None, None)?;
        assert_eq!(unpriced.len(), 1);
        assert_eq!(unpriced[0].model, "new-vendor/agent");
        let conn = lock_conn!(db.conn);
        for (model, expected) in [("gemini-3.8-flash", "4.500000"), ("grok-4.7", "16.000000")] {
            let cost: String = conn.query_row(
                "SELECT total_cost_usd FROM proxy_request_logs WHERE request_id = ?1",
                [model],
                |row| row.get(0),
            )?;
            assert_eq!(cost, expected);
        }
        Ok(())
    }

    #[test]
    fn unpriced_model_usage_uses_pricing_aliases_and_selected_range() -> Result<(), AppError> {
        let db = Database::memory()?;
        let start = local_ts(2026, 4, 1, 12, 0, 0);
        let end = local_ts(2026, 4, 1, 18, 0, 0);

        {
            let conn = lock_conn!(db.conn);
            insert_usage_log(
                &conn,
                "priced-alias",
                "codex",
                "copilot",
                "openai/gpt-6-astra-2026-04-01-high",
                "proxy",
                start + 60,
                1000,
                50,
                200,
                100,
                200,
                "0",
            )?;
            insert_usage_log(
                &conn,
                "unpriced-model",
                "codex",
                "copilot",
                "gpt-7-future-2026-04-01-high",
                "proxy",
                start + 120,
                1000,
                50,
                200,
                100,
                200,
                "0",
            )?;
            conn.execute(
                "UPDATE proxy_request_logs
                 SET request_model = 'gpt-6-astra',
                     pricing_model = 'gpt-7-future-2026-04-01-high',
                     input_token_semantics = ?1
                 WHERE request_id = 'unpriced-model'",
                [INPUT_TOKEN_SEMANTICS_TOTAL],
            )?;
            insert_usage_log(
                &conn,
                "outside-range",
                "codex",
                "copilot",
                "gpt-8-outside",
                "proxy",
                end + 1,
                100,
                10,
                0,
                0,
                200,
                "0",
            )?;
            insert_usage_log(
                &conn,
                "imported-model",
                "codex",
                "copilot",
                "gpt-7-imported",
                "codex_session",
                start + 180,
                100,
                10,
                0,
                0,
                200,
                "0",
            )?;
            insert_usage_log(
                &conn,
                "non-gpt-model",
                "codex",
                "copilot",
                "claude-retired",
                "proxy",
                start + 240,
                100,
                10,
                0,
                0,
                200,
                "0",
            )?;
            insert_usage_log(
                &conn,
                "no-token-usage",
                "codex",
                "copilot",
                "gpt-empty",
                "proxy",
                start + 300,
                0,
                0,
                0,
                0,
                200,
                "0",
            )?;
        }

        let unpriced =
            db.get_unpriced_model_usage(Some(start), Some(end), Some("codex"), None, None)?;
        assert_eq!(unpriced.len(), 2);
        assert!(unpriced.iter().any(|row| row.model == "claude-retired"));
        let future = unpriced
            .iter()
            .find(|row| row.model == "gpt-7-future-2026-04-01-high")
            .unwrap();
        assert_eq!(future.request_count, 1);
        assert_eq!(future.fresh_input_tokens, 700);
        assert_eq!(future.output_tokens, 50);
        assert_eq!(future.cache_read_tokens, 200);
        assert!((future.cache_hit_rate - 0.2).abs() < 0.0001);

        assert!(db
            .get_unpriced_model_usage(Some(end + 10), Some(end + 100), Some("codex"), None, None)?
            .is_empty());

        Ok(())
    }

    #[test]
    fn test_get_provider_stats_with_time_filter() -> Result<(), AppError> {
        let db = Database::memory()?;

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "old",
                    "p1",
                    "codex",
                    "gpt-6-astra",
                    100,
                    50,
                    "0.01",
                    100,
                    200,
                    1000
                ],
            )?;
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "new",
                    "p1",
                    "codex",
                    "gpt-6-astra",
                    200,
                    75,
                    "0.02",
                    120,
                    200,
                    2000
                ],
            )?;
        }

        let stats = db.get_provider_stats(Some(1500), Some(2500), Some("codex"), None, None)?;
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].provider_id, "p1");
        assert_eq!(stats[0].request_count, 1);
        assert_eq!(stats[0].total_tokens, 275);

        Ok(())
    }

    #[test]
    fn test_get_provider_stats_excludes_partial_rollup_boundary_days() -> Result<(), AppError> {
        let db = Database::memory()?;
        let start = local_ts(2024, 2, 1, 12, 0, 0);
        let end = local_ts(2024, 2, 3, 12, 0, 0);

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-02-01",
                    "codex",
                    "p-rollup",
                    "gpt-6-astra",
                    5,
                    5,
                    500,
                    250,
                    0,
                    0,
                    "0.50",
                    100
                ],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-02-02",
                    "codex",
                    "p-rollup",
                    "gpt-6-astra",
                    8,
                    7,
                    800,
                    400,
                    0,
                    0,
                    "0.80",
                    120
                ],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-02-03",
                    "codex",
                    "p-rollup",
                    "gpt-6-astra",
                    12,
                    11,
                    1200,
                    600,
                    0,
                    0,
                    "1.20",
                    140
                ],
            )?;
        }

        let stats = db.get_provider_stats(Some(start), Some(end), Some("codex"), None, None)?;
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].provider_id, "p-rollup");
        assert_eq!(stats[0].request_count, 8);
        assert_eq!(stats[0].total_tokens, 1200);

        Ok(())
    }

    #[test]
    fn test_get_daily_trends_respects_shorter_than_24_hours() -> Result<(), AppError> {
        let db = Database::memory()?;

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "req-short",
                    "p1",
                    "codex",
                    "gpt-6-astra",
                    100,
                    50,
                    "0.01",
                    100,
                    200,
                    10_800
                ],
            )?;
        }

        let stats = db.get_daily_trends(Some(0), Some(15 * 60 * 60), Some("codex"), None, None)?;
        assert_eq!(stats.len(), 15);
        assert_eq!(stats[3].request_count, 1);

        Ok(())
    }

    #[test]
    fn hourly_trends_include_end_boundary_without_replacing_the_last_bucket() -> Result<(), AppError>
    {
        for duration in [3600, 3 * 3600, 24 * 3600, 3 * 3600 + 1800] {
            let db = Database::memory()?;
            let start = local_ts(2026, 4, 1, 12, 0, 0);
            let end = start + duration;
            {
                let conn = lock_conn!(db.conn);
                for (id, app, provider, model, source, timestamp) in [
                    ("first", "codex", "p1", "gpt-6-astra", "proxy", start),
                    ("last-hour", "codex", "p1", "gpt-6-astra", "proxy", end - 1),
                    ("end", "codex", "p1", "gpt-6-astra", "proxy", end),
                    ("outside", "codex", "p1", "gpt-6-astra", "proxy", end + 1),
                    (
                        "imported",
                        "codex",
                        "p1",
                        "gpt-6-astra",
                        "codex_session",
                        end,
                    ),
                    ("other-model", "codex", "p1", "gpt-6-luna", "proxy", end),
                    ("other-provider", "codex", "p2", "gpt-6-astra", "proxy", end),
                    ("other-app", "historical", "p1", "gpt-6-astra", "proxy", end),
                ] {
                    insert_usage_log(
                        &conn, id, app, provider, model, source, timestamp, 1000, 100, 600, 20,
                        200, "0.012345",
                    )?;
                }
            }

            let stats = db.get_daily_trends(
                Some(start),
                Some(end),
                Some("codex"),
                Some("p1"),
                Some("gpt-6-astra"),
            )?;
            assert_eq!(stats.len(), ((duration + 3599) / 3600) as usize);
            assert_eq!(
                stats.last().unwrap().request_count,
                if duration == 3600 { 3 } else { 2 },
                "inclusive end must be aggregated into the last bucket ({duration}s range)"
            );
            assert_eq!(stats.iter().map(|s| s.request_count).sum::<u64>(), 3);
            assert_eq!(
                stats.iter().map(|s| s.total_input_tokens).sum::<u64>(),
                1200
            );
            assert_eq!(
                stats.iter().map(|s| s.total_output_tokens).sum::<u64>(),
                300
            );
            assert_eq!(
                stats.iter().map(|s| s.total_cache_read_tokens).sum::<u64>(),
                1800
            );
            assert_eq!(
                stats
                    .iter()
                    .map(|s| s.total_cache_creation_tokens)
                    .sum::<u64>(),
                60
            );
            assert_eq!(
                stats
                    .iter()
                    .map(|s| rust_decimal::Decimal::from_str(&s.total_cost).unwrap())
                    .sum::<rust_decimal::Decimal>(),
                rust_decimal::Decimal::from_str("0.037035").unwrap()
            );
        }
        Ok(())
    }

    #[test]
    fn test_get_daily_trends_groups_ranges_longer_than_24_hours_by_local_day(
    ) -> Result<(), AppError> {
        let db = Database::memory()?;
        let start = local_ts(2024, 3, 1, 12, 0, 0);
        let end = local_ts(2024, 3, 3, 12, 0, 0);

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "day-1-detail",
                    "p1",
                    "codex",
                    "gpt-6-astra",
                    100,
                    50,
                    "0.01",
                    100,
                    200,
                    local_ts(2024, 3, 1, 13, 0, 0)
                ],
            )?;
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "day-3-detail",
                    "p1",
                    "codex",
                    "gpt-6-astra",
                    200,
                    75,
                    "0.02",
                    110,
                    200,
                    local_ts(2024, 3, 3, 10, 0, 0)
                ],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-03-02",
                    "codex",
                    "p1",
                    "gpt-6-astra",
                    4,
                    4,
                    400,
                    200,
                    0,
                    0,
                    "0.40",
                    120
                ],
            )?;
        }

        let stats = db.get_daily_trends(Some(start), Some(end), Some("codex"), None, None)?;
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0].request_count, 1);
        assert_eq!(stats[0].total_tokens, 150);
        assert_eq!(stats[1].request_count, 4);
        assert_eq!(stats[1].total_tokens, 600);
        assert_eq!(stats[2].request_count, 1);
        assert_eq!(stats[2].total_tokens, 275);

        Ok(())
    }

    #[test]
    fn test_get_model_stats_excludes_partial_rollup_boundary_days() -> Result<(), AppError> {
        let db = Database::memory()?;
        let start = local_ts(2024, 4, 1, 12, 0, 0);
        let end = local_ts(2024, 4, 3, 12, 0, 0);

        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-04-01",
                    "codex",
                    "p1",
                    "gpt-6-luna",
                    6,
                    6,
                    600,
                    300,
                    0,
                    0,
                    "0.60",
                    100
                ],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-04-02",
                    "codex",
                    "p1",
                    "gpt-6-luna",
                    9,
                    8,
                    900,
                    450,
                    0,
                    0,
                    "0.90",
                    110
                ],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model,
                    request_count, success_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    "2024-04-03",
                    "codex",
                    "p1",
                    "gpt-6-luna",
                    12,
                    11,
                    1200,
                    600,
                    0,
                    0,
                    "1.20",
                    130
                ],
            )?;
        }

        let stats = db.get_model_stats(Some(start), Some(end), Some("codex"), None, None)?;
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].model, "gpt-6-luna");
        assert_eq!(stats[0].request_count, 9);
        assert_eq!(stats[0].total_tokens, 1350);

        Ok(())
    }

    #[test]
    fn test_strip_model_date_suffix_is_utf8_safe() {
        assert_eq!(
            strip_model_date_suffix("模型-2026-05-14").as_deref(),
            Some("模型")
        );
        assert_eq!(strip_model_date_suffix("abc🚀12345678"), None);
    }

    #[test]
    fn test_prefix_pricing_does_not_match_short_base_model_to_variant() -> Result<(), AppError> {
        let db = Database::memory()?;
        let conn = lock_conn!(db.conn);

        conn.execute("DELETE FROM model_pricing WHERE model_id LIKE 'gpt-5%'", [])?;
        for (model_id, display_name) in [("gpt-5-mini", "GPT-5 Mini"), ("gpt-5-pro", "GPT-5 Pro")] {
            conn.execute(
                "INSERT INTO model_pricing (
                    model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million
                ) VALUES (?1, ?2, '1', '2', '0', '0')",
                params![model_id, display_name],
            )?;
        }

        let result = find_model_pricing_row(&conn, "gpt-5")?;
        assert!(
            result.is_none(),
            "缺少 gpt-5 基础定价时，不应前缀误匹配到 gpt-5-mini/gpt-5-pro"
        );

        Ok(())
    }

    #[test]
    fn gpt_pricing_matches_dates_and_reasoning_without_vendor_fallbacks() -> Result<(), AppError> {
        let db = Database::memory()?;
        let conn = lock_conn!(db.conn);
        for (alias, model) in [
            ("gpt-5.3-codex@low", "gpt-5.3-codex"),
            ("OpenAI/GPT-5.5@HIGH", "gpt-5.5-high"),
            ("OpenAI/GPT-5.5-2026-05-14", "gpt-5.5"),
            ("gpt-5-mini-20260718", "gpt-5-mini"),
            ("gpt-6-astra@ultra", "gpt-6-astra"),
            ("gpt-6-luna@max", "gpt-6-luna"),
        ] {
            assert_eq!(
                find_model_pricing_row(&conn, alias)?,
                find_model_pricing_row(&conn, model)?,
                "{alias}"
            );
            assert!(find_model_pricing_row(&conn, alias)?.is_some(), "{alias}");
        }
        assert!(find_model_pricing_row(&conn, "retired-vendor/gpt-6-astra")?.is_none());
        assert!(find_model_pricing_row(&conn, "unknown")?.is_none());
        Ok(())
    }

    #[test]
    fn backfill_preserves_recorded_costs_and_never_reprices_imported_sessions(
    ) -> Result<(), AppError> {
        let db = Database::memory()?;
        {
            let conn = lock_conn!(db.conn);
            for (id, source, cost) in [
                ("missing", "proxy", "0"),
                ("recorded", "proxy", "12.345678"),
                ("imported", "codex_session", "0"),
            ] {
                insert_usage_log(
                    &conn,
                    id,
                    "codex",
                    "copilot",
                    "gpt-5-mini",
                    source,
                    1000,
                    1_000_000,
                    0,
                    0,
                    0,
                    200,
                    cost,
                )?;
            }
            conn.execute(
                "UPDATE model_pricing SET input_cost_per_million = '99' WHERE model_id = 'gpt-5-mini'",
                [],
            )?;
        }
        assert_eq!(db.backfill_missing_usage_costs()?, 1);
        let conn = lock_conn!(db.conn);
        for (id, expected) in [
            ("missing", "99.000000"),
            ("recorded", "12.345678"),
            ("imported", "0"),
        ] {
            assert_eq!(
                conn.query_row(
                    "SELECT total_cost_usd FROM proxy_request_logs WHERE request_id = ?1",
                    [id],
                    |row| row.get::<_, String>(0)
                )?,
                expected
            );
        }
        Ok(())
    }
}
