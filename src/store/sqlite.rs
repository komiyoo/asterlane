//! SQLite 实现：基于 `sqlx::SqlitePool` 的 `RequestEventRepository`。
//!
//! 使用运行时 query（`sqlx::query`/`sqlx::query_as`），
//! 不使用编译期宏 `query!`/`query_as!`（避免需要 `SQLX_OFFLINE` + `.sqlx/` 数据或 `DATABASE_URL`）。

use crate::observability::{
    RequestEvent, RequestStatus, SecurityEvent, SecurityEventKind, Severity,
};
use crate::store::error::{StoreError, decode_error};
use crate::store::repository::{
    AggregationDimension, AggregationFilter, AggregationRepository, OverallStats, ProxyKeyRecord,
    ProxyKeyRepository, RequestEventFilter, RequestEventRepository, Resource, ResourceRepository,
    SecurityEventFilter, SecurityEventRepository, UpstreamKeyRecord, UpstreamKeyRepository,
    UsageBucket, UsageBucketFilter, UsageBucketRepository, UsageSummary,
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

// ── Resource ──

fn row_to_resource(row: SqliteRow) -> Result<Resource, StoreError> {
    Ok(Resource {
        id: row.try_get("id").map_err(StoreError::from)?,
        domain: row.try_get("domain").map_err(StoreError::from)?,
        provider: row.try_get("provider").map_err(StoreError::from)?,
        base_url: row.try_get("base_url").map_err(StoreError::from)?,
        description: row.try_get("description").map_err(StoreError::from)?,
        config_json: row.try_get("config_json").map_err(StoreError::from)?,
        created_at: row.try_get("created_at").map_err(StoreError::from)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::from)?,
    })
}

impl ResourceRepository for SqliteRequestEventRepository {
    async fn insert_resource(&self, resource: &Resource) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO resources (id, domain, provider, base_url, description, config_json)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&resource.id)
        .bind(&resource.domain)
        .bind(&resource.provider)
        .bind(&resource.base_url)
        .bind(&resource.description)
        .bind(&resource.config_json)
        .execute(&self.pool)
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn get_resource(&self, id: &str) -> Result<Resource, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, domain, provider, base_url, description, config_json, created_at, updated_at
            FROM resources WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::from)?
        .ok_or_else(|| StoreError::NotFound(format!("resource {id}")))?;
        row_to_resource(row)
    }

    async fn list_resources(&self) -> Result<Vec<Resource>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, domain, provider, base_url, description, config_json, created_at, updated_at
            FROM resources ORDER BY id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::from)?;
        rows.into_iter().map(row_to_resource).collect()
    }

    async fn delete_resource(&self, id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM resources WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }
}

// ── ProxyKey ──

fn row_to_proxy_key(row: SqliteRow) -> Result<ProxyKeyRecord, StoreError> {
    Ok(ProxyKeyRecord {
        id: row.try_get("id").map_err(StoreError::from)?,
        display_name: row.try_get("display_name").map_err(StoreError::from)?,
        default_tool_page_size: row
            .try_get("default_tool_page_size")
            .map_err(StoreError::from)?,
        scope_json: row.try_get("scope_json").map_err(StoreError::from)?,
        created_at: row.try_get("created_at").map_err(StoreError::from)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::from)?,
    })
}

impl ProxyKeyRepository for SqliteRequestEventRepository {
    async fn insert_proxy_key(&self, key: &ProxyKeyRecord) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO proxy_keys (id, display_name, default_tool_page_size, scope_json)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(&key.id)
        .bind(&key.display_name)
        .bind(key.default_tool_page_size)
        .bind(&key.scope_json)
        .execute(&self.pool)
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn get_proxy_key(&self, id: &str) -> Result<ProxyKeyRecord, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, display_name, default_tool_page_size, scope_json, created_at, updated_at
            FROM proxy_keys WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::from)?
        .ok_or_else(|| StoreError::NotFound(format!("proxy_key {id}")))?;
        row_to_proxy_key(row)
    }

    async fn list_proxy_keys(&self) -> Result<Vec<ProxyKeyRecord>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, display_name, default_tool_page_size, scope_json, created_at, updated_at
            FROM proxy_keys ORDER BY id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::from)?;
        rows.into_iter().map(row_to_proxy_key).collect()
    }

    async fn delete_proxy_key(&self, id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM proxy_keys WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }
}

// ── UpstreamKey ──

fn row_to_upstream_key(row: SqliteRow) -> Result<UpstreamKeyRecord, StoreError> {
    Ok(UpstreamKeyRecord {
        id: row.try_get("id").map_err(StoreError::from)?,
        resource_id: row.try_get("resource_id").map_err(StoreError::from)?,
        secret_ref: row.try_get("secret_ref").map_err(StoreError::from)?,
        weight: row.try_get("weight").map_err(StoreError::from)?,
        health_state: row.try_get("health_state").map_err(StoreError::from)?,
        cooldown_until: row.try_get("cooldown_until").map_err(StoreError::from)?,
        created_at: row.try_get("created_at").map_err(StoreError::from)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::from)?,
    })
}

impl UpstreamKeyRepository for SqliteRequestEventRepository {
    async fn insert_upstream_key(&self, key: &UpstreamKeyRecord) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO upstream_keys (id, resource_id, secret_ref, weight, health_state, cooldown_until)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&key.id)
        .bind(&key.resource_id)
        .bind(&key.secret_ref)
        .bind(key.weight)
        .bind(&key.health_state)
        .bind(&key.cooldown_until)
        .execute(&self.pool)
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn get_upstream_key(&self, id: &str) -> Result<UpstreamKeyRecord, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, resource_id, secret_ref, weight, health_state, cooldown_until,
                   created_at, updated_at
            FROM upstream_keys WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::from)?
        .ok_or_else(|| StoreError::NotFound(format!("upstream_key {id}")))?;
        row_to_upstream_key(row)
    }

    async fn list_upstream_keys_for_resource(
        &self,
        resource_id: &str,
    ) -> Result<Vec<UpstreamKeyRecord>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, resource_id, secret_ref, weight, health_state, cooldown_until,
                   created_at, updated_at
            FROM upstream_keys WHERE resource_id = ? ORDER BY id
            "#,
        )
        .bind(resource_id)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::from)?;
        rows.into_iter().map(row_to_upstream_key).collect()
    }

    async fn update_upstream_key_health(
        &self,
        id: &str,
        health_state: &str,
        cooldown_until: Option<&str>,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE upstream_keys
            SET health_state = ?, cooldown_until = ?,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ?
            "#,
        )
        .bind(health_state)
        .bind(cooldown_until)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_upstream_key(&self, id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM upstream_keys WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }
}

// ── UsageBucket ──

fn row_to_usage_bucket(row: SqliteRow) -> Result<UsageBucket, StoreError> {
    Ok(UsageBucket {
        bucket_start: row.try_get("bucket_start").map_err(StoreError::from)?,
        granularity: row.try_get("granularity").map_err(StoreError::from)?,
        proxy_key_id: row.try_get("proxy_key_id").map_err(StoreError::from)?,
        resource_id: row.try_get("resource_id").map_err(StoreError::from)?,
        tool_name: row.try_get("tool_name").map_err(StoreError::from)?,
        upstream_key_ref: row.try_get("upstream_key_ref").map_err(StoreError::from)?,
        status: row.try_get("status").map_err(StoreError::from)?,
        request_count: row.try_get("request_count").map_err(StoreError::from)?,
        total_units: row.try_get("total_units").map_err(StoreError::from)?,
        error_count: row.try_get("error_count").map_err(StoreError::from)?,
        rate_limit_hits: row.try_get("rate_limit_hits").map_err(StoreError::from)?,
        total_latency_ms: row.try_get("total_latency_ms").map_err(StoreError::from)?,
        total_queued_ms: row.try_get("total_queued_ms").map_err(StoreError::from)?,
    })
}

impl UsageBucketRepository for SqliteRequestEventRepository {
    async fn upsert_bucket(&self, bucket: &UsageBucket) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO usage_buckets
                (bucket_start, granularity, proxy_key_id, resource_id, tool_name,
                 upstream_key_ref, status, request_count, total_units, error_count,
                 rate_limit_hits, total_latency_ms, total_queued_ms)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT (bucket_start, granularity, proxy_key_id, resource_id,
                         tool_name, upstream_key_ref, status)
            DO UPDATE SET
                request_count   = request_count   + excluded.request_count,
                total_units     = total_units      + excluded.total_units,
                error_count     = error_count      + excluded.error_count,
                rate_limit_hits = rate_limit_hits   + excluded.rate_limit_hits,
                total_latency_ms = total_latency_ms + excluded.total_latency_ms,
                total_queued_ms = total_queued_ms   + excluded.total_queued_ms
            "#,
        )
        .bind(&bucket.bucket_start)
        .bind(&bucket.granularity)
        .bind(&bucket.proxy_key_id)
        .bind(&bucket.resource_id)
        .bind(&bucket.tool_name)
        .bind(&bucket.upstream_key_ref)
        .bind(&bucket.status)
        .bind(bucket.request_count)
        .bind(bucket.total_units)
        .bind(bucket.error_count)
        .bind(bucket.rate_limit_hits)
        .bind(bucket.total_latency_ms)
        .bind(bucket.total_queued_ms)
        .execute(&self.pool)
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn query_buckets(
        &self,
        filter: &UsageBucketFilter,
        limit: u32,
    ) -> Result<Vec<UsageBucket>, StoreError> {
        let mut sql = String::from(
            r#"
            SELECT bucket_start, granularity, proxy_key_id, resource_id, tool_name,
                   upstream_key_ref, status, request_count, total_units, error_count,
                   rate_limit_hits, total_latency_ms, total_queued_ms
            FROM usage_buckets
            WHERE 1=1
            "#,
        );

        if filter.proxy_key_id.is_some() {
            sql.push_str(" AND proxy_key_id = ?");
        }
        if filter.resource_id.is_some() {
            sql.push_str(" AND resource_id = ?");
        }
        if filter.tool_name.is_some() {
            sql.push_str(" AND tool_name = ?");
        }
        if filter.granularity.is_some() {
            sql.push_str(" AND granularity = ?");
        }
        if filter.from.is_some() {
            sql.push_str(" AND bucket_start >= ?");
        }
        if filter.to.is_some() {
            sql.push_str(" AND bucket_start < ?");
        }
        sql.push_str(" ORDER BY bucket_start DESC LIMIT ?");

        // 所有动态拼接部分均为静态 SQL 片段（条件子句），
        // 用户输入全部通过 bind 参数传递，不存在注入风险。
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()));

        if let Some(ref proxy_key_id) = filter.proxy_key_id {
            query = query.bind(proxy_key_id);
        }
        if let Some(ref resource_id) = filter.resource_id {
            query = query.bind(resource_id);
        }
        if let Some(ref tool_name) = filter.tool_name {
            query = query.bind(tool_name);
        }
        if let Some(ref granularity) = filter.granularity {
            query = query.bind(granularity);
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

        rows.into_iter().map(row_to_usage_bucket).collect()
    }
}

// ── Aggregation ──

fn dimension_column(dim: AggregationDimension) -> &'static str {
    match dim {
        AggregationDimension::ProxyKey => "proxy_key_id",
        AggregationDimension::Resource => "resource_id",
        AggregationDimension::Tool => "tool_name",
        AggregationDimension::Status => "status_kind",
        AggregationDimension::Domain => {
            "CASE WHEN instr(tool_name, '__') > 0 THEN substr(tool_name, 1, instr(tool_name, '__') - 1) ELSE tool_name END"
        }
    }
}

fn append_aggregation_filter(sql: &mut String, filter: &AggregationFilter, time_col: &str) {
    if filter.proxy_key_id.is_some() {
        sql.push_str(" AND proxy_key_id = ?");
    }
    if filter.resource_id.is_some() {
        sql.push_str(" AND resource_id = ?");
    }
    if filter.from.is_some() {
        sql.push_str(&format!(" AND {time_col} >= ?"));
    }
    if filter.to.is_some() {
        sql.push_str(&format!(" AND {time_col} < ?"));
    }
}

fn row_to_usage_summary(row: SqliteRow) -> Result<UsageSummary, StoreError> {
    Ok(UsageSummary {
        dimension_value: row.try_get("dim_value").map_err(StoreError::from)?,
        request_count: row.try_get("request_count").map_err(StoreError::from)?,
        error_count: row.try_get("error_count").map_err(StoreError::from)?,
        total_units: row.try_get("total_units").map_err(StoreError::from)?,
        avg_latency_ms: row.try_get("avg_latency_ms").map_err(StoreError::from)?,
        rate_limit_hits: row.try_get("rate_limit_hits").map_err(StoreError::from)?,
    })
}

fn bind_aggregation_filter<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>,
    filter: &'q AggregationFilter,
    rfc_from: &'q Option<String>,
    rfc_to: &'q Option<String>,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments> {
    if let Some(pk) = &filter.proxy_key_id {
        query = query.bind(pk);
    }
    if let Some(rid) = &filter.resource_id {
        query = query.bind(rid);
    }
    if let Some(f) = rfc_from {
        query = query.bind(f.as_str());
    }
    if let Some(t) = rfc_to {
        query = query.bind(t.as_str());
    }
    query
}

impl AggregationRepository for SqliteRequestEventRepository {
    async fn summarize_by(
        &self,
        dimension: AggregationDimension,
        filter: &AggregationFilter,
        limit: u32,
    ) -> Result<Vec<UsageSummary>, StoreError> {
        let col = dimension_column(dimension);
        let mut sql = format!(
            r#"
            SELECT {col} AS dim_value,
                   COUNT(*) AS request_count,
                   SUM(CASE WHEN status_kind != 'success' THEN 1 ELSE 0 END) AS error_count,
                   SUM(request_units) AS total_units,
                   AVG(latency_ms) AS avg_latency_ms,
                   SUM(rate_limited) AS rate_limit_hits
            FROM request_events
            WHERE 1=1
            "#
        );
        append_aggregation_filter(&mut sql, filter, "timestamp");
        sql.push_str(&format!(
            " GROUP BY {col} ORDER BY request_count DESC LIMIT ?"
        ));

        let rfc_from = filter.from.map(|d| d.to_rfc3339());
        let rfc_to = filter.to.map(|d| d.to_rfc3339());
        let query = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()));
        let query = bind_aggregation_filter(query, filter, &rfc_from, &rfc_to);
        let query = query.bind(i64::from(limit));

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::from)?;

        rows.into_iter().map(row_to_usage_summary).collect()
    }

    async fn series_by_bucket(
        &self,
        granularity: &str,
        filter: &AggregationFilter,
        limit: u32,
    ) -> Result<Vec<UsageSummary>, StoreError> {
        // 读预聚合表；avg_latency 用总延迟/总请求（跨维度行的加权平均）。
        // request_count 每行 ≥ 1（absorb 至少一次），无除零。
        let mut sql = String::from(
            r#"
            SELECT bucket_start AS dim_value,
                   SUM(request_count) AS request_count,
                   SUM(error_count) AS error_count,
                   SUM(total_units) AS total_units,
                   CAST(SUM(total_latency_ms) AS REAL) / SUM(request_count) AS avg_latency_ms,
                   SUM(rate_limit_hits) AS rate_limit_hits
            FROM usage_buckets
            WHERE granularity = ?
            "#,
        );
        append_aggregation_filter(&mut sql, filter, "bucket_start");
        sql.push_str(" GROUP BY bucket_start ORDER BY bucket_start ASC LIMIT ?");

        let rfc_from = filter.from.map(|d| d.to_rfc3339());
        let rfc_to = filter.to.map(|d| d.to_rfc3339());
        let query = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(granularity);
        let query = bind_aggregation_filter(query, filter, &rfc_from, &rfc_to);
        let query = query.bind(i64::from(limit));

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::from)?;

        rows.into_iter().map(row_to_usage_summary).collect()
    }

    async fn overall_stats(&self, filter: &AggregationFilter) -> Result<OverallStats, StoreError> {
        let mut sql = String::from(
            r#"
            SELECT COUNT(*) AS total_requests,
                   SUM(CASE WHEN status_kind != 'success' THEN 1 ELSE 0 END) AS total_errors,
                   COUNT(DISTINCT tool_name) AS unique_tools,
                   COUNT(DISTINCT proxy_key_id) AS unique_proxy_keys,
                   COUNT(DISTINCT resource_id) AS unique_resources,
                   AVG(latency_ms) AS avg_latency_ms,
                   SUM(rate_limited) AS total_rate_limit_hits
            FROM request_events
            WHERE 1=1
            "#,
        );
        append_aggregation_filter(&mut sql, filter, "timestamp");

        let rfc_from = filter.from.map(|d| d.to_rfc3339());
        let rfc_to = filter.to.map(|d| d.to_rfc3339());
        let query = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()));
        let query = bind_aggregation_filter(query, filter, &rfc_from, &rfc_to);

        let row = query
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::from)?;

        Ok(OverallStats {
            total_requests: row.try_get("total_requests").map_err(StoreError::from)?,
            total_errors: row.try_get("total_errors").map_err(StoreError::from)?,
            unique_tools: row.try_get("unique_tools").map_err(StoreError::from)?,
            unique_proxy_keys: row.try_get("unique_proxy_keys").map_err(StoreError::from)?,
            unique_resources: row.try_get("unique_resources").map_err(StoreError::from)?,
            avg_latency_ms: row
                .try_get::<Option<f64>, _>("avg_latency_ms")
                .map_err(StoreError::from)?
                .unwrap_or(0.0),
            total_rate_limit_hits: row
                .try_get::<Option<i64>, _>("total_rate_limit_hits")
                .map_err(StoreError::from)?
                .unwrap_or(0),
        })
    }
}
