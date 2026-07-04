//! SQLite 实现：基于 `sqlx::SqlitePool` 的 `RequestEventRepository`。
//!
//! 使用运行时 query（`sqlx::query`/`sqlx::query_as`），
//! 不使用编译期宏 `query!`/`query_as!`（避免需要 `SQLX_OFFLINE` + `.sqlx/` 数据或 `DATABASE_URL`）。

use crate::observability::{
    RequestEvent, RequestStatus, SecurityEvent, SecurityEventKind, Severity,
};
use crate::store::error::{StoreError, decode_error};
use crate::store::repository::{
    RequestEventFilter, RequestEventRepository, SecurityEventFilter, SecurityEventRepository,
};
use chrono::Utc;
use sqlx::Row;
use sqlx::sqlite::{SqlitePool, SqliteRow};

/// SQLite-backed 请求事件 repository。
#[derive(Debug, Clone)]
pub struct SqliteRequestEventRepository {
    pool: SqlitePool,
}

impl SqliteRequestEventRepository {
    /// 从已有连接池创建。
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// 将 `RequestStatus` 编码为 DB 列 `(status_kind, status_code)`。
fn encode_status(status: &RequestStatus) -> (&'static str, Option<i64>) {
    match status {
        RequestStatus::Success => ("success", None),
        RequestStatus::UpstreamError(code) => ("upstream_error", Some(i64::from(*code))),
        RequestStatus::Timeout => ("timeout", None),
        RequestStatus::ConnectionFailed => ("connection_failed", None),
        RequestStatus::Limited => ("limited", None),
    }
}

/// 从 DB 行解码 `RequestStatus`。
fn decode_status(row: &SqliteRow) -> Result<RequestStatus, StoreError> {
    let kind: String = row.try_get("status_kind").map_err(StoreError::from)?;
    let status = match kind.as_str() {
        "success" => RequestStatus::Success,
        "upstream_error" => {
            let code: Option<i64> = row.try_get("status_code").map_err(StoreError::from)?;
            RequestStatus::UpstreamError(code.map(|c| c as u16).unwrap_or(0))
        }
        "timeout" => RequestStatus::Timeout,
        "connection_failed" => RequestStatus::ConnectionFailed,
        "limited" => RequestStatus::Limited,
        other => {
            return Err(StoreError::Query(decode_error(format!(
                "unknown status_kind: {other}"
            ))));
        }
    };
    Ok(status)
}

fn row_to_event(row: SqliteRow) -> Result<RequestEvent, StoreError> {
    let timestamp_str: String = row.try_get("timestamp").map_err(StoreError::from)?;
    let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
        .map_err(|e| StoreError::Query(decode_error(format!("invalid timestamp: {e}"))))?
        .with_timezone(&Utc);

    let status = decode_status(&row)?;

    Ok(RequestEvent {
        timestamp,
        request_id: row.try_get("request_id").map_err(StoreError::from)?,
        proxy_key_id: row.try_get("proxy_key_id").map_err(StoreError::from)?,
        resource_id: row.try_get("resource_id").map_err(StoreError::from)?,
        tool_name: row.try_get("tool_name").map_err(StoreError::from)?,
        upstream_key_ref: row.try_get("upstream_key_ref").map_err(StoreError::from)?,
        status,
        latency_ms: row
            .try_get::<i64, _>("latency_ms")
            .map_err(StoreError::from)? as u32,
        request_units: row
            .try_get::<i64, _>("request_units")
            .map_err(StoreError::from)? as u32,
        retry_count: row
            .try_get::<i64, _>("retry_count")
            .map_err(StoreError::from)? as u8,
        rate_limited: row
            .try_get::<i64, _>("rate_limited")
            .map_err(StoreError::from)?
            != 0,
        queued_ms: row
            .try_get::<i64, _>("queued_ms")
            .map_err(StoreError::from)? as u32,
    })
}

impl RequestEventRepository for SqliteRequestEventRepository {
    async fn insert_event(&self, event: &RequestEvent) -> Result<(), StoreError> {
        let (status_kind, status_code) = encode_status(&event.status);
        let timestamp = event.timestamp.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO request_events
                (timestamp, request_id, proxy_key_id, resource_id, tool_name,
                 upstream_key_ref, status_kind, status_code, latency_ms,
                 request_units, retry_count, rate_limited, queued_ms)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&timestamp)
        .bind(&event.request_id)
        .bind(&event.proxy_key_id)
        .bind(&event.resource_id)
        .bind(&event.tool_name)
        .bind(&event.upstream_key_ref)
        .bind(status_kind)
        .bind(status_code)
        .bind(i64::from(event.latency_ms))
        .bind(i64::from(event.request_units))
        .bind(i64::from(event.retry_count))
        .bind(i64::from(event.rate_limited))
        .bind(i64::from(event.queued_ms))
        .execute(&self.pool)
        .await
        .map_err(StoreError::from)?;

        Ok(())
    }

    async fn list_events(
        &self,
        filter: &RequestEventFilter,
        limit: u32,
    ) -> Result<Vec<RequestEvent>, StoreError> {
        let mut sql = String::from(
            r#"
            SELECT timestamp, request_id, proxy_key_id, resource_id, tool_name,
                   upstream_key_ref, status_kind, status_code, latency_ms,
                   request_units, retry_count, rate_limited, queued_ms
            FROM request_events
            WHERE 1=1
            "#,
        );

        if filter.proxy_key_id.is_some() {
            sql.push_str(" AND proxy_key_id = ?");
        }
        if filter.resource_id.is_some() {
            sql.push_str(" AND resource_id = ?");
        }
        if filter.from.is_some() {
            sql.push_str(" AND timestamp >= ?");
        }
        if filter.to.is_some() {
            sql.push_str(" AND timestamp < ?");
        }
        sql.push_str(" ORDER BY timestamp DESC LIMIT ?");

        // 所有动态拼接部分均为静态 SQL 片段（条件子句），
        // 用户输入全部通过 bind 参数传递，不存在注入风险。
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()));

        if let Some(ref proxy_key_id) = filter.proxy_key_id {
            query = query.bind(proxy_key_id);
        }
        if let Some(ref resource_id) = filter.resource_id {
            query = query.bind(resource_id);
        }
        if let Some(from) = filter.from {
            query = query.bind(from.to_rfc3339());
        }
        if let Some(to) = filter.to {
            query = query.bind(to.to_rfc3339());
        }
        query = query.bind(i64::from(limit));

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::from)?;

        rows.into_iter().map(row_to_event).collect()
    }
}

// ── SecurityEvent 编码/解码 ──

fn encode_security_kind(kind: &SecurityEventKind) -> &'static str {
    match kind {
        SecurityEventKind::IntegrityToolChanged => "integrity_tool_changed",
        SecurityEventKind::IntegrityToolAdded => "integrity_tool_added",
        SecurityEventKind::IntegrityToolRemoved => "integrity_tool_removed",
        SecurityEventKind::IntegrityHintFlipped => "integrity_hint_flipped",
        SecurityEventKind::ContentDefenseFlag => "content_defense_flag",
    }
}

fn decode_security_kind(s: &str) -> Result<SecurityEventKind, StoreError> {
    match s {
        "integrity_tool_changed" => Ok(SecurityEventKind::IntegrityToolChanged),
        "integrity_tool_added" => Ok(SecurityEventKind::IntegrityToolAdded),
        "integrity_tool_removed" => Ok(SecurityEventKind::IntegrityToolRemoved),
        "integrity_hint_flipped" => Ok(SecurityEventKind::IntegrityHintFlipped),
        "content_defense_flag" => Ok(SecurityEventKind::ContentDefenseFlag),
        other => Err(StoreError::Query(decode_error(format!(
            "unknown security event kind: {other}"
        )))),
    }
}

fn encode_severity(sev: &Severity) -> &'static str {
    match sev {
        Severity::Info => "info",
        Severity::Warn => "warn",
        Severity::Error => "error",
    }
}

fn decode_severity(s: &str) -> Result<Severity, StoreError> {
    match s {
        "info" => Ok(Severity::Info),
        "warn" => Ok(Severity::Warn),
        "error" => Ok(Severity::Error),
        other => Err(StoreError::Query(decode_error(format!(
            "unknown severity: {other}"
        )))),
    }
}

fn row_to_security_event(row: SqliteRow) -> Result<SecurityEvent, StoreError> {
    let timestamp_str: String = row.try_get("timestamp").map_err(StoreError::from)?;
    let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
        .map_err(|e| StoreError::Query(decode_error(format!("invalid timestamp: {e}"))))?
        .with_timezone(&Utc);

    let kind_str: String = row.try_get("kind").map_err(StoreError::from)?;
    let kind = decode_security_kind(&kind_str)?;

    let severity_str: String = row.try_get("severity").map_err(StoreError::from)?;
    let severity = decode_severity(&severity_str)?;

    let details_str: String = row.try_get("details_json").map_err(StoreError::from)?;
    let details: serde_json::Value = serde_json::from_str(&details_str)
        .map_err(|e| StoreError::Query(decode_error(format!("invalid details JSON: {e}"))))?;

    let tool_name: Option<String> = row.try_get("tool_name").map_err(StoreError::from)?;

    Ok(SecurityEvent {
        timestamp,
        resource_id: row.try_get("resource_id").map_err(StoreError::from)?,
        tool_name,
        kind,
        severity,
        details,
    })
}

impl SecurityEventRepository for SqliteRequestEventRepository {
    async fn insert_security_event(&self, event: &SecurityEvent) -> Result<(), StoreError> {
        let kind = encode_security_kind(&event.kind);
        let severity = encode_severity(&event.severity);
        let timestamp = event.timestamp.to_rfc3339();
        let details = serde_json::to_string(&event.details).unwrap_or_else(|_| "{}".to_string());
        let tool_name: Option<&str> = event.tool_name.as_deref();

        sqlx::query(
            r#"
            INSERT INTO security_events
                (timestamp, resource_id, tool_name, kind, severity, details_json)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&timestamp)
        .bind(&event.resource_id)
        .bind(tool_name)
        .bind(kind)
        .bind(severity)
        .bind(&details)
        .execute(&self.pool)
        .await
        .map_err(StoreError::from)?;

        Ok(())
    }

    async fn list_security_events(
        &self,
        filter: &SecurityEventFilter,
        limit: u32,
    ) -> Result<Vec<SecurityEvent>, StoreError> {
        let mut sql = String::from(
            r#"
            SELECT timestamp, resource_id, tool_name, kind, severity, details_json
            FROM security_events
            WHERE 1=1
            "#,
        );

        if filter.resource_id.is_some() {
            sql.push_str(" AND resource_id = ?");
        }
        if filter.kind.is_some() {
            sql.push_str(" AND kind = ?");
        }
        if filter.from.is_some() {
            sql.push_str(" AND timestamp >= ?");
        }
        if filter.to.is_some() {
            sql.push_str(" AND timestamp < ?");
        }
        sql.push_str(" ORDER BY timestamp DESC LIMIT ?");

        // 所有动态拼接部分均为静态 SQL 片段（条件子句），
        // 用户输入全部通过 bind 参数传递，不存在注入风险。
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()));

        if let Some(ref resource_id) = filter.resource_id {
            query = query.bind(resource_id);
        }
        if let Some(kind) = &filter.kind {
            query = query.bind(encode_security_kind(kind));
        }
        if let Some(from) = filter.from {
            query = query.bind(from.to_rfc3339());
        }
        if let Some(to) = filter.to {
            query = query.bind(to.to_rfc3339());
        }
        query = query.bind(i64::from(limit));

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::from)?;

        rows.into_iter().map(row_to_security_event).collect()
    }
}
