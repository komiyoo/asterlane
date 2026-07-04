use crate::catalog::ParamLocations;
use openapiv3::{
    OpenAPI, Operation, Parameter, ParameterSchemaOrContent, ReferenceOr, RequestBody, Schema,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const MAX_REF_DEPTH: usize = 10;

const AUTH_HEADERS: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-auth-token",
    "proxy-authorization",
];

#[derive(Debug, thiserror::Error)]
pub enum OpenApiError {
    #[error("failed to parse OpenAPI spec: {0}")]
    ParseError(String),
    #[error("unsupported spec version, expected OpenAPI 3.x")]
    UnsupportedVersion,
    #[error("$ref resolution failed: {0}")]
    RefResolution(String),
}

/// OpenAPI discovery config (matches what will be in ApiResource.discovery).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OpenApiDiscoveryConfig {
    /// Source type.
    #[serde(default)]
    pub source: SpecSource,
    /// File path (when source = file).
    #[serde(default)]
    pub path: Option<String>,
    /// URL (when source = url) -- not fetched in this module, caller provides bytes.
    #[serde(default)]
    pub url: Option<String>,
    /// Only include operations with these OpenAPI tags.
    #[serde(default)]
    pub include_tags: Vec<String>,
    /// Exclude specific operations by "METHOD /path" or operationId.
    #[serde(default)]
    pub exclude_operations: Vec<String>,
    /// HTTP methods to expose by default (empty = all except DELETE).
    #[serde(default)]
    pub default_method_exposure: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpecSource {
    #[default]
    File,
    Url,
}

/// A single discovered endpoint from an OpenAPI spec.
#[derive(Debug, Clone)]
pub struct DiscoveredEndpoint {
    pub tool_segment: String,
    pub method: String,
    pub path: String,
    pub description: String,
    pub input_schema: Value,
    pub param_locations: ParamLocations,
}

/// Parse an OpenAPI spec and extract endpoints, filtered by config.
///
/// `spec_bytes` is the raw YAML or JSON spec content.
/// Returns discovered endpoints that the caller will wrap into WrappedTool.
pub fn discover_endpoints(
    spec_bytes: &[u8],
    config: &OpenApiDiscoveryConfig,
) -> Result<Vec<DiscoveredEndpoint>, OpenApiError> {
    let spec_str =
        std::str::from_utf8(spec_bytes).map_err(|e| OpenApiError::ParseError(e.to_string()))?;

    let spec: OpenAPI = serde_json::from_str(spec_str)
        .map_err(|e| OpenApiError::ParseError(e.to_string()))
        .or_else(|_| {
            serde_norway::from_str(spec_str).map_err(|e| OpenApiError::ParseError(e.to_string()))
        })?;

    if !spec.openapi.starts_with("3.") {
        return Err(OpenApiError::UnsupportedVersion);
    }

    let mut endpoints = Vec::new();
    let mut seen_segments: HashMap<String, usize> = HashMap::new();

    for (path, path_ref) in spec.paths.iter() {
        let path_item = match path_ref {
            ReferenceOr::Item(item) => item,
            ReferenceOr::Reference { .. } => continue,
        };

        let methods: &[(&str, &Option<Operation>)] = &[
            ("get", &path_item.get),
            ("post", &path_item.post),
            ("put", &path_item.put),
            ("patch", &path_item.patch),
            ("delete", &path_item.delete),
        ];

        for &(method, op_opt) in methods {
            let Some(op) = op_opt else { continue };

            if !is_method_allowed(method, config) {
                continue;
            }

            if !config.include_tags.is_empty()
                && !op.tags.iter().any(|t| config.include_tags.contains(t))
            {
                continue;
            }

            let op_key = format!("{} {}", method.to_uppercase(), path);
            if config.exclude_operations.contains(&op_key) {
                continue;
            }
            if let Some(ref id) = op.operation_id {
                if config.exclude_operations.contains(id) {
                    continue;
                }
            }

            let raw_segment = match &op.operation_id {
                Some(id) => normalize_operation_id(id),
                None => fallback_segment(method, path),
            };
            let segment = dedup_segment(&mut seen_segments, raw_segment);

            let (input_schema, param_locations) =
                build_input_schema(op, &path_item.parameters, &spec)?;

            let description = op
                .summary
                .clone()
                .or_else(|| op.description.clone())
                .unwrap_or_default();

            endpoints.push(DiscoveredEndpoint {
                tool_segment: segment,
                method: method.to_string(),
                path: path.clone(),
                description,
                input_schema,
                param_locations,
            });
        }
    }

    Ok(endpoints)
}

fn is_method_allowed(method: &str, config: &OpenApiDiscoveryConfig) -> bool {
    if config.default_method_exposure.is_empty() {
        // ponytail: safety default -- expose all except DELETE
        method != "delete"
    } else {
        config
            .default_method_exposure
            .iter()
            .any(|m| m.eq_ignore_ascii_case(method))
    }
}

fn normalize_operation_id(id: &str) -> String {
    let normalized: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    collapse_underscores(&normalized)
}

fn fallback_segment(method: &str, path: &str) -> String {
    let slug: String = path
        .trim_start_matches('/')
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    collapse_underscores(&format!("{method}_{slug}"))
}

fn collapse_underscores(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev = false;
    for c in s.chars() {
        if c == '_' {
            if !prev {
                out.push('_');
            }
            prev = true;
        } else {
            out.push(c);
            prev = false;
        }
    }
    out.trim_matches('_').to_string()
}

fn dedup_segment(seen: &mut HashMap<String, usize>, segment: String) -> String {
    let count = seen.entry(segment.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        segment
    } else {
        format!("{}_{}", segment, count)
    }
}

// -- input schema construction --

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ParamKind {
    Path,
    Query,
    Header,
}

fn build_input_schema(
    op: &Operation,
    path_level_params: &[ReferenceOr<Parameter>],
    spec: &OpenAPI,
) -> Result<(Value, ParamLocations), OpenApiError> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut locs = ParamLocations {
        path_params: Vec::new(),
        query_params: Vec::new(),
        header_params: Vec::new(),
        has_body: false,
    };
    let mut seen: HashSet<String> = HashSet::new();

    // Operation params override path-level params (processed first)
    for param_ref in op.parameters.iter().chain(path_level_params.iter()) {
        let param = resolve_parameter(param_ref, spec)?;
        let (data, kind) = match param {
            Parameter::Path { parameter_data, .. } => (parameter_data, ParamKind::Path),
            Parameter::Query { parameter_data, .. } => (parameter_data, ParamKind::Query),
            Parameter::Header { parameter_data, .. } => (parameter_data, ParamKind::Header),
            Parameter::Cookie { .. } => continue,
        };

        if !seen.insert(data.name.clone()) {
            continue;
        }

        let schema_val = match &data.format {
            ParameterSchemaOrContent::Schema(s) => resolve_schema_to_value(s, spec, 0)?,
            ParameterSchemaOrContent::Content(_) => serde_json::json!({}),
        };

        match kind {
            ParamKind::Path => {
                properties.insert(data.name.clone(), schema_val);
                required.push(Value::String(data.name.clone()));
                locs.path_params.push(data.name.clone());
            }
            ParamKind::Query => {
                properties.insert(data.name.clone(), schema_val);
                if data.required {
                    required.push(Value::String(data.name.clone()));
                }
                locs.query_params.push(data.name.clone());
            }
            ParamKind::Header => {
                if AUTH_HEADERS.contains(&data.name.to_lowercase().as_str()) {
                    continue;
                }
                let field = format!("_{}", data.name);
                properties.insert(field.clone(), schema_val);
                if data.required {
                    required.push(Value::String(field.clone()));
                }
                locs.header_params.push((field, data.name.clone()));
            }
        }
    }

    if let Some(body_ref) = &op.request_body {
        let body = resolve_request_body(body_ref, spec)?;
        let schema_ref = body
            .content
            .iter()
            .find(|(k, _)| k.contains("json"))
            .or_else(|| body.content.iter().next())
            .and_then(|(_, mt)| mt.schema.as_ref());

        if let Some(s) = schema_ref {
            properties.insert("body".into(), resolve_schema_to_value(s, spec, 0)?);
            if body.required {
                required.push(Value::String("body".into()));
            }
            locs.has_body = true;
        }
    }

    let mut schema = serde_json::json!({"type": "object", "properties": properties});
    if !required.is_empty() {
        schema["required"] = Value::Array(required);
    }

    Ok((schema, locs))
}

// -- $ref resolution --

fn resolve_parameter<'a>(
    param_ref: &'a ReferenceOr<Parameter>,
    spec: &'a OpenAPI,
) -> Result<&'a Parameter, OpenApiError> {
    match param_ref {
        ReferenceOr::Item(param) => Ok(param),
        ReferenceOr::Reference { reference } => {
            let name = ref_name(reference, "parameters")?;
            let components = spec.components.as_ref().ok_or_else(|| {
                OpenApiError::RefResolution(format!("no components for {reference}"))
            })?;
            match components.parameters.get(name) {
                Some(ReferenceOr::Item(param)) => Ok(param),
                Some(ReferenceOr::Reference { reference: inner }) => Err(
                    OpenApiError::RefResolution(format!("nested parameter $ref: {inner}")),
                ),
                None => Err(OpenApiError::RefResolution(format!(
                    "parameter not found: {reference}"
                ))),
            }
        }
    }
}

fn resolve_request_body<'a>(
    body_ref: &'a ReferenceOr<RequestBody>,
    spec: &'a OpenAPI,
) -> Result<&'a RequestBody, OpenApiError> {
    match body_ref {
        ReferenceOr::Item(body) => Ok(body),
        ReferenceOr::Reference { reference } => {
            let name = ref_name(reference, "requestBodies")?;
            let components = spec.components.as_ref().ok_or_else(|| {
                OpenApiError::RefResolution(format!("no components for {reference}"))
            })?;
            match components.request_bodies.get(name) {
                Some(ReferenceOr::Item(body)) => Ok(body),
                Some(ReferenceOr::Reference { reference: inner }) => Err(
                    OpenApiError::RefResolution(format!("nested requestBody $ref: {inner}")),
                ),
                None => Err(OpenApiError::RefResolution(format!(
                    "request body not found: {reference}"
                ))),
            }
        }
    }
}

fn resolve_schema_to_value(
    schema_ref: &ReferenceOr<Schema>,
    spec: &OpenAPI,
    depth: usize,
) -> Result<Value, OpenApiError> {
    if depth > MAX_REF_DEPTH {
        return Ok(serde_json::json!({"type": "object"}));
    }

    match schema_ref {
        ReferenceOr::Reference { reference } => {
            let name = ref_name(reference, "schemas")?;
            let components = spec.components.as_ref().ok_or_else(|| {
                OpenApiError::RefResolution(format!("no components for {reference}"))
            })?;
            let inner = components.schemas.get(name).ok_or_else(|| {
                OpenApiError::RefResolution(format!("schema not found: {reference}"))
            })?;
            resolve_schema_to_value(inner, spec, depth + 1)
        }
        ReferenceOr::Item(schema) => {
            let mut value = serde_json::to_value(schema)
                .map_err(|e| OpenApiError::ParseError(e.to_string()))?;
            inline_schema_refs(&mut value, spec, depth)?;
            Ok(value)
        }
    }
}

/// Walk a serialized JSON Schema value and replace `{"$ref": "#/components/schemas/X"}`
/// with the resolved schema inline.
fn inline_schema_refs(value: &mut Value, spec: &OpenAPI, depth: usize) -> Result<(), OpenApiError> {
    if depth > MAX_REF_DEPTH {
        return Ok(());
    }

    // Extract $ref target (owned String) so the borrow on `value` is released before mutation.
    let ref_target = value
        .as_object()
        .and_then(|m| m.get("$ref"))
        .and_then(|v| v.as_str())
        .and_then(|r| r.strip_prefix("#/components/schemas/"))
        .map(String::from);

    if let Some(name) = ref_target {
        if let Some(schema_ref) = spec
            .components
            .as_ref()
            .and_then(|c| c.schemas.get(name.as_str()))
        {
            *value = resolve_schema_to_value(schema_ref, spec, depth + 1)?;
        }
        return Ok(());
    }

    match value {
        Value::Object(map) => {
            for v in map.values_mut() {
                inline_schema_refs(v, spec, depth)?;
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                inline_schema_refs(v, spec, depth)?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn ref_name<'a>(reference: &'a str, section: &str) -> Result<&'a str, OpenApiError> {
    let prefix = format!("#/components/{section}/");
    reference
        .strip_prefix(&prefix)
        .ok_or_else(|| OpenApiError::RefResolution(format!("unexpected $ref: {reference}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> OpenApiDiscoveryConfig {
        OpenApiDiscoveryConfig::default()
    }

    // -- 1. Parse minimal spec --

    #[test]
    fn parse_minimal_get_endpoint() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {
                "/pets": {
                    "get": {
                        "operationId": "listPets",
                        "summary": "List all pets",
                        "responses": {"200": {"description": "ok"}}
                    }
                }
            }
        }"#;

        let endpoints = discover_endpoints(spec.as_bytes(), &default_config()).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].tool_segment, "listpets");
        assert_eq!(endpoints[0].method, "get");
        assert_eq!(endpoints[0].path, "/pets");
        assert_eq!(endpoints[0].description, "List all pets");
    }

    // -- 2. POST with request body --

    #[test]
    fn post_with_request_body() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {
                "/pets": {
                    "post": {
                        "operationId": "createPet",
                        "summary": "Create a pet",
                        "requestBody": {
                            "required": true,
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "properties": {
                                            "name": {"type": "string"}
                                        }
                                    }
                                }
                            }
                        },
                        "responses": {"201": {"description": "created"}}
                    }
                }
            }
        }"#;

        let endpoints = discover_endpoints(spec.as_bytes(), &default_config()).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert!(endpoints[0].param_locations.has_body);

        let props = endpoints[0].input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("body"));
    }

    // -- 3. Path + query parameters --

    #[test]
    fn path_and_query_params() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {
                "/users/{id}/orders": {
                    "get": {
                        "operationId": "getUserOrders",
                        "parameters": [
                            {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                            {"name": "status", "in": "query", "schema": {"type": "string"}}
                        ],
                        "responses": {"200": {"description": "ok"}}
                    }
                }
            }
        }"#;

        let endpoints = discover_endpoints(spec.as_bytes(), &default_config()).unwrap();
        let ep = &endpoints[0];
        let props = ep.input_schema["properties"].as_object().unwrap();

        assert!(props.contains_key("id"));
        assert!(props.contains_key("status"));
        assert_eq!(ep.param_locations.path_params, vec!["id"]);
        assert_eq!(ep.param_locations.query_params, vec!["status"]);

        let required: Vec<&str> = ep.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"id"));
        assert!(!required.contains(&"status"));
    }

    // -- 4. operationId normalization --

    #[test]
    fn normalizes_operation_id() {
        assert_eq!(normalize_operation_id("Get-User-By-Id"), "get_user_by_id");
        assert_eq!(normalize_operation_id("listPets"), "listpets");
        assert_eq!(normalize_operation_id("__foo__bar__"), "foo_bar");
        assert_eq!(normalize_operation_id("a.b.c"), "a_b_c");
    }

    // -- 5. operationId fallback --

    #[test]
    fn fallback_when_no_operation_id() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {
                "/users/{id}": {
                    "get": {
                        "summary": "Get user",
                        "responses": {"200": {"description": "ok"}}
                    }
                }
            }
        }"#;

        let endpoints = discover_endpoints(spec.as_bytes(), &default_config()).unwrap();
        assert_eq!(endpoints[0].tool_segment, "get_users_id");
    }

    // -- 6. Tag filtering --

    #[test]
    fn filters_by_include_tags() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {
                "/pets": {
                    "get": {
                        "tags": ["pets"],
                        "operationId": "listPets",
                        "responses": {"200": {"description": "ok"}}
                    }
                },
                "/users": {
                    "get": {
                        "tags": ["users"],
                        "operationId": "listUsers",
                        "responses": {"200": {"description": "ok"}}
                    }
                }
            }
        }"#;

        let mut config = default_config();
        config.include_tags = vec!["pets".to_string()];

        let endpoints = discover_endpoints(spec.as_bytes(), &config).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].tool_segment, "listpets");
    }

    // -- 7. Operation exclusion --

    #[test]
    fn excludes_by_method_path_and_operation_id() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {
                "/pets": {
                    "get": {
                        "operationId": "listPets",
                        "responses": {"200": {"description": "ok"}}
                    },
                    "post": {
                        "operationId": "createPet",
                        "responses": {"201": {"description": "created"}}
                    }
                }
            }
        }"#;

        let mut config = default_config();
        config.exclude_operations = vec!["GET /pets".to_string()];

        let endpoints = discover_endpoints(spec.as_bytes(), &config).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].tool_segment, "createpet");

        // Also test exclusion by operationId
        let mut config2 = default_config();
        config2.exclude_operations = vec!["createPet".to_string()];

        let endpoints2 = discover_endpoints(spec.as_bytes(), &config2).unwrap();
        assert_eq!(endpoints2.len(), 1);
        assert_eq!(endpoints2[0].tool_segment, "listpets");
    }

    // -- 8. Default method exposure: DELETE excluded --

    #[test]
    fn delete_excluded_by_default() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {
                "/pets/{id}": {
                    "get": {
                        "operationId": "getPet",
                        "responses": {"200": {"description": "ok"}}
                    },
                    "delete": {
                        "operationId": "deletePet",
                        "responses": {"204": {"description": "deleted"}}
                    }
                }
            }
        }"#;

        let endpoints = discover_endpoints(spec.as_bytes(), &default_config()).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].method, "get");
    }

    #[test]
    fn delete_included_when_explicitly_allowed() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {
                "/pets/{id}": {
                    "delete": {
                        "operationId": "deletePet",
                        "responses": {"204": {"description": "deleted"}}
                    }
                }
            }
        }"#;

        let mut config = default_config();
        config.default_method_exposure = vec!["DELETE".to_string()];

        let endpoints = discover_endpoints(spec.as_bytes(), &config).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].method, "delete");
    }

    // -- 9. Header params: underscore prefix, auth headers skipped --

    #[test]
    fn header_params_get_underscore_prefix_and_auth_skipped() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {
                "/data": {
                    "get": {
                        "operationId": "getData",
                        "parameters": [
                            {"name": "X-Request-Id", "in": "header", "required": true, "schema": {"type": "string"}},
                            {"name": "Authorization", "in": "header", "schema": {"type": "string"}},
                            {"name": "X-Api-Key", "in": "header", "schema": {"type": "string"}}
                        ],
                        "responses": {"200": {"description": "ok"}}
                    }
                }
            }
        }"#;

        let endpoints = discover_endpoints(spec.as_bytes(), &default_config()).unwrap();
        let ep = &endpoints[0];
        let props = ep.input_schema["properties"].as_object().unwrap();

        // X-Request-Id gets underscore prefix
        assert!(props.contains_key("_X-Request-Id"));
        // Auth headers are skipped
        assert!(!props.contains_key("_Authorization"));
        assert!(!props.contains_key("_X-Api-Key"));

        assert_eq!(
            ep.param_locations.header_params,
            vec![("_X-Request-Id".to_string(), "X-Request-Id".to_string())]
        );
    }

    // -- Extra: unsupported version --

    #[test]
    fn rejects_non_3x_version() {
        let spec = r#"{
            "openapi": "2.0.0",
            "info": {"title": "Test", "version": "1.0"},
            "paths": {}
        }"#;

        let err = discover_endpoints(spec.as_bytes(), &default_config()).unwrap_err();
        assert!(matches!(err, OpenApiError::UnsupportedVersion));
    }

    // -- Extra: YAML parsing --

    #[test]
    fn parses_yaml_spec() {
        let spec = r#"
openapi: "3.0.0"
info:
  title: Test
  version: "1.0"
paths:
  /health:
    get:
      operationId: healthCheck
      responses:
        "200":
          description: ok
"#;

        let endpoints = discover_endpoints(spec.as_bytes(), &default_config()).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].tool_segment, "healthcheck");
    }

    // -- Extra: dedup --

    #[test]
    fn dedup_appends_suffix() {
        let mut seen = HashMap::new();
        assert_eq!(dedup_segment(&mut seen, "foo".into()), "foo");
        assert_eq!(dedup_segment(&mut seen, "foo".into()), "foo_2");
        assert_eq!(dedup_segment(&mut seen, "foo".into()), "foo_3");
        assert_eq!(dedup_segment(&mut seen, "bar".into()), "bar");
    }
}
