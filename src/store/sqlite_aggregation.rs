//! SQLite 实现：`UsageBucketRepository` 与 `AggregationRepository`。
//!
//! 从 `sqlite.rs` 拆出——用量桶与聚合查询独立于事件/实体 CRUD。

use crate::store::error::StoreError;
use crate::store::repository::{
    AggregationDimension, AggregationFilter, AggregationRepository, OverallStats, UsageBucket,
    UsageBucketFilter, UsageBucketRepository, UsageSummary,
};
use crate::store::sqlite::SqliteRequestEventRepository;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

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
        .execute(self.pool())
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
            .fetch_all(self.pool())
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
            .fetch_all(self.pool())
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
            .fetch_all(self.pool())
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
            .fetch_one(self.pool())
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
