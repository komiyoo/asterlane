//! 从管理 DTO 导出 Draft 7 JSON Schema。
//!
//! `inputs` 使用反序列化契约，`outputs` 使用序列化契约。生成文件由
//! `just admin-schema` 写入，`just admin-schema-check` 只比较。

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

use crate::error::HttpErrorEnvelope;

use super::types::{
    ConfigValidateResponse, CreatedResponse, DeletedResponse, EventsListParams, HealthResponse,
    JsonObject, KeyPoolResponse, McpHealthResponse, McpPresetResponse, McpServerDetailResponse,
    McpServerResponse, McpServerWriteParams, ProxyKeyResponse, ProxyKeyWriteParams,
    RequestEventResponse, ResourceSummaryResponse, ResourceWriteParams, SecurityEventResponse,
    SecurityEventsListParams, StatsResponse, TokenIssueParams, TokenIssueResponse,
    ToolCatalogResponse, ToolDefaultResponse, ToolInvokeParams, ToolInvokeResponse,
    ToolMetadataResponse, ToolMetadataWriteParams, UpdatedResponse, UsageListParams, UsageResponse,
};

const GENERATED_COMMENT: &str = "由 `just admin-schema` 从 Rust 管理 DTO 生成（schemars JSON Schema Draft 7）。不要手改。inputs 按反序列化，outputs 按序列化。检查失败时运行 `just admin-schema`。";

/// 反序列化输入。字段不是一次请求，而是各端点的请求或查询类型。
#[derive(JsonSchema)]
#[allow(dead_code)]
struct AdminInputs {
    /// `POST/PUT /admin/resources`
    resource_write_params: ResourceWriteParams,
    /// `POST/PUT /admin/proxy-keys`
    proxy_key_write_params: ProxyKeyWriteParams,
    /// `POST/PUT /admin/mcp-servers`。security 为嵌套 `defense.enabled`。
    mcp_server_write_params: McpServerWriteParams,
    /// `POST /admin/proxy-keys/{id}/token` 的对象 body。空 body 与省略 expires_at 等价。
    token_issue_params: TokenIssueParams,
    /// `PUT /admin/tools/{name}/metadata`
    tool_metadata_write_params: ToolMetadataWriteParams,
    /// `PUT /admin/tools/{name}/defaults` 的裸 JSON object。
    tool_default_body: JsonObject,
    /// `POST /admin/tools/{name}/invoke` 的裸 JSON object。
    tool_invoke_body: JsonObject,
    /// `GET /admin/events`
    events_list_params: EventsListParams,
    /// `GET /admin/security-events`
    security_events_list_params: SecurityEventsListParams,
    /// `GET /admin/usage`
    usage_list_params: UsageListParams,
    /// `POST /admin/tools/{name}/invoke`
    tool_invoke_params: ToolInvokeParams,
}

/// 序列化输出。数组端点的类型本身就是数组，不另包一层。
#[derive(JsonSchema)]
#[allow(dead_code)]
struct AdminOutputs {
    /// `GET /admin/health`
    health: HealthResponse,
    /// `GET /admin/stats`
    stats: StatsResponse,
    /// `GET /admin/usage`
    usage: UsageResponse,
    /// `GET /admin/resources`
    resources: Vec<ResourceSummaryResponse>,
    /// 创建成功的 JSON body。
    created: CreatedResponse,
    /// 更新成功的 JSON body。
    updated: UpdatedResponse,
    /// 删除成功且返回 JSON 时的 body。MCP server 与 token 吊销是 204，无 body。
    deleted: DeletedResponse,
    /// `GET /admin/proxy-keys`
    proxy_keys: Vec<ProxyKeyResponse>,
    /// `POST /admin/proxy-keys/{id}/token`。明文只出现在这个响应。
    token_issue: TokenIssueResponse,
    /// `GET /admin/key-pools`
    key_pools: Vec<KeyPoolResponse>,
    /// `GET /admin/events`
    events: Vec<RequestEventResponse>,
    /// `GET /admin/security-events`，审计页使用 `kind=admin_audit`。
    security_events: Vec<SecurityEventResponse>,
    /// `GET /admin/tools`
    tools: ToolCatalogResponse,
    /// `GET /admin/tool-defaults`
    tool_defaults: Vec<ToolDefaultResponse>,
    /// `GET /admin/tools/{name}/defaults`
    tool_default: ToolDefaultResponse,
    /// `GET /admin/tool-metadata`
    tool_metadata_list: Vec<ToolMetadataResponse>,
    /// `GET /admin/tools/{name}/metadata`
    tool_metadata: ToolMetadataResponse,
    /// `POST /admin/tools/{name}/invoke`
    tool_invoke: ToolInvokeResponse,
    /// `GET/POST/PUT /admin/mcp-servers`。security 为扁平 `defense_enabled`。
    mcp_servers: Vec<McpServerResponse>,
    /// `GET /admin/mcp-servers/{id}`
    mcp_server: McpServerDetailResponse,
    /// `POST /admin/mcp-servers/{id}/probe`
    mcp_health: McpHealthResponse,
    /// `GET /admin/mcp-presets`
    mcp_presets: Vec<McpPresetResponse>,
    /// `GET /admin/config/validate`。`GET /admin/config/export` 是 text/yaml，不是此对象。
    config_validate: ConfigValidateResponse,
    /// 失败响应 `{error:{code,message,request_id}}`。
    http_error: HttpErrorEnvelope,
}

/// 稳定的 pretty JSON，末尾带换行。
pub fn render_admin_schema() -> Result<String, serde_json::Error> {
    let mut text = serde_json::to_string_pretty(&admin_schema_value()?)?;
    text.push('\n');
    Ok(text)
}

fn admin_schema_value() -> Result<Value, serde_json::Error> {
    let inputs = generate::<AdminInputs>(SchemaSettings::draft07().for_deserialize())?;
    let outputs = generate::<AdminOutputs>(SchemaSettings::draft07().for_serialize())?;
    let (inputs, mut definitions) = namespace(inputs, "Input");
    let (outputs, output_defs) = namespace(outputs, "Output");
    definitions.extend(output_defs);

    let mut properties = Map::new();
    properties.insert("inputs".to_string(), inputs);
    properties.insert("outputs".to_string(), outputs);

    let mut root = Map::new();
    root.insert(
        "$comment".to_string(),
        Value::String(GENERATED_COMMENT.to_string()),
    );
    root.insert(
        "$schema".to_string(),
        Value::String("http://json-schema.org/draft-07/schema#".to_string()),
    );
    root.insert(
        "description".to_string(),
        Value::String(
            "管理 HTTP 契约。动态工具参数、调用结果、事件 details 与 input_schema 保持 JSON 值，不声明固定业务字段。"
                .to_string(),
        ),
    );
    root.insert(
        "title".to_string(),
        Value::String("AsterlaneAdminApi".to_string()),
    );
    root.insert("type".to_string(), Value::String("object".to_string()));
    root.insert("definitions".to_string(), Value::Object(definitions));
    root.insert("properties".to_string(), Value::Object(properties));
    Ok(Value::Object(root))
}

fn generate<T: JsonSchema + ?Sized>(settings: SchemaSettings) -> Result<Value, serde_json::Error> {
    let schema = settings.into_generator().into_root_schema_for::<T>();
    serde_json::to_value(&schema)
}

fn namespace(mut schema: Value, prefix: &str) -> (Value, Map<String, Value>) {
    rewrite_refs(&mut schema, prefix);
    let definitions = match schema
        .as_object_mut()
        .and_then(|obj| obj.remove("definitions"))
    {
        Some(Value::Object(definitions)) => definitions,
        _ => Map::new(),
    };
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$schema");
    }
    let mut renamed = Map::new();
    for (name, definition) in definitions {
        renamed.insert(format!("{prefix}{name}"), definition);
    }
    (schema, renamed)
}

fn rewrite_refs(value: &mut Value, prefix: &str) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("$ref").cloned()
                && let Some(name) = reference.strip_prefix("#/definitions/")
                && !name.starts_with(prefix)
            {
                map.insert(
                    "$ref".to_string(),
                    Value::String(format!("#/definitions/{prefix}{name}")),
                );
            }
            for child in map.values_mut() {
                rewrite_refs(child, prefix);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_refs(item, prefix);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_stable_and_splits_contracts() {
        let once = render_admin_schema().expect("schema");
        let twice = render_admin_schema().expect("schema");
        assert_eq!(once, twice);
        assert!(once.ends_with('\n'));

        let doc: Value = serde_json::from_str(&once).expect("json");
        assert_eq!(doc["$schema"], "http://json-schema.org/draft-07/schema#");
        assert!(
            doc["$comment"]
                .as_str()
                .is_some_and(|text| text.contains("just admin-schema"))
        );
        let defs = doc["definitions"].as_object().expect("definitions");

        let issue = defs
            .get("OutputTokenIssueResponse")
            .expect("issue response");
        assert!(issue["properties"].get("token").is_some());
        for (name, schema) in defs {
            if name.starts_with("Output") && name != "OutputTokenIssueResponse" {
                assert!(
                    !has_property(schema, "token"),
                    "{name} must not expose token"
                );
            }
            if name.starts_with("Output") {
                assert!(!has_property(schema, "token_digest"), "{name}");
                assert!(!has_property(schema, "token_ref"), "{name}");
            }
        }

        let flat = defs
            .get("OutputMcpSecurityResponse")
            .expect("read security")
            .to_string();
        assert!(flat.contains("defense_enabled"));
        assert!(!flat.contains("\"defense\""));

        let nested = defs
            .get("InputSecurityConfig")
            .or_else(|| defs.get("InputMcpServerWriteParams"))
            .expect("write security")
            .to_string();
        assert!(nested.contains("defense") || doc.to_string().contains("\"defense\""));

        let text = doc.to_string();
        assert!(
            text.contains("\"null\"")
                || text.contains("[\"string\",\"null\"]")
                || text.contains("\"date-time\"")
        );
        assert!(text.contains("date-time"));
        assert!(text.contains("\"enum\""));
        assert!(text.contains("\"default\""));
        assert!(text.contains("\"integer\"") || text.contains("\"number\""));
        assert!(!text.contains("token_digest"));
        assert!(!text.contains("alk_"));
    }

    fn has_property(value: &Value, name: &str) -> bool {
        match value {
            Value::Object(map) => {
                if map
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|props| props.contains_key(name))
                {
                    return true;
                }
                map.values().any(|child| has_property(child, name))
            }
            Value::Array(items) => items.iter().any(|item| has_property(item, name)),
            _ => false,
        }
    }
}
