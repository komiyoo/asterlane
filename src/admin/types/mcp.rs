//! MCP server 写入参数与读取响应。写入用嵌套 `defense.enabled`，读取用扁平 `defense_enabled`。

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{
    GatewayConfig, HealthCheckConfig, McpServerConfig, SecurityConfig, UpstreamAuth, UpstreamLimits,
};
use crate::integrity::IntegrityPolicy;
use crate::mcp::{HealthStatus, ServerHealth};
use crate::presets::{McpPreset, PresetAuth};

use super::AuthTypeResponse;

/// `POST/PUT /admin/mcp-servers` 的请求体。`PUT` 以路径 id 为准。
#[derive(Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) struct McpServerWriteParams {
    #[serde(default)]
    pub id: Option<String>,
    pub domain: String,
    pub provider: String,
    pub url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub auth: Option<UpstreamAuth>,
    #[serde(default)]
    pub security: Option<SecurityConfig>,
    #[serde(default)]
    pub limits: Option<UpstreamLimits>,
    #[serde(default)]
    pub health_check: Option<HealthCheckConfig>,
}

impl std::fmt::Debug for McpServerWriteParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServerWriteParams")
            .field("id", &self.id)
            .field("domain", &self.domain)
            .field("provider", &self.provider)
            .field("url", &self.url)
            .field("description", &self.description)
            .field("auth", &"<redacted>")
            .field("security", &self.security)
            .field("limits", &self.limits)
            .field("health_check", &self.health_check)
            .finish()
    }
}

/// 读取侧 security。与写入的 `SecurityConfig` 不是同一形状。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct McpSecurityResponse {
    pub integrity_policy: IntegrityPolicy,
    pub defense_enabled: bool,
    pub result_budget_bytes: Option<usize>,
}

/// 读取侧限额。不包含 `queue_timeout_secs`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct McpLimitsResponse {
    pub rps: Option<u32>,
    pub rpm: Option<u32>,
    pub max_concurrent: Option<u32>,
}

impl McpLimitsResponse {
    fn from_limits(limits: Option<&UpstreamLimits>) -> Self {
        Self {
            rps: limits.and_then(|item| item.rps),
            rpm: limits.and_then(|item| item.rpm),
            max_concurrent: limits.and_then(|item| item.max_concurrent),
        }
    }
}

/// 契约 health 对象。不含 `server_id` 与 `tool_count`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct McpHealthResponse {
    pub status: HealthStatus,
    pub last_check_at: Option<DateTime<Utc>>,
    pub last_ok_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<u64>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
}

impl McpHealthResponse {
    pub(crate) fn unknown() -> Self {
        Self {
            status: HealthStatus::Unknown,
            last_check_at: None,
            last_ok_at: None,
            latency_ms: None,
            consecutive_failures: 0,
            last_error: None,
        }
    }

    pub(crate) fn from_health(health: &ServerHealth) -> Self {
        Self {
            status: health.status,
            last_check_at: health.last_check_at,
            last_ok_at: health.last_ok_at,
            latency_ms: health.latency_ms,
            consecutive_failures: health.consecutive_failures,
            last_error: health.last_error.clone(),
        }
    }
}

/// `GET /admin/mcp-servers` 与创建/更新响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct McpServerResponse {
    pub id: String,
    pub domain: String,
    pub provider: String,
    pub url: String,
    pub description: String,
    pub builtin: bool,
    pub requires_key: bool,
    pub auth_type: AuthTypeResponse,
    pub security: McpSecurityResponse,
    pub limits: McpLimitsResponse,
    pub health_check_enabled: bool,
    pub health: McpHealthResponse,
    pub tool_count: usize,
}

impl McpServerResponse {
    pub(crate) fn from_config(
        server: &McpServerConfig,
        config: &GatewayConfig,
        health: Option<&ServerHealth>,
    ) -> Self {
        Self {
            id: server.id.clone(),
            domain: server.domain.clone(),
            provider: server.provider.clone(),
            url: server.url.clone(),
            description: server.description.clone(),
            builtin: config.builtin_mcp.contains(&server.id),
            requires_key: !matches!(server.auth, UpstreamAuth::None),
            auth_type: AuthTypeResponse::from_auth(&server.auth),
            security: McpSecurityResponse {
                integrity_policy: server.security.integrity_policy,
                defense_enabled: server.security.defense.enabled,
                result_budget_bytes: server.security.result_budget_bytes,
            },
            limits: McpLimitsResponse::from_limits(server.limits.as_ref()),
            health_check_enabled: server.health_check.enabled,
            health: health
                .map(McpHealthResponse::from_health)
                .unwrap_or_else(McpHealthResponse::unknown),
            tool_count: health.map_or(0, |item| item.tool_count),
        }
    }
}

/// 详情里的工具行。`input_schema` 保持任意 JSON。
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub(crate) struct McpServerToolResponse {
    pub description: String,
    pub description_override: Option<String>,
    pub wire_name: String,
    pub upstream_name: String,
    pub input_schema: Value,
}

/// `GET /admin/mcp-servers/{id}`。列表字段展平，另加 `tools`。
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub(crate) struct McpServerDetailResponse {
    #[serde(flatten)]
    pub server: McpServerResponse,
    pub tools: Vec<McpServerToolResponse>,
}

/// Preset 目录里的默认凭据形态。不含 secret ref。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum McpPresetAuthResponse {
    None,
    Bearer,
    Header { name: String },
}

impl McpPresetAuthResponse {
    fn from_preset(auth: PresetAuth) -> Self {
        match auth {
            PresetAuth::None => Self::None,
            PresetAuth::Bearer => Self::Bearer,
            PresetAuth::Header { name } => Self::Header {
                name: name.to_string(),
            },
        }
    }
}

/// `GET /admin/mcp-presets` 的一行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct McpPresetResponse {
    pub id: String,
    pub domain: String,
    pub provider: String,
    pub url: String,
    pub description: String,
    pub enabled: bool,
    pub auth: McpPresetAuthResponse,
    pub requires_key: bool,
    pub apply_url: Option<String>,
}

impl McpPresetResponse {
    pub(crate) fn from_preset(preset: &McpPreset, enabled: bool) -> Self {
        Self {
            id: preset.id.to_string(),
            domain: preset.domain.to_string(),
            provider: preset.provider.to_string(),
            url: preset.url.to_string(),
            description: preset.description.to_string(),
            enabled,
            auth: McpPresetAuthResponse::from_preset(preset.auth),
            requires_key: preset.requires_key(),
            apply_url: preset.apply_url.map(str::to_string),
        }
    }
}
