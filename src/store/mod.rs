//! Store 模块：数据库抽象、迁移与 repository 实现
//! （见 `docs/development-workflow.md` Store Strategy）。
//!
//! 模块边界：handler 不直接写 SQL，所有操作走 repository trait。
//! 当前提供 SQLite 实现，后续可加 Postgres。

pub mod error;
pub mod repository;
pub mod sqlite;

pub use error::StoreError;
pub use repository::{
    AggregationDimension, AggregationFilter, AggregationRepository, OverallStats, ProxyKeyRecord,
    ProxyKeyRepository, RequestEventFilter, RequestEventRepository, Resource, ResourceRepository,
    SecurityEventFilter, SecurityEventRepository, UpstreamKeyRecord, UpstreamKeyRepository,
    UsageBucket, UsageBucketFilter, UsageBucketRepository, UsageSummary,
};
pub use sqlite::SqliteRequestEventRepository;

use sqlx::sqlite::SqlitePool;

/// 嵌入 migrations 目录的 SQL 文件（编译期读取，不需 DB 连接）。
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// 运行数据库迁移。
///
/// 使用编译期嵌入的 SQL 文件，不依赖 `DATABASE_URL`。
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), StoreError> {
    MIGRATOR.run(pool).await.map_err(StoreError::from)?;
    Ok(())
}

/// 创建 in-memory SQLite 连接池（主要用于测试）。
pub async fn in_memory_pool() -> Result<SqlitePool, StoreError> {
    SqlitePool::connect("sqlite::memory:")
        .await
        .map_err(StoreError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::redaction::redact_secret_key;
    use crate::observability::{
        RequestEvent, RequestStatus, SecurityEvent, SecurityEventKind, Severity,
    };
    use chrono::Utc;

    fn sample_event(
        request_id: &str,
        proxy_key_id: &str,
        resource_id: &str,
        status: RequestStatus,
    ) -> RequestEvent {
        RequestEvent {
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-07-03T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            request_id: request_id.to_string(),
            proxy_key_id: proxy_key_id.to_string(),
            resource_id: resource_id.to_string(),
            tool_name: "search__tavily__web_search".to_string(),
            upstream_key_ref: redact_secret_key("sk-1234567890abcdefwxyz"),
            status,
            latency_ms: 142,
            request_units: 1,
            retry_count: 0,
            rate_limited: false,
            queued_ms: 0,
        }
    }

    async fn setup_repo() -> SqliteRequestEventRepository {
        let pool = in_memory_pool().await.unwrap();
        run_migrations(&pool).await.unwrap();
        SqliteRequestEventRepository::new(pool)
    }

    #[tokio::test]
    async fn migration_creates_tables() {
        let pool = in_memory_pool().await.unwrap();
        run_migrations(&pool).await.unwrap();
        // 验证表存在：插入不应报错
        sqlx::query(
            "INSERT INTO resources (id, domain, provider, base_url) VALUES ('r1', 'search', 'tavily', 'https://api.tavily.com')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO proxy_keys (id, display_name) VALUES ('k1', 'dev-key')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO upstream_keys (id, resource_id, secret_ref) VALUES ('u1', 'r1', 'secret://tavily/default')",
        )
        .execute(&pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn insert_and_list_event_roundtrip() {
        let repo = setup_repo().await;
        let event = sample_event(
            "req_001",
            "agent-dev",
            "tavily-default",
            RequestStatus::Success,
        );

        repo.insert_event(&event).await.unwrap();

        let events = repo
            .list_events(&RequestEventFilter::default(), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].request_id, "req_001");
        assert_eq!(events[0].proxy_key_id, "agent-dev");
        assert_eq!(events[0].status, RequestStatus::Success);
        assert_eq!(events[0].upstream_key_ref, "key:1234…wxyz");
        assert_eq!(events[0].latency_ms, 142);
    }

    #[tokio::test]
    async fn list_events_filters_by_proxy_key() {
        let repo = setup_repo().await;

        let e1 = sample_event("req_001", "key-a", "res-1", RequestStatus::Success);
        let e2 = sample_event(
            "req_002",
            "key-b",
            "res-1",
            RequestStatus::UpstreamError(500),
        );

        repo.insert_event(&e1).await.unwrap();
        repo.insert_event(&e2).await.unwrap();

        let filter = RequestEventFilter {
            proxy_key_id: Some("key-a".to_string()),
            ..Default::default()
        };
        let events = repo.list_events(&filter, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].request_id, "req_001");
    }

    #[tokio::test]
    async fn list_events_filters_by_resource() {
        let repo = setup_repo().await;

        let e1 = sample_event("req_001", "key-a", "res-1", RequestStatus::Success);
        let e2 = sample_event("req_002", "key-a", "res-2", RequestStatus::Success);

        repo.insert_event(&e1).await.unwrap();
        repo.insert_event(&e2).await.unwrap();

        let filter = RequestEventFilter {
            resource_id: Some("res-2".to_string()),
            ..Default::default()
        };
        let events = repo.list_events(&filter, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].resource_id, "res-2");
    }

    #[tokio::test]
    async fn list_events_filters_by_time_range() {
        let repo = setup_repo().await;

        let mut e1 = sample_event("req_001", "key-a", "res-1", RequestStatus::Success);
        e1.timestamp = chrono::DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut e2 = sample_event("req_002", "key-a", "res-1", RequestStatus::Success);
        e2.timestamp = chrono::DateTime::parse_from_rfc3339("2026-07-03T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        repo.insert_event(&e1).await.unwrap();
        repo.insert_event(&e2).await.unwrap();

        let filter = RequestEventFilter {
            from: Some(
                chrono::DateTime::parse_from_rfc3339("2026-07-02T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            to: Some(
                chrono::DateTime::parse_from_rfc3339("2026-07-04T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            ..Default::default()
        };
        let events = repo.list_events(&filter, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].request_id, "req_002");
    }

    #[tokio::test]
    async fn list_events_respects_limit() {
        let repo = setup_repo().await;

        for i in 0..5 {
            let event = sample_event(
                &format!("req_{i:03}"),
                "key-a",
                "res-1",
                RequestStatus::Success,
            );
            repo.insert_event(&event).await.unwrap();
        }

        let events = repo
            .list_events(&RequestEventFilter::default(), 3)
            .await
            .unwrap();
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn insert_preserves_all_status_variants() {
        let repo = setup_repo().await;

        let variants = vec![
            ("req_s", RequestStatus::Success),
            ("req_e", RequestStatus::UpstreamError(503)),
            ("req_t", RequestStatus::Timeout),
            ("req_c", RequestStatus::ConnectionFailed),
            ("req_l", RequestStatus::Limited),
            ("req_0", RequestStatus::UpstreamError(0)),
        ];

        for (id, status) in &variants {
            let event = sample_event(id, "key-a", "res-1", status.clone());
            repo.insert_event(&event).await.unwrap();
        }

        let events = repo
            .list_events(&RequestEventFilter::default(), 100)
            .await
            .unwrap();
        assert_eq!(events.len(), 6);

        // 验证每个变体都能正确往返
        for (id, expected_status) in &variants {
            let found = events.iter().find(|e| &e.request_id == id).unwrap();
            assert_eq!(&found.status, expected_status, "status mismatch for {id}");
        }
    }

    #[tokio::test]
    async fn insert_preserves_rate_limited_and_queued() {
        let repo = setup_repo().await;

        let mut event = sample_event("req_001", "key-a", "res-1", RequestStatus::Limited);
        event.rate_limited = true;
        event.queued_ms = 500;
        event.retry_count = 2;
        event.request_units = 10;

        repo.insert_event(&event).await.unwrap();

        let events = repo
            .list_events(&RequestEventFilter::default(), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].rate_limited);
        assert_eq!(events[0].queued_ms, 500);
        assert_eq!(events[0].retry_count, 2);
        assert_eq!(events[0].request_units, 10);
    }

    #[tokio::test]
    async fn empty_filter_returns_all_ordered_by_timestamp_desc() {
        let repo = setup_repo().await;

        let mut e1 = sample_event("req_001", "key-a", "res-1", RequestStatus::Success);
        e1.timestamp = chrono::DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut e2 = sample_event("req_002", "key-a", "res-1", RequestStatus::Success);
        e2.timestamp = chrono::DateTime::parse_from_rfc3339("2026-07-03T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        repo.insert_event(&e1).await.unwrap();
        repo.insert_event(&e2).await.unwrap();

        let events = repo
            .list_events(&RequestEventFilter::default(), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        // DESC：e2（7/3）在前，e1（7/1）在后
        assert_eq!(events[0].request_id, "req_002");
        assert_eq!(events[1].request_id, "req_001");
    }

    // ── SecurityEventRepository 测试 ──

    fn sample_security_event(resource_id: &str, kind: SecurityEventKind) -> SecurityEvent {
        SecurityEvent {
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-07-04T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            resource_id: resource_id.to_string(),
            tool_name: Some("search__tavily__web_search".to_string()),
            kind,
            severity: Severity::Warn,
            details: serde_json::json!({"tool_name": "search__tavily__web_search"}),
        }
    }

    #[tokio::test]
    async fn security_event_insert_and_list_roundtrip() {
        let repo = setup_repo().await;
        let event =
            sample_security_event("tavily-default", SecurityEventKind::IntegrityToolChanged);

        repo.insert_security_event(&event).await.unwrap();

        let events = repo
            .list_security_events(&SecurityEventFilter::default(), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].resource_id, "tavily-default");
        assert_eq!(events[0].kind, SecurityEventKind::IntegrityToolChanged);
        assert_eq!(events[0].severity, Severity::Warn);
        assert_eq!(
            events[0].tool_name,
            Some("search__tavily__web_search".to_string())
        );
        assert_eq!(events[0].details["tool_name"], "search__tavily__web_search");
    }

    #[tokio::test]
    async fn security_event_filters_by_resource() {
        let repo = setup_repo().await;

        let e1 = sample_security_event("res-1", SecurityEventKind::IntegrityToolAdded);
        let e2 = sample_security_event("res-2", SecurityEventKind::IntegrityToolRemoved);

        repo.insert_security_event(&e1).await.unwrap();
        repo.insert_security_event(&e2).await.unwrap();

        let filter = SecurityEventFilter {
            resource_id: Some("res-2".to_string()),
            ..Default::default()
        };
        let events = repo.list_security_events(&filter, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].resource_id, "res-2");
        assert_eq!(events[0].kind, SecurityEventKind::IntegrityToolRemoved);
    }

    #[tokio::test]
    async fn security_event_filters_by_kind() {
        let repo = setup_repo().await;

        let e1 = sample_security_event("res-1", SecurityEventKind::IntegrityToolChanged);
        let e2 = sample_security_event("res-1", SecurityEventKind::ContentDefenseFlag);

        repo.insert_security_event(&e1).await.unwrap();
        repo.insert_security_event(&e2).await.unwrap();

        let filter = SecurityEventFilter {
            kind: Some(SecurityEventKind::ContentDefenseFlag),
            ..Default::default()
        };
        let events = repo.list_security_events(&filter, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, SecurityEventKind::ContentDefenseFlag);
    }

    #[tokio::test]
    async fn security_event_preserves_null_tool_name() {
        let repo = setup_repo().await;
        let mut event = sample_security_event("res-1", SecurityEventKind::IntegrityToolAdded);
        event.tool_name = None;

        repo.insert_security_event(&event).await.unwrap();

        let events = repo
            .list_security_events(&SecurityEventFilter::default(), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, None);
    }

    // ── ResourceRepository 测试 ──

    use crate::store::repository::{
        ProxyKeyRecord, ProxyKeyRepository, Resource, ResourceRepository, UpstreamKeyRecord,
        UpstreamKeyRepository, UsageBucket, UsageBucketFilter, UsageBucketRepository,
    };

    fn sample_resource(id: &str) -> Resource {
        Resource {
            id: id.to_string(),
            domain: "search".to_string(),
            provider: "tavily".to_string(),
            base_url: "https://api.tavily.com".to_string(),
            description: Some("Tavily search".to_string()),
            config_json: "{}".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn resource_insert_get_roundtrip() {
        let repo = setup_repo().await;
        repo.insert_resource(&sample_resource("r1")).await.unwrap();

        let r = repo.get_resource("r1").await.unwrap();
        assert_eq!(r.id, "r1");
        assert_eq!(r.domain, "search");
        assert_eq!(r.provider, "tavily");
        assert_eq!(r.description, Some("Tavily search".to_string()));
        assert!(!r.created_at.is_empty());
    }

    #[tokio::test]
    async fn resource_list_and_delete() {
        let repo = setup_repo().await;
        repo.insert_resource(&sample_resource("r1")).await.unwrap();
        repo.insert_resource(&sample_resource("r2")).await.unwrap();

        let all = repo.list_resources().await.unwrap();
        assert_eq!(all.len(), 2);

        assert!(repo.delete_resource("r1").await.unwrap());
        assert!(!repo.delete_resource("r1").await.unwrap()); // already gone

        let remaining = repo.list_resources().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "r2");
    }

    #[tokio::test]
    async fn resource_update() {
        let repo = setup_repo().await;
        repo.insert_resource(&sample_resource("r1")).await.unwrap();

        let mut updated = sample_resource("r1");
        updated.domain = "ai".to_string();
        updated.base_url = "https://api.new.com".to_string();
        assert!(repo.update_resource(&updated).await.unwrap());

        let r = repo.get_resource("r1").await.unwrap();
        assert_eq!(r.domain, "ai");
        assert_eq!(r.base_url, "https://api.new.com");

        assert!(
            !repo
                .update_resource(&sample_resource("nope"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn resource_get_not_found() {
        let repo = setup_repo().await;
        let err = repo.get_resource("nope").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound(_)));
    }

    // ── ProxyKeyRepository 测试 ──

    fn sample_proxy_key(id: &str) -> ProxyKeyRecord {
        ProxyKeyRecord {
            id: id.to_string(),
            display_name: format!("key-{id}"),
            default_tool_page_size: 50,
            scope_json: r#"{"resources":["*"]}"#.to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn proxy_key_insert_get_roundtrip() {
        let repo = setup_repo().await;
        repo.insert_proxy_key(&sample_proxy_key("pk1"))
            .await
            .unwrap();

        let k = repo.get_proxy_key("pk1").await.unwrap();
        assert_eq!(k.id, "pk1");
        assert_eq!(k.display_name, "key-pk1");
        assert_eq!(k.default_tool_page_size, 50);
    }

    #[tokio::test]
    async fn proxy_key_list_and_delete() {
        let repo = setup_repo().await;
        repo.insert_proxy_key(&sample_proxy_key("pk1"))
            .await
            .unwrap();
        repo.insert_proxy_key(&sample_proxy_key("pk2"))
            .await
            .unwrap();

        assert_eq!(repo.list_proxy_keys().await.unwrap().len(), 2);
        assert!(repo.delete_proxy_key("pk1").await.unwrap());
        assert_eq!(repo.list_proxy_keys().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn proxy_key_update() {
        let repo = setup_repo().await;
        repo.insert_proxy_key(&sample_proxy_key("pk1"))
            .await
            .unwrap();

        let mut updated = sample_proxy_key("pk1");
        updated.display_name = "renamed".to_string();
        updated.default_tool_page_size = 100;
        assert!(repo.update_proxy_key(&updated).await.unwrap());

        let k = repo.get_proxy_key("pk1").await.unwrap();
        assert_eq!(k.display_name, "renamed");
        assert_eq!(k.default_tool_page_size, 100);

        assert!(
            !repo
                .update_proxy_key(&sample_proxy_key("nope"))
                .await
                .unwrap()
        );
    }

    // ── UpstreamKeyRepository 测试 ──

    fn sample_upstream_key(id: &str, resource_id: &str) -> UpstreamKeyRecord {
        UpstreamKeyRecord {
            id: id.to_string(),
            resource_id: resource_id.to_string(),
            secret_ref: format!("secret://tavily/{id}"),
            weight: 1,
            health_state: "healthy".to_string(),
            cooldown_until: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn upstream_key_insert_get_roundtrip() {
        let repo = setup_repo().await;
        // FK：先插入 resource
        repo.insert_resource(&sample_resource("r1")).await.unwrap();
        repo.insert_upstream_key(&sample_upstream_key("u1", "r1"))
            .await
            .unwrap();

        let k = repo.get_upstream_key("u1").await.unwrap();
        assert_eq!(k.id, "u1");
        assert_eq!(k.resource_id, "r1");
        assert_eq!(k.health_state, "healthy");
        assert_eq!(k.cooldown_until, None);
    }

    #[tokio::test]
    async fn upstream_key_list_for_resource() {
        let repo = setup_repo().await;
        repo.insert_resource(&sample_resource("r1")).await.unwrap();
        repo.insert_resource(&sample_resource("r2")).await.unwrap();
        repo.insert_upstream_key(&sample_upstream_key("u1", "r1"))
            .await
            .unwrap();
        repo.insert_upstream_key(&sample_upstream_key("u2", "r1"))
            .await
            .unwrap();
        repo.insert_upstream_key(&sample_upstream_key("u3", "r2"))
            .await
            .unwrap();

        let keys = repo.list_upstream_keys_for_resource("r1").await.unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().all(|k| k.resource_id == "r1"));
    }

    #[tokio::test]
    async fn upstream_key_update_health() {
        let repo = setup_repo().await;
        repo.insert_resource(&sample_resource("r1")).await.unwrap();
        repo.insert_upstream_key(&sample_upstream_key("u1", "r1"))
            .await
            .unwrap();

        let updated = repo
            .update_upstream_key_health("u1", "degraded", Some("2026-07-05T00:00:00Z"))
            .await
            .unwrap();
        assert!(updated);

        let k = repo.get_upstream_key("u1").await.unwrap();
        assert_eq!(k.health_state, "degraded");
        assert_eq!(k.cooldown_until, Some("2026-07-05T00:00:00Z".to_string()));

        // 不存在的 key 返回 false
        assert!(
            !repo
                .update_upstream_key_health("nope", "healthy", None)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn upstream_key_delete() {
        let repo = setup_repo().await;
        repo.insert_resource(&sample_resource("r1")).await.unwrap();
        repo.insert_upstream_key(&sample_upstream_key("u1", "r1"))
            .await
            .unwrap();

        assert!(repo.delete_upstream_key("u1").await.unwrap());
        assert!(!repo.delete_upstream_key("u1").await.unwrap());
    }

    // ── UsageBucketRepository 测试 ──

    fn sample_bucket() -> UsageBucket {
        UsageBucket {
            bucket_start: "2026-07-03T12:00:00Z".to_string(),
            granularity: "hour".to_string(),
            proxy_key_id: "pk1".to_string(),
            resource_id: "r1".to_string(),
            tool_name: "web_search".to_string(),
            upstream_key_ref: "key:1234…wxyz".to_string(),
            status: "success".to_string(),
            request_count: 5,
            total_units: 10,
            error_count: 0,
            rate_limit_hits: 0,
            total_latency_ms: 500,
            total_queued_ms: 0,
        }
    }

    #[tokio::test]
    async fn usage_bucket_upsert_insert_then_increment() {
        let repo = setup_repo().await;
        let bucket = sample_bucket();

        // 首次插入
        repo.upsert_bucket(&bucket).await.unwrap();
        let results = repo
            .query_buckets(&UsageBucketFilter::default(), 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].request_count, 5);
        assert_eq!(results[0].total_units, 10);

        // 二次 upsert 累加
        repo.upsert_bucket(&bucket).await.unwrap();
        let results = repo
            .query_buckets(&UsageBucketFilter::default(), 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].request_count, 10);
        assert_eq!(results[0].total_units, 20);
        assert_eq!(results[0].total_latency_ms, 1000);
    }

    #[tokio::test]
    async fn usage_bucket_query_filters() {
        let repo = setup_repo().await;
        repo.upsert_bucket(&sample_bucket()).await.unwrap();

        let mut b2 = sample_bucket();
        b2.resource_id = "r2".to_string();
        repo.upsert_bucket(&b2).await.unwrap();

        let filter = UsageBucketFilter {
            resource_id: Some("r1".to_string()),
            ..Default::default()
        };
        let results = repo.query_buckets(&filter, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].resource_id, "r1");
    }

    #[tokio::test]
    async fn usage_bucket_query_time_range() {
        let repo = setup_repo().await;
        repo.upsert_bucket(&sample_bucket()).await.unwrap();

        let mut b2 = sample_bucket();
        b2.bucket_start = "2026-07-04T12:00:00Z".to_string();
        repo.upsert_bucket(&b2).await.unwrap();

        let filter = UsageBucketFilter {
            from: Some(
                chrono::DateTime::parse_from_rfc3339("2026-07-04T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            ..Default::default()
        };
        let results = repo.query_buckets(&filter, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].bucket_start, "2026-07-04T12:00:00Z");
    }

    #[tokio::test]
    async fn series_by_bucket_sums_across_dimensions_ascending() {
        let repo = setup_repo().await;
        // 同一小时两个维度行（不同 tool）+ 另一小时一行 + 一行其他粒度
        repo.upsert_bucket(&sample_bucket()).await.unwrap();
        let mut b2 = sample_bucket();
        b2.tool_name = "news_search".to_string();
        b2.error_count = 2;
        repo.upsert_bucket(&b2).await.unwrap();
        let mut b3 = sample_bucket();
        b3.bucket_start = "2026-07-03T13:00:00Z".to_string();
        repo.upsert_bucket(&b3).await.unwrap();
        let mut b4 = sample_bucket();
        b4.granularity = "day".to_string();
        repo.upsert_bucket(&b4).await.unwrap();

        let rows = repo
            .series_by_bucket("hour", &AggregationFilter::default(), 100)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "day 粒度行不掺入");
        assert_eq!(rows[0].dimension_value, "2026-07-03T12:00:00Z");
        assert_eq!(rows[0].request_count, 10);
        assert_eq!(rows[0].error_count, 2);
        assert!((rows[0].avg_latency_ms - 100.0).abs() < f64::EPSILON); // 1000ms/10req
        assert_eq!(rows[1].dimension_value, "2026-07-03T13:00:00Z");
        assert_eq!(rows[1].request_count, 5);
    }

    // ── AggregationRepository 测试 ──

    use crate::store::repository::{
        AggregationDimension, AggregationFilter, AggregationRepository,
    };

    fn sample_event_with_tool(
        request_id: &str,
        proxy_key_id: &str,
        resource_id: &str,
        tool_name: &str,
        status: RequestStatus,
    ) -> RequestEvent {
        RequestEvent {
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-07-03T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            request_id: request_id.to_string(),
            proxy_key_id: proxy_key_id.to_string(),
            resource_id: resource_id.to_string(),
            tool_name: tool_name.to_string(),
            upstream_key_ref: "key:test".to_string(),
            status,
            latency_ms: 100,
            request_units: 1,
            retry_count: 0,
            rate_limited: false,
            queued_ms: 0,
        }
    }

    #[tokio::test]
    async fn summarize_by_resource() {
        let repo = setup_repo().await;
        repo.insert_event(&sample_event_with_tool(
            "r1",
            "k1",
            "res-a",
            "search__tavily__web",
            RequestStatus::Success,
        ))
        .await
        .unwrap();
        repo.insert_event(&sample_event_with_tool(
            "r2",
            "k1",
            "res-a",
            "search__tavily__web",
            RequestStatus::Success,
        ))
        .await
        .unwrap();
        repo.insert_event(&sample_event_with_tool(
            "r3",
            "k1",
            "res-b",
            "search__exa__web",
            RequestStatus::UpstreamError(500),
        ))
        .await
        .unwrap();

        let results = repo
            .summarize_by(
                AggregationDimension::Resource,
                &AggregationFilter::default(),
                10,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        let a = results
            .iter()
            .find(|r| r.dimension_value == "res-a")
            .unwrap();
        assert_eq!(a.request_count, 2);
        assert_eq!(a.error_count, 0);
        let b = results
            .iter()
            .find(|r| r.dimension_value == "res-b")
            .unwrap();
        assert_eq!(b.request_count, 1);
        assert_eq!(b.error_count, 1);
    }

    #[tokio::test]
    async fn summarize_by_domain() {
        let repo = setup_repo().await;
        repo.insert_event(&sample_event_with_tool(
            "r1",
            "k1",
            "res",
            "search__tavily__web",
            RequestStatus::Success,
        ))
        .await
        .unwrap();
        repo.insert_event(&sample_event_with_tool(
            "r2",
            "k1",
            "res",
            "code__github__repo",
            RequestStatus::Success,
        ))
        .await
        .unwrap();

        let results = repo
            .summarize_by(
                AggregationDimension::Domain,
                &AggregationFilter::default(),
                10,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.dimension_value == "search"));
        assert!(results.iter().any(|r| r.dimension_value == "code"));
    }

    #[tokio::test]
    async fn overall_stats_mixed_events() {
        let repo = setup_repo().await;
        repo.insert_event(&sample_event_with_tool(
            "r1",
            "k1",
            "res-a",
            "tool1",
            RequestStatus::Success,
        ))
        .await
        .unwrap();
        repo.insert_event(&sample_event_with_tool(
            "r2",
            "k2",
            "res-b",
            "tool2",
            RequestStatus::UpstreamError(503),
        ))
        .await
        .unwrap();

        let stats = repo
            .overall_stats(&AggregationFilter::default())
            .await
            .unwrap();
        assert_eq!(stats.total_requests, 2);
        assert_eq!(stats.total_errors, 1);
        assert_eq!(stats.unique_tools, 2);
        assert_eq!(stats.unique_proxy_keys, 2);
        assert_eq!(stats.unique_resources, 2);
        assert!(stats.avg_latency_ms > 0.0);
    }
}
