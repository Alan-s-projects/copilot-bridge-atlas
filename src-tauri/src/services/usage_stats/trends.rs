use super::{
    compute_rollup_date_bounds, effective_usage_log_filter, effective_usage_rollup_filter,
    providers_join, push_provider_model_filters, DailyStats,
};
use crate::{
    database::{lock_conn, Database},
    error::AppError,
    services::sql_helpers::fresh_input_sql,
};
use chrono::{Datelike, Duration, Local, LocalResult, Months, NaiveDate, NaiveDateTime, TimeZone};
use rusqlite::ToSql;
use serde::{Deserialize, Serialize};

pub const TREND_INTERVALS: [u32; 8] = [1, 3, 5, 7, 10, 15, 30, 60];
pub const MAX_TREND_BUCKETS: usize = 2000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrendUnit {
    Minute,
    Hour,
    #[default]
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrendGrouping {
    pub interval: u32,
    pub unit: TrendUnit,
}

impl Default for TrendGrouping {
    fn default() -> Self {
        Self {
            interval: 1,
            unit: TrendUnit::Day,
        }
    }
}

impl TrendGrouping {
    pub fn validate(&self) -> Result<(), AppError> {
        if !TREND_INTERVALS.contains(&self.interval) {
            return Err(AppError::Config("Unsupported trend interval".into()));
        }
        Ok(())
    }

    fn calendar(&self) -> bool {
        matches!(
            self.unit,
            TrendUnit::Day | TrendUnit::Week | TrendUnit::Month
        )
    }

    fn floor(&self, value: NaiveDateTime) -> Result<NaiveDateTime, AppError> {
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let count = i64::from(self.interval);
        let date = match self.unit {
            TrendUnit::Minute | TrendUnit::Hour => {
                let seconds = count
                    * if self.unit == TrendUnit::Minute {
                        60
                    } else {
                        3600
                    };
                let rounded = value.and_utc().timestamp().div_euclid(seconds) * seconds;
                return chrono::DateTime::from_timestamp(rounded, 0)
                    .map(|date| date.naive_utc())
                    .ok_or_else(invalid_time);
            }
            TrendUnit::Month => {
                let month = (i64::from(value.year()) - 1970) * 12 + i64::from(value.month0());
                let rounded = month.div_euclid(count) * count;
                NaiveDate::from_ymd_opt(
                    i32::try_from(1970 + rounded.div_euclid(12)).map_err(|_| invalid_time())?,
                    (rounded.rem_euclid(12) + 1) as u32,
                    1,
                )
                .ok_or_else(invalid_time)?
            }
            TrendUnit::Day | TrendUnit::Week => {
                let anchor = if self.unit == TrendUnit::Week {
                    NaiveDate::from_ymd_opt(1970, 1, 5).unwrap() // Monday
                } else {
                    epoch
                };
                let step = count * if self.unit == TrendUnit::Week { 7 } else { 1 };
                let days = value.date().signed_duration_since(anchor).num_days();
                anchor
                    .checked_add_signed(Duration::days(days.div_euclid(step) * step))
                    .ok_or_else(invalid_time)?
            }
        };
        date.and_hms_opt(0, 0, 0).ok_or_else(invalid_time)
    }

    fn advance(&self, value: NaiveDateTime, backwards: bool) -> Result<NaiveDateTime, AppError> {
        if self.unit == TrendUnit::Month {
            let months = Months::new(self.interval);
            return (if backwards {
                value.checked_sub_months(months)
            } else {
                value.checked_add_months(months)
            })
            .ok_or_else(invalid_time);
        }
        let seconds = i64::from(self.interval)
            * match self.unit {
                TrendUnit::Minute => 60,
                TrendUnit::Hour => 3600,
                TrendUnit::Day => 86400,
                TrendUnit::Week => 7 * 86400,
                TrendUnit::Month => unreachable!(),
            };
        value
            .checked_add_signed(Duration::seconds(if backwards {
                -seconds
            } else {
                seconds
            }))
            .ok_or_else(invalid_time)
    }
}

fn invalid_time() -> AppError {
    AppError::Config("Trend date range is outside supported calendar bounds".into())
}

// A midnight or sub-hour boundary can be skipped by a timezone transition.
// Resolve it to the first valid local minute; preserve both instants in a fold.
fn resolve_boundary<T: TimeZone>(zone: &T, mut value: NaiveDateTime) -> Result<Vec<i64>, AppError> {
    for _ in 0..=24 * 60 {
        match zone.from_local_datetime(&value) {
            LocalResult::Single(date) => return Ok(vec![date.timestamp()]),
            LocalResult::Ambiguous(a, b) => {
                let mut values = vec![a.timestamp(), b.timestamp()];
                values.sort_unstable();
                values.dedup();
                return Ok(values);
            }
            LocalResult::None => {
                value = value
                    .checked_add_signed(Duration::minutes(1))
                    .ok_or_else(invalid_time)?
            }
        }
    }
    Err(invalid_time())
}

fn bucket_boundaries<T: TimeZone>(
    start: i64,
    end: i64,
    grouping: TrendGrouping,
    zone: &T,
) -> Result<Vec<(i64, i64)>, AppError> {
    grouping.validate()?;
    if start > end {
        return Err(AppError::Config(
            "Trend range start must not be after its end".into(),
        ));
    }
    let local = zone
        .timestamp_opt(start, 0)
        .single()
        .ok_or_else(invalid_time)?;
    let resolve = |naive| {
        let mut values = resolve_boundary(zone, naive)?;
        // A repeated midnight still belongs to one calendar day/week/month.
        if grouping.calendar() {
            values.truncate(1);
        }
        Ok::<_, AppError>(values)
    };
    let mut naive = grouping.floor(local.naive_local())?;
    let mut current = loop {
        if let Some(found) = resolve(naive)?.into_iter().filter(|&ts| ts <= start).max() {
            break found;
        }
        naive = grouping.advance(naive, true)?;
    };
    let mut result = Vec::new();
    while current <= end {
        if result.len() == MAX_TREND_BUCKETS {
            return Err(AppError::Config(format!("More than {MAX_TREND_BUCKETS} intervals. Choose a larger grouping or a shorter date range.")));
        }
        let local = zone
            .timestamp_opt(current, 0)
            .single()
            .ok_or_else(invalid_time)?;
        let aligned = grouping.floor(local.naive_local())?;
        let mut candidates = resolve(aligned)?;
        candidates.extend(resolve(grouping.advance(aligned, false)?)?);
        let next = candidates
            .into_iter()
            .filter(|&ts| ts > current)
            .min()
            .ok_or_else(invalid_time)?;
        result.push((current, next));
        current = next;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Timelike, Utc};

    fn ts(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .earliest()
            .unwrap()
            .timestamp()
    }

    fn insert(
        db: &Database,
        id: &str,
        timestamp: i64,
        input: u32,
        read: u32,
        write: u32,
        status: u16,
        cost: &str,
        billing_model: &str,
    ) {
        db.conn.lock().unwrap().execute(
            "INSERT INTO proxy_request_logs (request_id,provider_id,app_type,model,pricing_model,
             input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,input_token_semantics,
             status_code,latency_ms,total_cost_usd,created_at,data_source)
             VALUES (?1,'copilot','codex','upstream-alias',?2,?3,25,?4,?5,1,?6,100,?7,?8,'proxy')",
            rusqlite::params![id,billing_model,input,read,write,status,cost,timestamp],
        ).unwrap();
    }

    #[test]
    fn all_forty_groupings_reconcile_recorded_totals_and_weight_rates() {
        let db = Database::memory().unwrap();
        let start = ts(2026, 9, 21, 0, 0);
        let end = start + 12 * 3600 - 1;
        insert(
            &db,
            "one",
            start + 30,
            1000,
            800,
            100,
            200,
            "0.123456",
            "gpt-6-astra",
        );
        insert(
            &db,
            "two",
            start + 3 * 3600,
            100,
            0,
            0,
            500,
            "0.2",
            "gpt-6-astra",
        );
        insert(
            &db,
            "excluded",
            start + 40,
            9000,
            8000,
            0,
            200,
            "100",
            "other-model",
        );
        let summary = db
            .get_usage_summary(
                Some(start),
                Some(end),
                Some("codex"),
                None,
                Some("gpt-6-astra"),
            )
            .unwrap();
        for unit in [
            TrendUnit::Minute,
            TrendUnit::Hour,
            TrendUnit::Day,
            TrendUnit::Week,
            TrendUnit::Month,
        ] {
            for interval in TREND_INTERVALS {
                let response = db
                    .get_grouped_usage_trends(
                        Some(start),
                        Some(end),
                        Some("codex"),
                        None,
                        Some("gpt-6-astra"),
                        TrendGrouping { interval, unit },
                    )
                    .unwrap();
                assert!(!response.has_incomplete_rollup_data);
                let count: u64 = response
                    .buckets
                    .iter()
                    .map(|b| b.totals.request_count)
                    .sum();
                let cost: f64 = response
                    .buckets
                    .iter()
                    .map(|b| b.totals.total_cost.parse::<f64>().unwrap())
                    .sum();
                assert_eq!(count, summary.total_requests);
                assert_eq!(
                    response.total_cost.as_deref(),
                    Some(summary.total_cost.as_str())
                );
                assert!((cost - summary.total_cost.parse::<f64>().unwrap()).abs() < 0.000001);
                assert_eq!(
                    response
                        .buckets
                        .iter()
                        .map(|b| b.totals.total_input_tokens)
                        .sum::<u64>(),
                    200
                );
                assert_eq!(
                    response
                        .buckets
                        .iter()
                        .map(|b| b.totals.total_cache_read_tokens)
                        .sum::<u64>(),
                    800
                );
                assert_eq!(
                    response
                        .buckets
                        .iter()
                        .map(|b| b.totals.total_cache_creation_tokens)
                        .sum::<u64>(),
                    100
                );
                assert_eq!(
                    response
                        .buckets
                        .iter()
                        .map(|b| b.totals.total_output_tokens)
                        .sum::<u64>(),
                    50
                );
                for bucket in response
                    .buckets
                    .iter()
                    .filter(|b| b.totals.request_count == 0)
                {
                    assert_eq!(bucket.read_cache_hit_rate, None);
                    assert_eq!(bucket.success_rate, None);
                    assert_eq!(bucket.totals.total_cost, "0.000000");
                }
                if response.buckets.len() == 1 {
                    assert_eq!(response.buckets[0].success_rate, Some(0.5));
                    assert!(
                        (response.buckets[0].read_cache_hit_rate.unwrap() - 800.0 / 1100.0).abs()
                            < 0.000001
                    );
                }
            }
        }
    }

    #[test]
    fn fixed_boundaries_are_range_independent_and_include_the_end_second() {
        let db = Database::memory().unwrap();
        let start = ts(2026, 9, 27, 10, 2);
        let end = ts(2026, 9, 27, 10, 5);
        insert(&db, "before", start - 1, 100, 0, 0, 200, "5", "test");
        insert(&db, "start", start, 100, 0, 0, 200, "1", "test");
        insert(&db, "end", end, 100, 0, 0, 200, "2", "test");
        insert(&db, "after", end + 1, 100, 0, 0, 200, "7", "test");
        let response = db
            .get_grouped_usage_trends(
                Some(start),
                Some(end),
                None,
                None,
                None,
                TrendGrouping {
                    interval: 5,
                    unit: TrendUnit::Minute,
                },
            )
            .unwrap();
        assert_eq!(response.buckets.len(), 2);
        assert_eq!(response.buckets[0].start_date, ts(2026, 9, 27, 10, 0));
        assert_eq!(response.buckets[0].totals.total_cost, "1.000000");
        assert_eq!(response.buckets[1].totals.total_cost, "2.000000");
    }

    #[test]
    fn calendar_months_and_monday_weeks_use_real_calendar_boundaries() {
        let start = Utc
            .with_ymd_and_hms(2024, 2, 15, 12, 0, 0)
            .unwrap()
            .timestamp();
        let month = bucket_boundaries(
            start,
            start,
            TrendGrouping {
                interval: 1,
                unit: TrendUnit::Month,
            },
            &Utc,
        )
        .unwrap();
        assert_eq!(month[0].1 - month[0].0, 29 * 86400);
        assert_eq!(Utc.timestamp_opt(month[0].0, 0).unwrap().day(), 1);
        for interval in TREND_INTERVALS {
            let weeks = bucket_boundaries(
                start,
                start,
                TrendGrouping {
                    interval,
                    unit: TrendUnit::Week,
                },
                &Utc,
            )
            .unwrap();
            assert_eq!(
                Utc.timestamp_opt(weeks[0].0, 0).unwrap().weekday(),
                chrono::Weekday::Mon
            );
            assert_eq!(weeks[0].1 - weeks[0].0, i64::from(interval) * 7 * 86400);
        }
    }

    #[test]
    fn daylight_saving_gaps_folds_and_midnight_folds_do_not_lose_time() {
        use chrono_tz::America::{Havana, New_York};
        for (month, day, hours) in [(3, 8, 23), (11, 1, 25)] {
            let start = New_York
                .with_ymd_and_hms(2026, month, day, 0, 0, 0)
                .earliest()
                .unwrap()
                .timestamp();
            let end = New_York
                .with_ymd_and_hms(2026, month, day, 23, 59, 59)
                .latest()
                .unwrap()
                .timestamp();
            let days = bucket_boundaries(start, end, TrendGrouping::default(), &New_York).unwrap();
            assert_eq!(days.len(), 1);
            assert_eq!(days[0].1 - days[0].0, hours * 3600);
            let buckets = bucket_boundaries(
                start,
                end,
                TrendGrouping {
                    interval: 1,
                    unit: TrendUnit::Hour,
                },
                &New_York,
            )
            .unwrap();
            assert_eq!(buckets.len(), hours as usize);
            assert!(buckets.windows(2).all(|pair| pair[0].1 == pair[1].0));
            if hours == 25 {
                assert_eq!(
                    buckets
                        .iter()
                        .filter(|(from, _)| New_York.timestamp_opt(*from, 0).unwrap().hour() == 1)
                        .count(),
                    2
                );
            }
        }
        let start = Havana
            .with_ymd_and_hms(2026, 11, 1, 0, 0, 0)
            .earliest()
            .unwrap()
            .timestamp();
        let end = Havana
            .with_ymd_and_hms(2026, 11, 1, 23, 59, 59)
            .latest()
            .unwrap()
            .timestamp();
        assert_eq!(
            bucket_boundaries(start, end, TrendGrouping::default(), &Havana)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn daily_archives_are_included_only_when_reconstructible() {
        let db = Database::memory().unwrap();
        db.conn.lock().unwrap().execute(
            "INSERT INTO usage_daily_rollups (date,app_type,provider_id,model,request_count,success_count,
             input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,input_token_semantics,total_cost_usd)
             VALUES ('2026-09-21','codex','copilot','archived',4,3,100,50,800,100,2,'1.25')",[],
        ).unwrap();
        let start = ts(2026, 9, 21, 0, 0);
        let end = ts(2026, 9, 21, 23, 59) + 59;
        let day = db
            .get_grouped_usage_trends(
                Some(start),
                Some(end),
                None,
                None,
                None,
                TrendGrouping::default(),
            )
            .unwrap();
        assert_eq!(day.buckets[0].totals.total_cost, "1.250000");
        assert_eq!(day.buckets[0].success_rate, Some(0.75));
        let fine = db
            .get_grouped_usage_trends(
                Some(start),
                Some(end),
                None,
                None,
                None,
                TrendGrouping {
                    interval: 1,
                    unit: TrendUnit::Hour,
                },
            )
            .unwrap();
        assert!(fine.has_incomplete_rollup_data);
        assert!(fine
            .buckets
            .iter()
            .all(|b| b.incomplete && b.success_rate.is_none()));
        let partial = db
            .get_grouped_usage_trends(
                Some(start + 3600),
                Some(end),
                None,
                None,
                None,
                TrendGrouping::default(),
            )
            .unwrap();
        assert!(partial.has_incomplete_rollup_data);
        assert_eq!(partial.buckets[0].totals.request_count, 0);
    }

    #[test]
    fn invalid_preferences_default_and_excessive_buckets_fail_explicitly() {
        let db = Database::memory().unwrap();
        assert_eq!(
            db.get_usage_trend_grouping().unwrap(),
            TrendGrouping::default()
        );
        for invalid in [
            "broken",
            r#"{"interval":2,"unit":"hour"}"#,
            r#"{"interval":1,"unit":"unknown"}"#,
        ] {
            db.set_setting("usage_trend_grouping", invalid).unwrap();
            assert_eq!(
                db.get_usage_trend_grouping().unwrap(),
                TrendGrouping::default()
            );
        }
        let saved = TrendGrouping {
            interval: 3,
            unit: TrendUnit::Hour,
        };
        db.set_usage_trend_grouping(saved).unwrap();
        assert_eq!(db.get_usage_trend_grouping().unwrap(), saved);
        assert_eq!(
            db.get_usage_date_range().unwrap(),
            crate::settings::UsageDateRange::default()
        );
        db.set_setting("usage_date_range", r#"{"preset":"custom"}"#)
            .unwrap();
        assert_eq!(
            db.get_usage_date_range().unwrap(),
            crate::settings::UsageDateRange::default()
        );
        let custom: crate::settings::UsageDateRange = serde_json::from_str(
            r#"{"preset":"custom","customStartDate":100,"customEndDate":200}"#,
        )
        .unwrap();
        db.set_usage_date_range(custom.clone()).unwrap();
        assert_eq!(db.get_usage_date_range().unwrap(), custom);
        assert!(bucket_boundaries(
            0,
            86400 * 30,
            TrendGrouping {
                interval: 1,
                unit: TrendUnit::Minute
            },
            &Utc
        )
        .is_err());
        assert!(bucket_boundaries(10, 0, TrendGrouping::default(), &Utc).is_err());
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendBucket {
    #[serde(flatten)]
    pub totals: DailyStats,
    pub start_date: i64,
    pub end_date: i64,
    pub read_cache_hit_rate: Option<f64>,
    pub success_rate: Option<f64>,
    pub incomplete: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTrends {
    pub grouping: TrendGrouping,
    pub buckets: Vec<TrendBucket>,
    pub has_incomplete_rollup_data: bool,
    pub total_cost: Option<String>,
    pub start_date: i64,
    pub end_date: i64,
}

impl Database {
    pub fn get_grouped_usage_trends(
        &self,
        start_date: Option<i64>,
        end_date: Option<i64>,
        app_type: Option<&str>,
        provider_name: Option<&str>,
        model: Option<&str>,
        grouping: TrendGrouping,
    ) -> Result<UsageTrends, AppError> {
        crate::copilot_bridge::require_codex(app_type.unwrap_or("codex"))?;
        let end = end_date.unwrap_or_else(|| Local::now().timestamp());
        let start = start_date.unwrap_or_else(|| end.saturating_sub(86400));
        let boundaries = bucket_boundaries(start, end, grouping, &Local)?;
        let mut rows = Vec::with_capacity(boundaries.len());
        for &(from, to) in &boundaries {
            let clipped_start = from.max(start);
            let clipped_end = (to - 1).min(end);
            let covered = compute_rollup_date_bounds(Some(clipped_start), Some(clipped_end))?;
            let local_start = Local
                .timestamp_opt(clipped_start, 0)
                .single()
                .ok_or_else(invalid_time)?
                .date_naive();
            let local_end = Local
                .timestamp_opt(clipped_end, 0)
                .single()
                .ok_or_else(invalid_time)?
                .date_naive();
            rows.push(serde_json::json!({
                "start":clipped_start, "end":clipped_end,
                "firstDay":local_start.to_string(), "lastDay":local_end.to_string(),
                "rollupStart": if grouping.calendar() && !covered.is_empty { covered.start } else { None },
                "rollupEnd": if grouping.calendar() && !covered.is_empty { covered.end } else { None },
            }));
        }
        let conn = lock_conn!(self.conn);
        let mut parameters: Vec<Box<dyn ToSql>> = vec![Box::new(
            serde_json::to_string(&rows).map_err(|_| invalid_time())?,
        )];
        let mut detail_conditions = vec![effective_usage_log_filter("l")];
        push_provider_model_filters(
            &mut detail_conditions,
            &mut parameters,
            "l",
            "p",
            provider_name,
            model,
        );
        let mut rollup_conditions = vec![effective_usage_rollup_filter("r")];
        push_provider_model_filters(
            &mut rollup_conditions,
            &mut parameters,
            "r",
            "p2",
            provider_name,
            model,
        );
        let detail_join = if provider_name.is_some() {
            providers_join("l", "p")
        } else {
            String::new()
        };
        let rollup_join = if provider_name.is_some() {
            providers_join("r", "p2")
        } else {
            String::new()
        };
        let fresh = fresh_input_sql("l");
        let rolled_fresh = fresh_input_sql("r");
        let eligible = "r.date >= b.rollup_start AND r.date <= b.rollup_end";
        let sql = format!(
            "WITH buckets AS (
                SELECT CAST(key AS INTEGER) AS idx,
                  json_extract(value,'$.start') AS start_ts, json_extract(value,'$.end') AS end_ts,
                  json_extract(value,'$.firstDay') AS first_day, json_extract(value,'$.lastDay') AS last_day,
                  json_extract(value,'$.rollupStart') AS rollup_start, json_extract(value,'$.rollupEnd') AS rollup_end
                FROM json_each(?)
             ), details AS (
                SELECT b.idx, COUNT(*) AS requests,
                  SUM(CASE WHEN l.status_code >= 200 AND l.status_code < 300 THEN 1 ELSE 0 END) AS successes,
                  SUM(CAST(l.total_cost_usd AS REAL)) AS cost,
                  SUM({fresh}) AS fresh, SUM(l.output_tokens) AS output,
                  SUM(l.cache_read_tokens) AS cached, SUM(l.cache_creation_tokens) AS written
                FROM buckets b JOIN proxy_request_logs l ON l.created_at >= b.start_ts AND l.created_at <= b.end_ts
                {detail_join} WHERE {detail_filter} GROUP BY b.idx
             ), archives AS (
                SELECT b.idx,
                  SUM(CASE WHEN {eligible} THEN r.request_count ELSE 0 END) AS requests,
                  SUM(CASE WHEN {eligible} THEN r.success_count ELSE 0 END) AS successes,
                  SUM(CASE WHEN {eligible} THEN CAST(r.total_cost_usd AS REAL) ELSE 0 END) AS cost,
                  SUM(CASE WHEN {eligible} THEN {rolled_fresh} ELSE 0 END) AS fresh,
                  SUM(CASE WHEN {eligible} THEN r.output_tokens ELSE 0 END) AS output,
                  SUM(CASE WHEN {eligible} THEN r.cache_read_tokens ELSE 0 END) AS cached,
                  SUM(CASE WHEN {eligible} THEN r.cache_creation_tokens ELSE 0 END) AS written,
                  MAX(CASE WHEN COALESCE(({eligible}),0) = 0 AND r.request_count > 0 THEN 1 ELSE 0 END) AS incomplete
                FROM buckets b JOIN usage_daily_rollups r ON r.date >= b.first_day AND r.date <= b.last_day
                {rollup_join} WHERE {rollup_filter} GROUP BY b.idx
             )
             SELECT b.idx, COALESCE(d.requests,0)+COALESCE(a.requests,0),
               COALESCE(d.successes,0)+COALESCE(a.successes,0),
               COALESCE(d.cost,0)+COALESCE(a.cost,0),
               COALESCE(d.fresh,0)+COALESCE(a.fresh,0), COALESCE(d.output,0)+COALESCE(a.output,0),
               COALESCE(d.cached,0)+COALESCE(a.cached,0), COALESCE(d.written,0)+COALESCE(a.written,0),
               COALESCE(a.incomplete,0)
             FROM buckets b LEFT JOIN details d ON d.idx=b.idx LEFT JOIN archives a ON a.idx=b.idx ORDER BY b.idx",
            detail_filter=detail_conditions.join(" AND "), rollup_filter=rollup_conditions.join(" AND "),
        );
        let refs: Vec<&dyn ToSql> = parameters.iter().map(|p| p.as_ref()).collect();
        let mut statement = conn.prepare(&sql)?;
        let mut total_cost = 0.0_f64;
        let buckets = statement
            .query_map(refs.as_slice(), |row| {
                let index: usize = row.get(0)?;
                let (from, to) = boundaries[index];
                let requests: u64 = row.get(1)?;
                let successes: u64 = row.get(2)?;
                let input: u64 = row.get(4)?;
                let output: u64 = row.get(5)?;
                let cached: u64 = row.get(6)?;
                let written: u64 = row.get(7)?;
                let incomplete: bool = row.get(8)?;
                let cost: f64 = row.get(3)?;
                total_cost += cost;
                let denominator = input + cached + written;
                Ok(TrendBucket {
                    totals: DailyStats {
                        date: Local
                            .timestamp_opt(from, 0)
                            .single()
                            .expect("validated bucket")
                            .to_rfc3339(),
                        request_count: requests,
                        total_cost: format!("{cost:.6}"),
                        total_tokens: denominator + output,
                        total_input_tokens: input,
                        total_output_tokens: output,
                        total_cache_read_tokens: cached,
                        total_cache_creation_tokens: written,
                    },
                    start_date: from,
                    end_date: to,
                    incomplete,
                    read_cache_hit_rate: (!incomplete && denominator > 0)
                        .then(|| cached as f64 / denominator as f64),
                    success_rate: (!incomplete && requests > 0)
                        .then(|| successes as f64 / requests as f64),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let has_incomplete_rollup_data = buckets.iter().any(|bucket| bucket.incomplete);
        Ok(UsageTrends {
            grouping,
            start_date: start,
            end_date: end,
            has_incomplete_rollup_data,
            total_cost: (!has_incomplete_rollup_data).then(|| format!("{total_cost:.6}")),
            buckets,
        })
    }
}
