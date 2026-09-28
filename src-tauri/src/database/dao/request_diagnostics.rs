//! Bounded request and response snapshots for failed Copilot calls.
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDiagnosticDetail {
    pub upstream_status: Option<u16>,
    pub failure_stage: Option<String>,
    pub request_headers: Option<String>,
    pub request_body: Option<String>,
    pub response_headers: Option<String>,
    pub response_body: Option<String>,
}

impl Database {
    pub(crate) fn save_request_diagnostics(
        &self,
        request_id: &str,
        detail: &RequestDiagnosticDetail,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO proxy_request_diagnostics
                (request_id, upstream_status, failure_stage,
                 request_headers, request_body, response_headers, response_body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(request_id) DO UPDATE SET
                upstream_status=excluded.upstream_status,
                failure_stage=excluded.failure_stage,
                request_headers=excluded.request_headers,
                request_body=excluded.request_body,
                response_headers=excluded.response_headers,
                response_body=excluded.response_body",
            params![
                request_id,
                detail.upstream_status,
                detail.failure_stage,
                detail.request_headers,
                detail.request_body,
                detail.response_headers,
                detail.response_body,
            ],
        )?;
        Ok(())
    }

    pub fn get_request_diagnostics(
        &self,
        request_id: &str,
    ) -> Result<Option<RequestDiagnosticDetail>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT d.upstream_status, d.failure_stage,
                    d.request_headers, d.request_body, d.response_headers, d.response_body
             FROM proxy_request_diagnostics d
             JOIN proxy_request_logs l ON l.request_id = d.request_id
             WHERE d.request_id = ?1 AND l.app_type = 'codex' AND l.status_code NOT BETWEEN 200 AND 299",
            [request_id],
            |row| {
                Ok(RequestDiagnosticDetail {
                    upstream_status: row.get(0)?,
                    failure_stage: row.get(1)?,
                    request_headers: row.get(2)?,
                    request_body: row.get(3)?,
                    response_headers: row.get(4)?,
                    response_body: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }
}
