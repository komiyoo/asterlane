//! 管理 API 共用的确认响应与动态 JSON object。

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;

use crate::config::UpstreamAuth;

/// `GET /admin/health`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// `POST` 创建成功。字段名保持 `created`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct CreatedResponse {
    pub created: String,
}

/// `PUT` 更新成功。字段名保持 `updated`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct UpdatedResponse {
    pub updated: String,
}

/// `DELETE` 删除成功且响应为 JSON。字段名保持 `deleted`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct DeletedResponse {
    pub deleted: String,
}

/// 资源与 MCP 列表里的凭据形态。不含 secret ref。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthTypeResponse {
    None,
    Bearer,
    Header,
    /// serde 的 `snake_case` 会把 `OAuth` 变成 `o_auth`，这里显式改名。
    #[serde(rename = "oauth")]
    OAuth,
}

impl AuthTypeResponse {
    pub(crate) fn from_auth(auth: &UpstreamAuth) -> Self {
        match auth {
            UpstreamAuth::None => Self::None,
            UpstreamAuth::Bearer { .. } => Self::Bearer,
            UpstreamAuth::Header { .. } => Self::Header,
            UpstreamAuth::OAuth { .. } => Self::OAuth,
        }
    }
}

/// 裸 JSON object。工具默认参数与调试调用的字段随工具变化，这里不声明固定属性。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct JsonObject(pub serde_json::Map<String, Value>);

impl JsonSchema for JsonObject {
    fn schema_name() -> Cow<'static, str> {
        "JsonObject".into()
    }

    fn schema_id() -> Cow<'static, str> {
        "asterlane::admin::types::JsonObject".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "additionalProperties": true,
            "description": "动态 JSON object，不声明固定业务字段",
            "type": "object"
        })
    }
}
