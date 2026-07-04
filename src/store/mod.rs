//! Store 模块：数据库抽象、迁移与 repository 实现
//! （见 `docs/development-workflow.md` Store Strategy）。
//!
//! 模块边界：handler 不直接写 SQL，所有操作走 repository trait。
//! 当前提供 SQLite 实现，后续可加 Postgres。

pub mod error;
pub mod repository;
pub mod sqlite;

pub use error::StoreError;
pub use repository::{RequestEventFilter, RequestEventRepository};
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
    use crate::observability::{RequestEvent, RequestStatus};
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
            tool_name: "search__tavily__web_search__post".to_string(),
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
}
