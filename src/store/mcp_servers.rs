//! MCP server 配置 repository（见 docs/mcp-governance-and-key-limits.md §6）。
//!
//! admin CRUD 的持久化路径，模式照抄 `resources` 表：`config_json` 存
//! auth（仅 secret ref）/ security / limits / health_check；与 resources
//! 一致，启动时不从 DB 加载（配置以 YAML 为准）。trait + `()` no-op +
//! SQLite 实现独立成文件，照抄 `tool_metadata.rs`。

use crate::store::error::StoreError;
use crate::store::sqlite::SqliteRequestEventRepository;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

/// MCP server 配置记录（DB 行映射）。
#[derive(Debug, Clone)]
pub struct McpServerRecord {
    pub id: String,
    pub domain: String,
    pub provider: String,
    pub url: String,
    pub description: Option<String>,
    /// auth / security / limits / health_check 的 JSON（auth 只含 secret ref）。
    pub config_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// MCP server 配置 repository trait（insert/update/delete/list）。
pub trait McpServerRepository: Send + Sync {
    /// 插入一条记录（created_at/updated_at 由 DB 默认值填充）。
    fn insert_mcp_server(
        &self,
        server: &McpServerRecord,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 更新记录，返回是否有行被更新。
    fn update_mcp_server(
        &self,
        server: &McpServerRecord,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    /// 删除记录，返回是否有行被删除。
    fn delete_mcp_server(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    /// 列出全部记录（按 id 升序）。
    fn list_mcp_servers(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<McpServerRecord>, StoreError>> + Send;
}

impl McpServerRepository for () {
    async fn insert_mcp_server(&self, _server: &McpServerRecord) -> Result<(), StoreError> {
        Ok(())
    }
    async fn update_mcp_server(&self, _server: &McpServerRecord) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn delete_mcp_server(&self, _id: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn list_mcp_servers(&self) -> Result<Vec<McpServerRecord>, StoreError> {
        Ok(Vec::new())
    }
}

fn row_to_record(row: SqliteRow) -> Result<McpServerRecord, StoreError> {
    Ok(McpServerRecord {
        id: row.try_get("id").map_err(StoreError::from)?,
        domain: row.try_get("domain").map_err(StoreError::from)?,
        provider: row.try_get("provider").map_err(StoreError::from)?,
        url: row.try_get("url").map_err(StoreError::from)?,
        description: row.try_get("description").map_err(StoreError::from)?,
        config_json: row.try_get("config_json").map_err(StoreError::from)?,
        created_at: row.try_get("created_at").map_err(StoreError::from)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::from)?,
    })
}

impl McpServerRepository for SqliteRequestEventRepository {
    async fn insert_mcp_server(&self, server: &McpServerRecord) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO mcp_servers (id, domain, provider, url, description, config_json)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&server.id)
        .bind(&server.domain)
        .bind(&server.provider)
        .bind(&server.url)
        .bind(&server.description)
        .bind(&server.config_json)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn update_mcp_server(&self, server: &McpServerRecord) -> Result<bool, StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE mcp_servers
            SET domain = ?, provider = ?, url = ?, description = ?, config_json = ?,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ?
            "#,
        )
        .bind(&server.domain)
        .bind(&server.provider)
        .bind(&server.url)
        .bind(&server.description)
        .bind(&server.config_json)
        .bind(&server.id)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_mcp_server(&self, id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM mcp_servers WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_mcp_servers(&self) -> Result<Vec<McpServerRecord>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, domain, provider, url, description, config_json, created_at, updated_at
            FROM mcp_servers ORDER BY id
            "#,
        )
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.into_iter().map(row_to_record).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{in_memory_pool, run_migrations};

    async fn repo() -> SqliteRequestEventRepository {
        let pool = in_memory_pool().await.unwrap();
        run_migrations(&pool).await.unwrap();
        SqliteRequestEventRepository::new(pool)
    }

    fn record(id: &str) -> McpServerRecord {
        McpServerRecord {
            id: id.to_string(),
            domain: "search".to_string(),
            provider: id.to_string(),
            url: "https://mcp.example.com/mcp".to_string(),
            description: Some("test server".to_string()),
            config_json: r#"{"auth":{"type":"none"}}"#.to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn insert_list_update_delete_roundtrip() {
        let repo = repo().await;
        repo.insert_mcp_server(&record("exa")).await.unwrap();

        let all = repo.list_mcp_servers().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "exa");
        assert_eq!(all[0].url, "https://mcp.example.com/mcp");
        assert!(!all[0].created_at.is_empty());

        let mut updated = record("exa");
        updated.url = "https://changed.example.com/mcp".to_string();
        assert!(repo.update_mcp_server(&updated).await.unwrap());
        let all = repo.list_mcp_servers().await.unwrap();
        assert_eq!(all[0].url, "https://changed.example.com/mcp");

        assert!(repo.delete_mcp_server("exa").await.unwrap());
        assert!(!repo.delete_mcp_server("exa").await.unwrap());
        assert!(repo.list_mcp_servers().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn update_missing_returns_false_and_noop_impl_is_empty() {
        let repo = repo().await;
        assert!(!repo.update_mcp_server(&record("nope")).await.unwrap());
        assert!(().list_mcp_servers().await.unwrap().is_empty());
        assert!(!().delete_mcp_server("x").await.unwrap());
    }
}
