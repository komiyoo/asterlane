//! 上游 MCP OAuth 凭据 repository（见 docs/runtime/key-credentials-and-persistence.md）。
//!
//! 只存已加密的密文（`secrets::TokenEncryptionKey` 的输出），加解密与 rmcp 类型
//! 都不在 store 内：repository 对内容一无所知，按 server id 读写一个字符串。
//! trait + `()` no-op + SQLite 实现独立成文件，照抄 `mcp_servers.rs`。

use crate::store::error::StoreError;
use crate::store::sqlite::SqliteRequestEventRepository;
use sqlx::Row;

/// 上游 OAuth 凭据 repository trait（get/put/delete）。
pub trait UpstreamOAuthCredentialRepository: Send + Sync {
    /// 读取某个 server 的密文；不存在返回 `None`。
    fn get_sealed_credentials(
        &self,
        server_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<String>, StoreError>> + Send;

    /// upsert 某个 server 的密文；`updated_at` 由 DB 生成。
    fn put_sealed_credentials(
        &self,
        server_id: &str,
        sealed: &str,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 删除某个 server 的密文，返回是否有行被删除。
    fn delete_sealed_credentials(
        &self,
        server_id: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;
}

impl UpstreamOAuthCredentialRepository for () {
    async fn get_sealed_credentials(&self, _server_id: &str) -> Result<Option<String>, StoreError> {
        Ok(None)
    }
    async fn put_sealed_credentials(
        &self,
        _server_id: &str,
        _sealed: &str,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    async fn delete_sealed_credentials(&self, _server_id: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
}

impl UpstreamOAuthCredentialRepository for SqliteRequestEventRepository {
    async fn get_sealed_credentials(&self, server_id: &str) -> Result<Option<String>, StoreError> {
        sqlx::query("SELECT sealed_credentials FROM upstream_oauth_credentials WHERE server_id = ?")
            .bind(server_id)
            .fetch_optional(self.pool())
            .await
            .map_err(StoreError::from)?
            .map(|row| row.try_get("sealed_credentials").map_err(StoreError::from))
            .transpose()
    }

    async fn put_sealed_credentials(
        &self,
        server_id: &str,
        sealed: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO upstream_oauth_credentials (server_id, sealed_credentials, updated_at)
            VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT (server_id) DO UPDATE SET
                sealed_credentials = excluded.sealed_credentials,
                updated_at         = excluded.updated_at
            "#,
        )
        .bind(server_id)
        .bind(sealed)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn delete_sealed_credentials(&self, server_id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM upstream_oauth_credentials WHERE server_id = ?")
            .bind(server_id)
            .execute(self.pool())
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
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

    #[tokio::test]
    async fn put_get_overwrite_delete_roundtrip() {
        let repo = repo().await;
        assert_eq!(repo.get_sealed_credentials("linear").await.unwrap(), None);

        repo.put_sealed_credentials("linear", "sealed-1")
            .await
            .unwrap();
        assert_eq!(
            repo.get_sealed_credentials("linear")
                .await
                .unwrap()
                .as_deref(),
            Some("sealed-1")
        );

        // upsert：同一 server 只保留最新密文
        repo.put_sealed_credentials("linear", "sealed-2")
            .await
            .unwrap();
        assert_eq!(
            repo.get_sealed_credentials("linear")
                .await
                .unwrap()
                .as_deref(),
            Some("sealed-2")
        );

        // 不同 server 互不影响
        repo.put_sealed_credentials("notion", "sealed-n")
            .await
            .unwrap();
        assert!(repo.delete_sealed_credentials("linear").await.unwrap());
        assert!(!repo.delete_sealed_credentials("linear").await.unwrap());
        assert_eq!(repo.get_sealed_credentials("linear").await.unwrap(), None);
        assert_eq!(
            repo.get_sealed_credentials("notion")
                .await
                .unwrap()
                .as_deref(),
            Some("sealed-n")
        );
    }

    #[tokio::test]
    async fn updated_at_is_set_by_the_database() {
        let repo = repo().await;
        repo.put_sealed_credentials("linear", "sealed")
            .await
            .unwrap();
        let row = sqlx::query("SELECT updated_at FROM upstream_oauth_credentials")
            .fetch_one(repo.pool())
            .await
            .unwrap();
        let updated_at: String = row.try_get("updated_at").unwrap();
        assert!(updated_at.ends_with('Z'), "{updated_at}");
    }

    #[tokio::test]
    async fn noop_repository_stores_nothing() {
        assert_eq!(().get_sealed_credentials("x").await.unwrap(), None);
        ().put_sealed_credentials("x", "s").await.unwrap();
        assert!(!().delete_sealed_credentials("x").await.unwrap());
    }
}
