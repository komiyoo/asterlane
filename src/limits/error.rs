//! limits 模块错误类型与 `AsterlaneError` 接入
//! （见 `docs/error-model.md` limit.* 错误码）。

use crate::error::{AsterlaneError, ErrorCode};
use std::time::Duration;
use thiserror::Error;

/// limits 模块错误。
///
/// 构造时不携带明文密钥；`Display` 输出为可安全展示的脱敏消息。
#[derive(Debug, Error)]
pub enum LimitError {
    /// 配额耗尽。
    ///
    /// `dimension` 为限流维度名称（如 `"endpoint"`、`"upstream_key"`），
    /// `reset_after` 为距离下次可用的剩余时间（governor GCRA 提供）。
    #[error("quota exceeded for {dimension}")]
    QuotaExceeded {
        dimension: String,
        reset_after: Option<Duration>,
    },

    /// 队列满。
    #[error("request queue full")]
    QueueFull,

    /// 排队超时。
    #[error("request exceeded queue wait limit")]
    QueueTimeout,
}

/// 把 `LimitError` 映射为顶层 `AsterlaneError`。
///
/// 使用 `AsterlaneError::internal(code, message)` 构造 `Internal` 变体，
/// 不修改 `src/error.rs`。`message` 取自 `LimitError` 的 `Display` 输出
/// （已脱敏），由 `AsterlaneError::http_response()` / `mcp_error()` 在
/// 边界处转换为 HTTP status / MCP error form。
impl From<LimitError> for AsterlaneError {
    fn from(err: LimitError) -> Self {
        match &err {
            LimitError::QuotaExceeded { reset_after, .. } => {
                AsterlaneError::internal_with_retry_after(
                    ErrorCode::LimitQuotaExceeded,
                    err.to_string(),
                    *reset_after,
                )
            }
            LimitError::QueueFull => {
                AsterlaneError::internal(ErrorCode::LimitQueueFull, err.to_string())
            }
            LimitError::QueueTimeout => {
                AsterlaneError::internal(ErrorCode::LimitQueueTimeout, err.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_exceeded_maps_to_limit_quota_exceeded() {
        let err = AsterlaneError::from(LimitError::QuotaExceeded {
            dimension: "endpoint".to_string(),
            reset_after: None,
        });
        assert_eq!(err.error_code(), ErrorCode::LimitQuotaExceeded);
        assert_eq!(err.exit_code(), 7); // limit → 7
    }

    #[test]
    fn queue_full_maps_to_limit_queue_full() {
        let err = AsterlaneError::from(LimitError::QueueFull);
        assert_eq!(err.error_code(), ErrorCode::LimitQueueFull);
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn queue_timeout_maps_to_limit_queue_timeout() {
        let err = AsterlaneError::from(LimitError::QueueTimeout);
        assert_eq!(err.error_code(), ErrorCode::LimitQueueTimeout);
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn quota_exceeded_http_returns_429() {
        let err = AsterlaneError::from(LimitError::QuotaExceeded {
            dimension: "endpoint".to_string(),
            reset_after: None,
        });
        assert_eq!(err.http_response().status, 429);
    }

    #[test]
    fn queue_full_http_returns_503() {
        let err = AsterlaneError::from(LimitError::QueueFull);
        assert_eq!(err.http_response().status, 503);
    }

    #[test]
    fn queue_timeout_http_returns_503() {
        let err = AsterlaneError::from(LimitError::QueueTimeout);
        assert_eq!(err.http_response().status, 503);
    }

    #[test]
    fn quota_exceeded_mcp_returns_tool_result_is_error() {
        use crate::error::McpErrorForm;
        let err = AsterlaneError::from(LimitError::QuotaExceeded {
            dimension: "endpoint".to_string(),
            reset_after: None,
        });
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn error_message_does_not_leak_secrets() {
        let err = AsterlaneError::from(LimitError::QuotaExceeded {
            dimension: "endpoint".to_string(),
            reset_after: None,
        });
        let display = err.to_string();
        assert!(!display.contains("Bearer"));
        assert!(!display.contains("sk-"));
        assert!(!display.contains("Authorization"));
        assert!(!display.contains("x-api-key"));
    }
}
