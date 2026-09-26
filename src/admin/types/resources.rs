//! 资源写入参数与列表响应。`auth` / `key_pool` 省略语义由 `resource_keys` 保持。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{ApiResource, KeyPoolConfig, UpstreamAuth, UpstreamLimits};

use super::AuthTypeResponse;

/// `POST/PUT /admin/resources` 的请求体。更新时省略 `auth` 或 `key_pool` 表示保留原值。
#[derive(Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) struct ResourceWriteParams {
    pub id: String,
    pub domain: String,
    #[serde(default)]
    pub provider: String,
    pub base_url: String,
    #[serde(default)]
    pub description: String,
    /// 上游限额。`0` 在替换配置时拒绝。
    #[serde(default)]
    pub limits: Option<UpstreamLimits>,
    /// 创建缺省为无凭据；更新时省略则保留已有值。
    #[serde(default)]
    pub auth: Option<UpstreamAuth>,
    /// 创建原样写入；更新时省略则保留已有值。
    #[serde(default)]
    pub key_pool: Option<KeyPoolConfig>,
}

impl std::fmt::Debug for ResourceWriteParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceWriteParams")
            .field("id", &self.id)
            .field("domain", &self.domain)
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("description", &self.description)
            .field("limits", &self.limits)
            .field("auth", &"<redacted>")
            .field(
                "key_pool_len",
                &self.key_pool.as_ref().map(|pool| pool.keys.len()),
            )
            .finish()
    }
}

/// `GET /admin/resources` 的一行。只暴露凭据形态和池大小。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ResourceSummaryResponse {
    pub id: String,
    pub domain: String,
    pub provider: String,
    pub base_url: String,
    pub endpoint_count: usize,
    pub auth_type: AuthTypeResponse,
    pub key_pool_size: usize,
}

impl ResourceSummaryResponse {
    pub(crate) fn from_resource(resource: &ApiResource) -> Self {
        Self {
            id: resource.id.clone(),
            domain: resource.domain.clone(),
            provider: resource.provider_or_id().to_string(),
            base_url: resource.base_url.clone(),
            endpoint_count: resource.endpoints.len(),
            auth_type: AuthTypeResponse::from_auth(&resource.auth),
            key_pool_size: resource
                .key_pool
                .as_ref()
                .map(|pool| pool.keys.len())
                .unwrap_or(0),
        }
    }
}
