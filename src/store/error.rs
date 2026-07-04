//! Store 模块错误类型与 `AsterlaneError` 接入
//! （见 `docs/error-model.md` store.* 错误码）。

use crate::error::{AsterlaneError, ErrorCode};
use thiserror::Error;

/// Store 模块错误。
#[derive(Debug, Error)]
pub enum StoreError {
    /// 数据库迁移失败。
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    /// 查询执行失败。
    #[error("database query failed: {0}")]
    Query(#[source] sqlx::Error),

    /// 实体未找到。
    #[error("entity not found: {0}")]
    NotFound(String),
}

impl From<sqlx::Error> for StoreError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => Self::NotFound("row not found".to_string()),
            _ => Self::Query(err),
        }
    }
}

impl From<StoreError> for AsterlaneError {
    fn from(err: StoreError) -> Self {
        let code = match &err {
            StoreError::Migration(_) => ErrorCode::StoreMigrationFailed,
            StoreError::Query(_) | StoreError::NotFound(_) => ErrorCode::StoreUnavailable,
        };
        AsterlaneError::internal(code, err.to_string())
    }
}

/// 构造解码错误（用于将字符串消息包装为 `sqlx::Error::Decode`）。
pub(crate) fn decode_error(msg: impl Into<String>) -> sqlx::Error {
    sqlx::Error::decode(DecodeStringError(msg.into()))
}

/// 内部辅助类型：将字符串消息包装为 `std::error::Error`，
/// 供 `sqlx::Error::Decode(BoxDynError)` 使用。
#[derive(Debug)]
struct DecodeStringError(String);

impl std::fmt::Display for DecodeStringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeStringError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_error_maps_to_store_migration_failed() {
        let err = StoreError::Migration(sqlx::migrate::MigrateError::VersionMissing(1));
        let aster = AsterlaneError::from(err);
        assert_eq!(aster.error_code(), ErrorCode::StoreMigrationFailed);
        assert_eq!(aster.exit_code(), 5);
    }

    #[test]
    fn query_error_maps_to_store_unavailable() {
        let err = StoreError::Query(sqlx::Error::RowNotFound);
        let aster = AsterlaneError::from(err);
        assert_eq!(aster.error_code(), ErrorCode::StoreUnavailable);
    }

    #[test]
    fn not_found_maps_to_store_unavailable() {
        let err = StoreError::NotFound("event req_123".to_string());
        let aster = AsterlaneError::from(err);
        assert_eq!(aster.error_code(), ErrorCode::StoreUnavailable);
        let view = aster.http_response();
        assert_eq!(view.status, 503);
    }

    #[test]
    fn row_not_found_sqlx_error_classifies_as_not_found() {
        let err = StoreError::from(sqlx::Error::RowNotFound);
        assert!(matches!(err, StoreError::NotFound(_)));
    }
}
