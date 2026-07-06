//! 工具默认调用参数 repository（见 docs/tool-debugging-and-cli.md 第 3 节）。
//!
//! 平台级、按工具维度的调试辅助：只在控制台/CLI 调试调用显式选择时合并，
//! 不参与 agent 正常调用路径。trait + SQLite 实现独立成文件，
//! 避免继续膨胀 `repository.rs` / `sqlite.rs`（两者已近单文件预算）。

use crate::store::error::StoreError;
use crate::store::sqlite::SqliteRequestEventRepository;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

/// 工具默认参数记录（DB 行映射）。
#[derive(Debug, Clone)]
pub struct ToolDefaultRecord {
    /// 工具 wire name（主键）。
    pub tool_name: String,
    /// 默认参数 JSON object 文本。
    pub args_json: String,
    /// 来源：`manual`（手工/CLI 写入）| `captured`（从实际调用保存）。
    pub source: String,
    /// 最后写入的 admin key id。
    pub updated_by: Option<String>,
    /// 最后更新时间（写路径由 DB 生成，构造时可留空）。
    pub updated_at: String,
}

/// 工具默认参数 repository trait（get/set/delete/list）。
pub trait ToolDefaultsRepository: Send + Sync {
    /// 按 wire name 获取默认参数；不存在返回 `None`。
    fn get_tool_default(
        &self,
        tool_name: &str,
    ) -> impl std::future::Future<Output = Result<Option<ToolDefaultRecord>, StoreError>> + Send;

    /// upsert 默认参数；`updated_at` 由 DB 生成，记录中的值被忽略。
    fn set_tool_default(
        &self,
        record: &ToolDefaultRecord,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 删除默认参数，返回是否有行被删除。
    fn delete_tool_default(
        &self,
        tool_name: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    /// 列出全部默认参数（按 tool_name 升序）。
    fn list_tool_defaults(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<ToolDefaultRecord>, StoreError>> + Send;
}

impl ToolDefaultsRepository for () {
    async fn get_tool_default(
        &self,
        _tool_name: &str,
    ) -> Result<Option<ToolDefaultRecord>, StoreError> {
        Ok(None)
    }
    async fn set_tool_default(&self, _record: &ToolDefaultRecord) -> Result<(), StoreError> {
        Ok(())
    }
    async fn delete_tool_default(&self, _tool_name: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn list_tool_defaults(&self) -> Result<Vec<ToolDefaultRecord>, StoreError> {
        Ok(Vec::new())
    }
}

fn row_to_record(row: SqliteRow) -> Result<ToolDefaultRecord, StoreError> {
    Ok(ToolDefaultRecord {
        tool_name: row.try_get("tool_name").map_err(StoreError::from)?,
        args_json: row.try_get("args_json").map_err(StoreError::from)?,
        source: row.try_get("source").map_err(StoreError::from)?,
        updated_by: row.try_get("updated_by").map_err(StoreError::from)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::from)?,
    })
}

impl ToolDefaultsRepository for SqliteRequestEventRepository {
    async fn get_tool_default(
        &self,
        tool_name: &str,
    ) -> Result<Option<ToolDefaultRecord>, StoreError> {
        sqlx::query(
            r#"
            SELECT tool_name, args_json, source, updated_by, updated_at
            FROM tool_defaults WHERE tool_name = ?
            "#,
        )
        .bind(tool_name)
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?
        .map(row_to_record)
        .transpose()
    }

    async fn set_tool_default(&self, record: &ToolDefaultRecord) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO tool_defaults (tool_name, args_json, source, updated_by, updated_at)
            VALUES (?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT (tool_name) DO UPDATE SET
                args_json  = excluded.args_json,
                source     = excluded.source,
                updated_by = excluded.updated_by,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&record.tool_name)
        .bind(&record.args_json)
        .bind(&record.source)
        .bind(&record.updated_by)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn delete_tool_default(&self, tool_name: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM tool_defaults WHERE tool_name = ?")
            .bind(tool_name)
            .execute(self.pool())
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_tool_defaults(&self) -> Result<Vec<ToolDefaultRecord>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT tool_name, args_json, source, updated_by, updated_at
            FROM tool_defaults ORDER BY tool_name
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

    fn record(tool_name: &str, args_json: &str, source: &str) -> ToolDefaultRecord {
        ToolDefaultRecord {
            tool_name: tool_name.to_string(),
            args_json: args_json.to_string(),
            source: source.to_string(),
            updated_by: Some("ops".to_string()),
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn set_get_roundtrip_and_upsert_overwrites() {
        let repo = repo().await;
        repo.set_tool_default(&record("search__mock__search", r#"{"q":"a"}"#, "manual"))
            .await
            .unwrap();

        let got = repo
            .get_tool_default("search__mock__search")
            .await
            .unwrap()
            .expect("present");
        assert_eq!(got.args_json, r#"{"q":"a"}"#);
        assert_eq!(got.source, "manual");
        assert_eq!(got.updated_by.as_deref(), Some("ops"));
        assert!(!got.updated_at.is_empty());

        // 同名 upsert 覆盖 args/source
        repo.set_tool_default(&record("search__mock__search", r#"{"q":"b"}"#, "captured"))
            .await
            .unwrap();
        let got = repo
            .get_tool_default("search__mock__search")
            .await
            .unwrap()
            .expect("present");
        assert_eq!(got.args_json, r#"{"q":"b"}"#);
        assert_eq!(got.source, "captured");
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let repo = repo().await;
        assert!(
            repo.get_tool_default("no__such__tool")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_reports_whether_row_existed() {
        let repo = repo().await;
        repo.set_tool_default(&record("a__b__c", "{}", "manual"))
            .await
            .unwrap();
        assert!(repo.delete_tool_default("a__b__c").await.unwrap());
        assert!(!repo.delete_tool_default("a__b__c").await.unwrap());
        assert!(repo.get_tool_default("a__b__c").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_returns_all_sorted_by_tool_name() {
        let repo = repo().await;
        repo.set_tool_default(&record("b__b__b", "{}", "manual"))
            .await
            .unwrap();
        repo.set_tool_default(&record("a__a__a", "{}", "captured"))
            .await
            .unwrap();
        let all = repo.list_tool_defaults().await.unwrap();
        assert_eq!(
            all.iter().map(|r| r.tool_name.as_str()).collect::<Vec<_>>(),
            vec!["a__a__a", "b__b__b"]
        );
    }

    #[tokio::test]
    async fn noop_impl_returns_empty() {
        assert!(().get_tool_default("x").await.unwrap().is_none());
        assert!(().list_tool_defaults().await.unwrap().is_empty());
        assert!(!().delete_tool_default("x").await.unwrap());
    }
}
