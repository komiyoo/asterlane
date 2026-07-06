//! Asterlane 错误模型：稳定错误码、顶层聚合错误类型与边界转换。
//!
//! 设计依据见 `docs/error-model.md`。边界转换返回纯数据类型，
//! 不依赖 axum/reqwest/sqlx，由调用方转换为实际 CLI/HTTP/MCP 输出。

use crate::catalog::CatalogError;
use crate::naming::ToolNameError;
use crate::policy::PolicyError;
use std::fmt::{Display, Formatter};
use thiserror::Error;

/// 稳定错误码枚举，跨版本不变。
///
/// 错误码字符串值一经发布不得变更（见 `docs/compatibility-policy.md`）。
/// 新增错误码不算 breaking；删除/合并需经过弃用周期。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    // ── config ──
    /// YAML 解析失败。
    ConfigInvalidYaml,
    /// 引用不存在的 resource_id。
    ConfigUnknownResource,
    /// scope 正则编译失败。
    ConfigInvalidRegex,
    /// 工具名段不合法或超长。
    ConfigInvalidToolName,

    // ── auth ──
    /// 请求未携带 gateway key。
    AuthMissingGatewayKey,
    /// gateway key 未识别。
    AuthInvalidGatewayKey,
    /// gateway key token 已过期。
    AuthExpiredGatewayKey,
    /// key scope 不允许该工具。
    AuthForbiddenTool,
    /// secret ref 解析失败。
    AuthMissingUpstreamSecret,

    // ── catalog ──
    /// 调用不存在的工具。
    CatalogUnknownTool,
    /// cursor 非法或 limit 越界。
    CatalogInvalidPagination,

    // ── store ──
    /// 数据库迁移失败。
    StoreMigrationFailed,
    /// 仓库不可用。
    StoreUnavailable,

    // ── proxy ──
    /// 上游超时。
    ProxyUpstreamTimeout,
    /// 重试耗尽。
    ProxyRetryExhausted,
    /// 上游 4xx/5xx。
    ProxyUpstreamError,
    /// 上游连接失败。
    ProxyConnectionFailed,

    // ── limit ──
    /// 配额耗尽。
    LimitQuotaExceeded,
    /// 队列满。
    LimitQueueFull,
    /// 排队超时。
    LimitQueueTimeout,
    /// Per-key 累计调用配额（`max_calls`）耗尽。
    LimitCallsExhausted,
    /// Per-key 当日调用配额（`max_calls_per_day`）耗尽，UTC 零点重置。
    LimitDailyCallsExhausted,

    // ── mcp ──
    /// 参数不合法。
    McpInvalidToolCall,
    /// 上游 MCP server 失败。
    McpUpstreamMcpFailure,

    // ── transform ──
    /// 变换规则尝试设置危险 header。
    TransformDangerousHeader,
    /// JSON Pointer 路径不合法。
    TransformInvalidPointer,

    // ── admin ──
    /// admin token 缺失或不匹配。
    AdminUnauthorized,
    /// admin 查询参数不合法。
    AdminInvalidQuery,
    /// admin 管理的实体未找到。
    AdminNotFound,
    /// admin 写操作冲突（如 ID 重复）。
    AdminConflict,
}

impl ErrorCode {
    /// 返回错误码的稳定字符串值，如 `"config.invalid_yaml"`。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConfigInvalidYaml => "config.invalid_yaml",
            Self::ConfigUnknownResource => "config.unknown_resource",
            Self::ConfigInvalidRegex => "config.invalid_regex",
            Self::ConfigInvalidToolName => "config.invalid_tool_name",
            Self::AuthMissingGatewayKey => "auth.missing_gateway_key",
            Self::AuthInvalidGatewayKey => "auth.invalid_gateway_key",
            Self::AuthExpiredGatewayKey => "auth.expired_gateway_key",
            Self::AuthForbiddenTool => "auth.forbidden_tool",
            Self::AuthMissingUpstreamSecret => "auth.missing_upstream_secret",
            Self::CatalogUnknownTool => "catalog.unknown_tool",
            Self::CatalogInvalidPagination => "catalog.invalid_pagination",
            Self::StoreMigrationFailed => "store.migration_failed",
            Self::StoreUnavailable => "store.unavailable",
            Self::ProxyUpstreamTimeout => "proxy.upstream_timeout",
            Self::ProxyRetryExhausted => "proxy.retry_exhausted",
            Self::ProxyUpstreamError => "proxy.upstream_error",
            Self::ProxyConnectionFailed => "proxy.connection_failed",
            Self::LimitQuotaExceeded => "limit.quota_exceeded",
            Self::LimitQueueFull => "limit.queue_full",
            Self::LimitQueueTimeout => "limit.queue_timeout",
            Self::LimitCallsExhausted => "limit.calls_exhausted",
            Self::LimitDailyCallsExhausted => "limit.daily_calls_exhausted",
            Self::McpInvalidToolCall => "mcp.invalid_tool_call",
            Self::McpUpstreamMcpFailure => "mcp.upstream_mcp_failure",
            Self::TransformDangerousHeader => "transform.dangerous_header",
            Self::TransformInvalidPointer => "transform.invalid_pointer",
            Self::AdminUnauthorized => "admin.unauthorized",
            Self::AdminInvalidQuery => "admin.invalid_query",
            Self::AdminNotFound => "admin.not_found",
            Self::AdminConflict => "admin.conflict",
        }
    }

    /// 返回错误码的 category 前缀，如 `"config"`、`"auth"`。
    pub fn category(&self) -> &'static str {
        match self {
            Self::ConfigInvalidYaml
            | Self::ConfigUnknownResource
            | Self::ConfigInvalidRegex
            | Self::ConfigInvalidToolName => "config",
            Self::AuthMissingGatewayKey
            | Self::AuthInvalidGatewayKey
            | Self::AuthExpiredGatewayKey
            | Self::AuthForbiddenTool
            | Self::AuthMissingUpstreamSecret => "auth",
            Self::CatalogUnknownTool | Self::CatalogInvalidPagination => "catalog",
            Self::StoreMigrationFailed | Self::StoreUnavailable => "store",
            Self::ProxyUpstreamTimeout
            | Self::ProxyRetryExhausted
            | Self::ProxyUpstreamError
            | Self::ProxyConnectionFailed => "proxy",
            Self::LimitQuotaExceeded
            | Self::LimitQueueFull
            | Self::LimitQueueTimeout
            | Self::LimitCallsExhausted
            | Self::LimitDailyCallsExhausted => "limit",
            Self::McpInvalidToolCall | Self::McpUpstreamMcpFailure => "mcp",
            Self::TransformDangerousHeader | Self::TransformInvalidPointer => "transform",
            Self::AdminUnauthorized
            | Self::AdminInvalidQuery
            | Self::AdminNotFound
            | Self::AdminConflict => "admin",
        }
    }
}

impl Display for ErrorCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 顶层聚合错误类型。
///
/// 通过 `#[from]` 聚合批1 已有模块错误。批2模块（store/proxy/limit/mcp/secrets）
/// 通过 [`AsterlaneError::internal`] 构造 [`Self::Internal`] 变体接入，
/// 在自己模块 `impl From<ModuleError> for AsterlaneError` 中映射，无需修改本文件。
///
/// `Display` 输出为安全消息，不含 Authorization header、Bearer token、
/// `x-api-key` 值、上游响应体、secret ref 完整 URI 或 upstream key 明文。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AsterlaneError {
    /// 工具名解析或规范化失败。
    #[error(transparent)]
    ToolName(#[from] ToolNameError),
    /// Catalog 构建或查询失败。
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// 策略/scope 校验失败。
    #[error(transparent)]
    Policy(#[from] PolicyError),
    /// 兜底变体：供批2模块错误接入。
    ///
    /// `message` 必须是可安全展示的脱敏消息。
    #[error("{code}: {message}")]
    Internal {
        code: ErrorCode,
        message: String,
        retry_after: Option<std::time::Duration>,
    },
}

impl AsterlaneError {
    /// 构造 `Internal` 兜底变体。
    pub fn internal(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Internal {
            code,
            message: message.into(),
            retry_after: None,
        }
    }

    /// 构造带 `Retry-After` 的 `Internal` 变体（限流场景）。
    pub fn internal_with_retry_after(
        code: ErrorCode,
        message: impl Into<String>,
        retry_after: Option<std::time::Duration>,
    ) -> Self {
        Self::Internal {
            code,
            message: message.into(),
            retry_after,
        }
    }

    /// 返回该错误的稳定错误码。
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::ToolName(_) => ErrorCode::ConfigInvalidToolName,
            Self::Catalog(err) => match err {
                CatalogError::ToolName(_) => ErrorCode::ConfigInvalidToolName,
                CatalogError::Regex(_) => ErrorCode::ConfigInvalidRegex,
                CatalogError::Policy(_) => ErrorCode::ConfigInvalidRegex,
                CatalogError::OpenApi(_) => ErrorCode::ConfigInvalidYaml,
            },
            Self::Policy(_) => ErrorCode::ConfigInvalidRegex,
            Self::Internal { code, .. } => *code,
        }
    }

    /// CLI 退出码映射（见 error-model.md）。
    ///
    /// | Category   | 退出码 |
    /// |------------|--------|
    /// | config     | 2      |
    /// | auth       | 3      |
    /// | catalog/mcp| 4      |
    /// | store      | 5      |
    /// | proxy      | 6      |
    /// | limit      | 7      |
    /// | 其他       | 1      |
    pub fn exit_code(&self) -> i32 {
        match self.error_code().category() {
            "config" => 2,
            "auth" | "admin" => 3,
            "catalog" | "mcp" => 4,
            "store" => 5,
            "proxy" => 6,
            "limit" => 7,
            "transform" => 8,
            _ => 1,
        }
    }

    /// HTTP 边界转换：返回 HTTP status + 安全消息 + 错误码（见 error-model.md）。
    ///
    /// `request_id` 始终为 `None`，由 HTTP handler 在响应时填入。
    pub fn http_response(&self) -> HttpErrorView {
        let code = self.error_code();
        let retry_after = match self {
            Self::Internal { retry_after, .. } => *retry_after,
            _ => None,
        };
        HttpErrorView {
            status: http_status_for(code),
            code,
            message: self.safe_message(),
            request_id: None,
            retry_after,
        }
    }

    /// MCP 边界转换（见 error-model.md）。
    ///
    /// 上游 4xx/5xx/超时/重试耗尽/配额限流 → `ToolResultIsError`；
    /// 未知工具 → -32601；参数错误 → -32602；网关自身故障 → -32603。
    pub fn mcp_error(&self) -> McpErrorForm {
        let code = self.error_code();
        let message = self.safe_message();
        match code {
            // 未知工具 → Method not found
            ErrorCode::CatalogUnknownTool => McpErrorForm::JsonRpc(-32601, message),
            // 参数错误 → Invalid params
            ErrorCode::CatalogInvalidPagination | ErrorCode::McpInvalidToolCall => {
                McpErrorForm::JsonRpc(-32602, message)
            }
            // 上游类错误 / 配额限流 → tool result isError: true
            ErrorCode::ProxyUpstreamTimeout
            | ErrorCode::ProxyRetryExhausted
            | ErrorCode::ProxyUpstreamError
            | ErrorCode::ProxyConnectionFailed
            | ErrorCode::McpUpstreamMcpFailure
            | ErrorCode::LimitQuotaExceeded
            | ErrorCode::LimitQueueFull
            | ErrorCode::LimitQueueTimeout
            | ErrorCode::LimitCallsExhausted => McpErrorForm::ToolResultIsError(message),
            // 网关自身故障 → Internal error
            _ => McpErrorForm::JsonRpc(-32603, message),
        }
    }

    /// 返回可安全展示的消息（脱敏）。
    ///
    /// 模块错误使用其 `Display` 输出（模块错误构造时不携带明文密钥）。
    /// `Internal` 变体使用构造时提供的 message。
    fn safe_message(&self) -> String {
        match self {
            Self::ToolName(err) => err.to_string(),
            Self::Catalog(err) => err.to_string(),
            Self::Policy(err) => err.to_string(),
            Self::Internal { message, .. } => message.clone(),
        }
    }
}

/// HTTP 边界返回的纯数据视图。
///
/// 不依赖 axum/reqwest，由调用方转换为实际 HTTP 响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpErrorView {
    /// HTTP status code。
    pub status: u16,
    /// 稳定错误码。
    pub code: ErrorCode,
    /// 可安全展示的消息。
    pub message: String,
    /// 请求 ID（由 HTTP handler 填入）。
    pub request_id: Option<String>,
    /// 限流时的 Retry-After 值。
    pub retry_after: Option<std::time::Duration>,
}

/// MCP 边界返回的错误形态。
///
/// - `ToolResultIsError`：作为 tool result `isError: true` 返回给 LLM。
/// - `JsonRpc`：作为 JSON-RPC error 返回给基础设施。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum McpErrorForm {
    /// tool result `isError: true`，内容为清洗后的文本。
    ToolResultIsError(String),
    /// JSON-RPC error，含 error code 和脱敏 message。
    JsonRpc(i32, String),
}

/// 按 error-model.md HTTP status 表返回 status code。
fn http_status_for(code: ErrorCode) -> u16 {
    match code {
        ErrorCode::ConfigInvalidYaml
        | ErrorCode::ConfigUnknownResource
        | ErrorCode::ConfigInvalidRegex
        | ErrorCode::ConfigInvalidToolName => 500,
        ErrorCode::AuthMissingGatewayKey
        | ErrorCode::AuthInvalidGatewayKey
        | ErrorCode::AuthExpiredGatewayKey => 401,
        ErrorCode::AuthForbiddenTool => 403,
        ErrorCode::AuthMissingUpstreamSecret => 503,
        ErrorCode::CatalogUnknownTool => 404,
        ErrorCode::McpUpstreamMcpFailure => 502,
        ErrorCode::CatalogInvalidPagination | ErrorCode::McpInvalidToolCall => 400,
        ErrorCode::StoreMigrationFailed | ErrorCode::StoreUnavailable => 503,
        ErrorCode::ProxyUpstreamTimeout | ErrorCode::ProxyConnectionFailed => 504,
        ErrorCode::ProxyRetryExhausted | ErrorCode::ProxyUpstreamError => 502,
        ErrorCode::LimitQuotaExceeded
        | ErrorCode::LimitCallsExhausted
        | ErrorCode::LimitDailyCallsExhausted => 429,
        ErrorCode::LimitQueueFull | ErrorCode::LimitQueueTimeout => 503,
        ErrorCode::TransformDangerousHeader | ErrorCode::TransformInvalidPointer => 500,
        ErrorCode::AdminUnauthorized => 401,
        ErrorCode::AdminInvalidQuery => 400,
        ErrorCode::AdminNotFound => 404,
        ErrorCode::AdminConflict => 409,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogError;
    use crate::naming::ToolNameError;
    use crate::policy::PolicyError;

    // ── ErrorCode: as_str / category ──

    #[test]
    fn error_code_as_str_covers_all_spec_codes() {
        assert_eq!(ErrorCode::ConfigInvalidYaml.as_str(), "config.invalid_yaml");
        assert_eq!(
            ErrorCode::ConfigUnknownResource.as_str(),
            "config.unknown_resource"
        );
        assert_eq!(
            ErrorCode::ConfigInvalidRegex.as_str(),
            "config.invalid_regex"
        );
        assert_eq!(
            ErrorCode::ConfigInvalidToolName.as_str(),
            "config.invalid_tool_name"
        );
        assert_eq!(
            ErrorCode::AuthMissingGatewayKey.as_str(),
            "auth.missing_gateway_key"
        );
        assert_eq!(
            ErrorCode::AuthInvalidGatewayKey.as_str(),
            "auth.invalid_gateway_key"
        );
        assert_eq!(ErrorCode::AuthForbiddenTool.as_str(), "auth.forbidden_tool");
        assert_eq!(
            ErrorCode::AuthMissingUpstreamSecret.as_str(),
            "auth.missing_upstream_secret"
        );
        assert_eq!(
            ErrorCode::CatalogUnknownTool.as_str(),
            "catalog.unknown_tool"
        );
        assert_eq!(
            ErrorCode::CatalogInvalidPagination.as_str(),
            "catalog.invalid_pagination"
        );
        assert_eq!(
            ErrorCode::StoreMigrationFailed.as_str(),
            "store.migration_failed"
        );
        assert_eq!(ErrorCode::StoreUnavailable.as_str(), "store.unavailable");
        assert_eq!(
            ErrorCode::ProxyUpstreamTimeout.as_str(),
            "proxy.upstream_timeout"
        );
        assert_eq!(
            ErrorCode::ProxyRetryExhausted.as_str(),
            "proxy.retry_exhausted"
        );
        assert_eq!(
            ErrorCode::ProxyUpstreamError.as_str(),
            "proxy.upstream_error"
        );
        assert_eq!(
            ErrorCode::ProxyConnectionFailed.as_str(),
            "proxy.connection_failed"
        );
        assert_eq!(
            ErrorCode::LimitQuotaExceeded.as_str(),
            "limit.quota_exceeded"
        );
        assert_eq!(ErrorCode::LimitQueueFull.as_str(), "limit.queue_full");
        assert_eq!(ErrorCode::LimitQueueTimeout.as_str(), "limit.queue_timeout");
        assert_eq!(
            ErrorCode::LimitCallsExhausted.as_str(),
            "limit.calls_exhausted"
        );
        assert_eq!(
            ErrorCode::McpInvalidToolCall.as_str(),
            "mcp.invalid_tool_call"
        );
        assert_eq!(
            ErrorCode::McpUpstreamMcpFailure.as_str(),
            "mcp.upstream_mcp_failure"
        );
        assert_eq!(ErrorCode::AdminUnauthorized.as_str(), "admin.unauthorized");
        assert_eq!(ErrorCode::AdminInvalidQuery.as_str(), "admin.invalid_query");
        assert_eq!(ErrorCode::AdminNotFound.as_str(), "admin.not_found");
        assert_eq!(ErrorCode::AdminConflict.as_str(), "admin.conflict");
    }

    #[test]
    fn error_code_category_groups_correctly() {
        let config_codes = [
            ErrorCode::ConfigInvalidYaml,
            ErrorCode::ConfigUnknownResource,
            ErrorCode::ConfigInvalidRegex,
            ErrorCode::ConfigInvalidToolName,
        ];
        for code in config_codes {
            assert_eq!(code.category(), "config", "{code} should be config");
        }

        let auth_codes = [
            ErrorCode::AuthMissingGatewayKey,
            ErrorCode::AuthInvalidGatewayKey,
            ErrorCode::AuthForbiddenTool,
            ErrorCode::AuthMissingUpstreamSecret,
        ];
        for code in auth_codes {
            assert_eq!(code.category(), "auth", "{code} should be auth");
        }

        assert_eq!(ErrorCode::CatalogUnknownTool.category(), "catalog");
        assert_eq!(ErrorCode::CatalogInvalidPagination.category(), "catalog");
        assert_eq!(ErrorCode::StoreMigrationFailed.category(), "store");
        assert_eq!(ErrorCode::StoreUnavailable.category(), "store");
        assert_eq!(ErrorCode::ProxyUpstreamTimeout.category(), "proxy");
        assert_eq!(ErrorCode::ProxyRetryExhausted.category(), "proxy");
        assert_eq!(ErrorCode::ProxyUpstreamError.category(), "proxy");
        assert_eq!(ErrorCode::ProxyConnectionFailed.category(), "proxy");
        assert_eq!(ErrorCode::LimitQuotaExceeded.category(), "limit");
        assert_eq!(ErrorCode::LimitQueueFull.category(), "limit");
        assert_eq!(ErrorCode::LimitQueueTimeout.category(), "limit");
        assert_eq!(ErrorCode::LimitCallsExhausted.category(), "limit");
        assert_eq!(ErrorCode::McpInvalidToolCall.category(), "mcp");
        assert_eq!(ErrorCode::McpUpstreamMcpFailure.category(), "mcp");
        assert_eq!(ErrorCode::AdminUnauthorized.category(), "admin");
        assert_eq!(ErrorCode::AdminInvalidQuery.category(), "admin");
        assert_eq!(ErrorCode::AdminNotFound.category(), "admin");
        assert_eq!(ErrorCode::AdminConflict.category(), "admin");
    }

    #[test]
    fn error_code_display_matches_as_str() {
        let code = ErrorCode::AuthForbiddenTool;
        assert_eq!(code.to_string(), code.as_str());
    }

    // ── error_code mapping from module errors ──

    #[test]
    fn tool_name_error_maps_to_config_invalid_tool_name() {
        let err = AsterlaneError::from(ToolNameError::EmptySegment);
        assert_eq!(err.error_code(), ErrorCode::ConfigInvalidToolName);
    }

    #[test]
    fn policy_error_maps_to_config_invalid_regex() {
        let bad_pattern = "(".to_string();
        let regex_err = regex::Regex::new(&bad_pattern).unwrap_err();
        let err = AsterlaneError::from(PolicyError::InvalidRegex(regex_err));
        assert_eq!(err.error_code(), ErrorCode::ConfigInvalidRegex);
    }

    #[test]
    fn catalog_tool_name_error_maps_to_config_invalid_tool_name() {
        let err = AsterlaneError::from(CatalogError::ToolName(ToolNameError::InvalidShape(
            "bad".to_string(),
        )));
        assert_eq!(err.error_code(), ErrorCode::ConfigInvalidToolName);
    }

    #[test]
    fn catalog_regex_error_maps_to_config_invalid_regex() {
        let bad_pattern = "(".to_string();
        let regex_err = regex::Regex::new(&bad_pattern).unwrap_err();
        let err = AsterlaneError::from(CatalogError::Regex(regex_err));
        assert_eq!(err.error_code(), ErrorCode::ConfigInvalidRegex);
    }

    #[test]
    fn catalog_policy_error_maps_to_config_invalid_regex() {
        let bad_pattern = "(".to_string();
        let regex_err = regex::Regex::new(&bad_pattern).unwrap_err();
        let err = AsterlaneError::from(CatalogError::Policy(PolicyError::InvalidRegex(regex_err)));
        assert_eq!(err.error_code(), ErrorCode::ConfigInvalidRegex);
    }

    // ── exit_code ──

    #[test]
    fn exit_code_config_is_2() {
        let err = AsterlaneError::from(ToolNameError::EmptySegment);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn exit_code_auth_is_3() {
        let err = AsterlaneError::internal(ErrorCode::AuthMissingGatewayKey, "missing key");
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn exit_code_catalog_is_4() {
        let err = AsterlaneError::internal(ErrorCode::CatalogUnknownTool, "unknown tool");
        assert_eq!(err.exit_code(), 4);
    }

    #[test]
    fn exit_code_mcp_is_4() {
        let err = AsterlaneError::internal(ErrorCode::McpInvalidToolCall, "bad args");
        assert_eq!(err.exit_code(), 4);
    }

    #[test]
    fn exit_code_store_is_5() {
        let err = AsterlaneError::internal(ErrorCode::StoreUnavailable, "down");
        assert_eq!(err.exit_code(), 5);
    }

    #[test]
    fn exit_code_proxy_is_6() {
        let err = AsterlaneError::internal(ErrorCode::ProxyUpstreamTimeout, "timeout");
        assert_eq!(err.exit_code(), 6);
    }

    #[test]
    fn exit_code_limit_is_7() {
        let err = AsterlaneError::internal(ErrorCode::LimitQuotaExceeded, "exceeded");
        assert_eq!(err.exit_code(), 7);
    }

    // ── http_response ──

    #[test]
    fn http_config_errors_return_500() {
        let err = AsterlaneError::from(ToolNameError::EmptySegment);
        let view = err.http_response();
        assert_eq!(view.status, 500);
        assert_eq!(view.code, ErrorCode::ConfigInvalidToolName);
        assert!(view.request_id.is_none());
    }

    #[test]
    fn http_auth_missing_key_returns_401() {
        let err = AsterlaneError::internal(ErrorCode::AuthMissingGatewayKey, "missing key");
        let view = err.http_response();
        assert_eq!(view.status, 401);
    }

    #[test]
    fn http_auth_invalid_key_returns_401() {
        let err = AsterlaneError::internal(ErrorCode::AuthInvalidGatewayKey, "bad key");
        let view = err.http_response();
        assert_eq!(view.status, 401);
    }

    #[test]
    fn http_auth_forbidden_returns_403() {
        let err = AsterlaneError::internal(ErrorCode::AuthForbiddenTool, "forbidden");
        let view = err.http_response();
        assert_eq!(view.status, 403);
    }

    #[test]
    fn http_auth_missing_secret_returns_503() {
        let err = AsterlaneError::internal(ErrorCode::AuthMissingUpstreamSecret, "no secret");
        let view = err.http_response();
        assert_eq!(view.status, 503);
    }

    #[test]
    fn http_catalog_unknown_tool_returns_404() {
        let err = AsterlaneError::internal(ErrorCode::CatalogUnknownTool, "unknown tool");
        let view = err.http_response();
        assert_eq!(view.status, 404);
    }

    #[test]
    fn http_catalog_invalid_pagination_returns_400() {
        let err = AsterlaneError::internal(ErrorCode::CatalogInvalidPagination, "bad cursor");
        let view = err.http_response();
        assert_eq!(view.status, 400);
    }

    #[test]
    fn http_store_errors_return_503() {
        let err = AsterlaneError::internal(ErrorCode::StoreUnavailable, "down");
        let view = err.http_response();
        assert_eq!(view.status, 503);
    }

    #[test]
    fn http_proxy_timeout_returns_504() {
        let err = AsterlaneError::internal(ErrorCode::ProxyUpstreamTimeout, "timeout");
        let view = err.http_response();
        assert_eq!(view.status, 504);
    }

    #[test]
    fn http_proxy_connection_failed_returns_504() {
        let err = AsterlaneError::internal(ErrorCode::ProxyConnectionFailed, "conn failed");
        let view = err.http_response();
        assert_eq!(view.status, 504);
    }

    #[test]
    fn http_proxy_upstream_error_returns_502() {
        let err = AsterlaneError::internal(ErrorCode::ProxyUpstreamError, "500 from upstream");
        let view = err.http_response();
        assert_eq!(view.status, 502);
    }

    #[test]
    fn http_proxy_retry_exhausted_returns_502() {
        let err = AsterlaneError::internal(ErrorCode::ProxyRetryExhausted, "retries done");
        let view = err.http_response();
        assert_eq!(view.status, 502);
    }

    #[test]
    fn http_limit_quota_returns_429() {
        let err = AsterlaneError::internal(ErrorCode::LimitQuotaExceeded, "quota exceeded");
        let view = err.http_response();
        assert_eq!(view.status, 429);
    }

    #[test]
    fn http_limit_queue_full_returns_503() {
        let err = AsterlaneError::internal(ErrorCode::LimitQueueFull, "queue full");
        let view = err.http_response();
        assert_eq!(view.status, 503);
    }

    #[test]
    fn http_limit_calls_exhausted_returns_429() {
        let err = AsterlaneError::internal(ErrorCode::LimitCallsExhausted, "quota exhausted");
        let view = err.http_response();
        assert_eq!(view.status, 429);
    }

    #[test]
    fn http_mcp_invalid_tool_call_returns_400() {
        let err = AsterlaneError::internal(ErrorCode::McpInvalidToolCall, "bad args");
        let view = err.http_response();
        assert_eq!(view.status, 400);
    }

    #[test]
    fn http_mcp_upstream_failure_returns_502() {
        let err = AsterlaneError::internal(ErrorCode::McpUpstreamMcpFailure, "upstream mcp error");
        let view = err.http_response();
        assert_eq!(view.status, 502);
    }

    #[test]
    fn http_response_message_is_safe() {
        let err = AsterlaneError::internal(ErrorCode::AuthForbiddenTool, "tool not permitted");
        let view = err.http_response();
        assert!(!view.message.contains("Bearer"));
        assert!(!view.message.contains("Authorization"));
        assert!(!view.message.contains("x-api-key"));
    }

    // ── mcp_error ──

    #[test]
    fn mcp_catalog_unknown_tool_returns_jsonrpc_32601() {
        let err = AsterlaneError::internal(ErrorCode::CatalogUnknownTool, "unknown tool");
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, _) => assert_eq!(code, -32601),
            other => panic!("expected JsonRpc, got {other:?}"),
        }
    }

    #[test]
    fn mcp_catalog_invalid_pagination_returns_jsonrpc_32602() {
        let err = AsterlaneError::internal(ErrorCode::CatalogInvalidPagination, "bad cursor");
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, _) => assert_eq!(code, -32602),
            other => panic!("expected JsonRpc, got {other:?}"),
        }
    }

    #[test]
    fn mcp_invalid_tool_call_returns_jsonrpc_32602() {
        let err = AsterlaneError::internal(ErrorCode::McpInvalidToolCall, "bad args");
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, _) => assert_eq!(code, -32602),
            other => panic!("expected JsonRpc, got {other:?}"),
        }
    }

    #[test]
    fn mcp_config_error_returns_jsonrpc_32603() {
        let err = AsterlaneError::from(ToolNameError::EmptySegment);
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, _) => assert_eq!(code, -32603),
            other => panic!("expected JsonRpc, got {other:?}"),
        }
    }

    #[test]
    fn mcp_auth_error_returns_jsonrpc_32603() {
        let err = AsterlaneError::internal(ErrorCode::AuthForbiddenTool, "forbidden");
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, _) => assert_eq!(code, -32603),
            other => panic!("expected JsonRpc, got {other:?}"),
        }
    }

    #[test]
    fn mcp_store_error_returns_jsonrpc_32603() {
        let err = AsterlaneError::internal(ErrorCode::StoreUnavailable, "down");
        match err.mcp_error() {
            McpErrorForm::JsonRpc(code, _) => assert_eq!(code, -32603),
            other => panic!("expected JsonRpc, got {other:?}"),
        }
    }

    #[test]
    fn mcp_proxy_timeout_returns_tool_result_is_error() {
        let err = AsterlaneError::internal(ErrorCode::ProxyUpstreamTimeout, "timeout");
        match err.mcp_error() {
            McpErrorForm::ToolResultIsError(msg) => assert!(msg.contains("timeout")),
            other => panic!("expected ToolResultIsError, got {other:?}"),
        }
    }

    #[test]
    fn mcp_proxy_upstream_error_returns_tool_result_is_error() {
        let err = AsterlaneError::internal(ErrorCode::ProxyUpstreamError, "upstream 500");
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_proxy_retry_exhausted_returns_tool_result_is_error() {
        let err = AsterlaneError::internal(ErrorCode::ProxyRetryExhausted, "retries done");
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_proxy_connection_failed_returns_tool_result_is_error() {
        let err = AsterlaneError::internal(ErrorCode::ProxyConnectionFailed, "conn failed");
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_upstream_mcp_failure_returns_tool_result_is_error() {
        let err = AsterlaneError::internal(ErrorCode::McpUpstreamMcpFailure, "upstream mcp error");
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_limit_quota_returns_tool_result_is_error() {
        let err = AsterlaneError::internal(ErrorCode::LimitQuotaExceeded, "quota exceeded");
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_limit_queue_full_returns_tool_result_is_error() {
        let err = AsterlaneError::internal(ErrorCode::LimitQueueFull, "queue full");
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    #[test]
    fn mcp_limit_queue_timeout_returns_tool_result_is_error() {
        let err = AsterlaneError::internal(ErrorCode::LimitQueueTimeout, "queue timeout");
        assert!(matches!(
            err.mcp_error(),
            McpErrorForm::ToolResultIsError(_)
        ));
    }

    // ── Internal fallback ──

    #[test]
    fn internal_variant_preserves_code_and_message() {
        let err =
            AsterlaneError::internal(ErrorCode::StoreMigrationFailed, "database migration failed");
        assert_eq!(err.error_code(), ErrorCode::StoreMigrationFailed);
        assert_eq!(err.exit_code(), 5);
        let view = err.http_response();
        assert_eq!(view.status, 503);
        assert_eq!(view.message, "database migration failed");
    }

    #[test]
    fn internal_from_module_error_via_from_trait() {
        let tool_err = ToolNameError::InvalidSegment("bad@segment".to_string());
        let err: AsterlaneError = tool_err.into();
        assert_eq!(err.error_code(), ErrorCode::ConfigInvalidToolName);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn internal_display_does_not_leak_secrets() {
        let err = AsterlaneError::internal(
            ErrorCode::AuthMissingUpstreamSecret,
            "upstream secret unavailable for resource tavily",
        );
        let display = err.to_string();
        assert!(!display.contains("Bearer"));
        assert!(!display.contains("secret://"));
        assert!(!display.contains("x-api-key"));
    }

    #[test]
    fn http_view_has_no_request_id_by_default() {
        let err = AsterlaneError::internal(ErrorCode::StoreUnavailable, "down");
        let view = err.http_response();
        assert!(view.request_id.is_none());
    }
}
