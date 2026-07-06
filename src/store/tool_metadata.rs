//! 工具介绍 override repository（见 docs/mcp-governance-and-key-limits.md 第 5 节）。
//!
//! 管理员编写的工具介绍，覆盖上游 description；对外可见描述 = override ?? 上游原始。
//! integrity baseline 继续使用上游原始 description（override 不参与 fingerprint）。
//! trait + SQLite 实现独立成文件，模式照抄 `tool_defaults.rs`。

use crate::store::error::StoreError;
use crate::store::sqlite::SqliteRequestEventRepository;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

/// 工具介绍 override 记录（DB 行映射）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolMetadataEntry {
    /// 工具 wire name（主键）。
    pub tool_name: String,
    /// 管理员编写的介绍（覆盖上游 description）。
    pub description: String,
    /// 最后写入的 admin key id。
    pub updated_by: Option<String>,
    /// 最后更新时间（写路径由 DB 生成）。
    pub updated_at: String,
}

/// 工具介绍 override repository trait（get/set/delete/list）。
pub trait ToolMetadataRepository: Send + Sync {
    /// 按 wire name 获取介绍 override；不存在返回 `None`。
    fn get_tool_metadata(
        &self,
        tool_name: &str,
    ) -> impl std::future::Future<Output = Result<Option<ToolMetadataEntry>, StoreError>> + Send;

    /// upsert 介绍 override；`updated_at` 由 DB 生成。
    fn set_tool_metadata(
        &self,
        tool_name: &str,
        description: &str,
        updated_by: Option<&str>,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 删除介绍 override，返回是否有行被删除。
    fn delete_tool_metadata(
        &self,
        tool_name: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    /// 列出全部介绍 override（按 tool_name 升序）。
    fn list_tool_metadata(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<ToolMetadataEntry>, StoreError>> + Send;
}

impl ToolMetadataRepository for () {
    async fn get_tool_metadata(
        &self,
        _tool_name: &str,
    ) -> Result<Option<ToolMetadataEntry>, StoreError> {
        Ok(None)
    }
    async fn set_tool_metadata(
        &self,
        _tool_name: &str,
        _description: &str,
        _updated_by: Option<&str>,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    async fn delete_tool_metadata(&self, _tool_name: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn list_tool_metadata(&self) -> Result<Vec<ToolMetadataEntry>, StoreError> {
        Ok(Vec::new())
    }
}

fn row_to_entry(row: SqliteRow) -> Result<ToolMetadataEntry, StoreError> {
    Ok(ToolMetadataEntry {
        tool_name: row.try_get("tool_name").map_err(StoreError::from)?,
        description: row.try_get("description").map_err(StoreError::from)?,
        updated_by: row.try_get("updated_by").map_err(StoreError::from)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::from)?,
    })
}

impl ToolMetadataRepository for SqliteRequestEventRepository {
    async fn get_tool_metadata(
        &self,
        tool_name: &str,
    ) -> Result<Option<ToolMetadataEntry>, StoreError> {
        sqlx::query(
            r#"
            SELECT tool_name, description, updated_by, updated_at
            FROM tool_metadata WHERE tool_name = ?
            "#,
        )
        .bind(tool_name)
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?
        .map(row_to_entry)
        .transpose()
    }

    async fn set_tool_metadata(
        &self,
        tool_name: &str,
        description: &str,
        updated_by: Option<&str>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO tool_metadata (tool_name, description, updated_by, updated_at)
            VALUES (?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT (tool_name) DO UPDATE SET
                description = excluded.description,
                updated_by  = excluded.updated_by,
                updated_at  = excluded.updated_at
            "#,
        )
        .bind(tool_name)
        .bind(description)
        .bind(updated_by)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn delete_tool_metadata(&self, tool_name: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM tool_metadata WHERE tool_name = ?")
            .bind(tool_name)
            .execute(self.pool())
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_tool_metadata(&self) -> Result<Vec<ToolMetadataEntry>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT tool_name, description, updated_by, updated_at
            FROM tool_metadata ORDER BY tool_name
            "#,
        )
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.into_iter().map(row_to_entry).collect()
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
    async fn set_get_roundtrip_and_upsert_overwrites() {
        let repo = repo().await;
        repo.set_tool_metadata("search__mock__search", "初版介绍", Some("ops"))
            .await
            .unwrap();

        let got = repo
            .get_tool_metadata("search__mock__search")
            .await
            .unwrap()
            .expect("present");
        assert_eq!(got.tool_name, "search__mock__search");
        assert_eq!(got.description, "初版介绍");
        assert_eq!(got.updated_by.as_deref(), Some("ops"));
        assert!(!got.updated_at.is_empty());

        // 同名 upsert 覆盖 description/updated_by
        repo.set_tool_metadata("search__mock__search", "修订后介绍", None)
            .await
            .unwrap();
        let got = repo
            .get_tool_metadata("search__mock__search")
            .await
            .unwrap()
            .expect("present");
        assert_eq!(got.description, "修订后介绍");
        assert_eq!(got.updated_by, None);
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let repo = repo().await;
        assert!(
            repo.get_tool_metadata("no__such__tool")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_reports_whether_row_existed() {
        let repo = repo().await;
        repo.set_tool_metadata("a__b__c", "desc", Some("ops"))
            .await
            .unwrap();
        assert!(repo.delete_tool_metadata("a__b__c").await.unwrap());
        assert!(!repo.delete_tool_metadata("a__b__c").await.unwrap());
        assert!(repo.get_tool_metadata("a__b__c").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_returns_all_sorted_by_tool_name() {
        let repo = repo().await;
        repo.set_tool_metadata("b__b__b", "d2", None).await.unwrap();
        repo.set_tool_metadata("a__a__a", "d1", None).await.unwrap();
        let all = repo.list_tool_metadata().await.unwrap();
        assert_eq!(
            all.iter().map(|r| r.tool_name.as_str()).collect::<Vec<_>>(),
            vec!["a__a__a", "b__b__b"]
        );
    }

    #[tokio::test]
    async fn noop_impl_returns_empty() {
        assert!(().get_tool_metadata("x").await.unwrap().is_none());
        assert!(().list_tool_metadata().await.unwrap().is_empty());
        assert!(!().delete_tool_metadata("x").await.unwrap());
        ().set_tool_metadata("x", "d", None).await.unwrap();
    }
}
