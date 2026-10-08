//! 给代理看的 schema 投影。
//!
//! `asl__search` 与 `asl__describe` 用这里的签名、描述封顶和压缩 schema。
//! catalog、`tools/list` 和管理面继续持有原始 JSON Schema。

use serde_json::{Map, Value};

/// 结果描述与 schema 内说明的最大 Unicode 标量数。超长时末尾为省略号。
pub(crate) const TEXT_CAP: usize = 200;

/// 取第一句，并限制在 [`TEXT_CAP`] 以内。
///
/// 句界是换行、中文句号，或后面跟着空白和大写/中文的 `.` `!` `?`。
/// `e.g.` 这类后接小写的缩写不断句。
pub(crate) fn cap_text(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let sentence = first_sentence(text);
    let count = sentence.chars().count();
    if count <= TEXT_CAP {
        sentence.to_string()
    } else {
        let truncated: String = sentence.chars().take(TEXT_CAP - 1).collect();
        format!("{truncated}…")
    }
}

/// 顶层参数的紧凑签名。嵌套对象不展开。顺序与 `parameters` 相同。
pub(crate) fn compact_signature(schema: &Value) -> String {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return String::new();
    };
    let required = required_names(schema);
    properties
        .iter()
        .map(|(name, property)| {
            let optional = if required.contains(&name.as_str()) {
                ""
            } else {
                "?"
            };
            format!(
                "{}{optional}: {}",
                display_name(name),
                type_stub(property, 0)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 去掉样板后的可调用 schema。不修改 `schema`。
pub(crate) fn project_schema(schema: &Value) -> Value {
    project_node(schema, None)
}

fn first_sentence(text: &str) -> &str {
    for (byte_idx, ch) in text.char_indices() {
        if ch == '\n' || ch == '\r' {
            return text[..byte_idx].trim_end();
        }
        let end = byte_idx + ch.len_utf8();
        if matches!(ch, '。' | '！' | '？') {
            return &text[..end];
        }
        if matches!(ch, '.' | '!' | '?') && ascii_sentence_end(&text[end..]) {
            return &text[..end];
        }
    }
    text
}

fn ascii_sentence_end(rest: &str) -> bool {
    let mut chars = rest.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    if !first.is_whitespace() {
        return false;
    }
    if first == '\n' || first == '\r' {
        return true;
    }
    for ch in chars {
        if ch == '\n' || ch == '\r' {
            return true;
        }
        if !ch.is_whitespace() {
            return ch.is_uppercase()
                || is_cjk(ch)
                || matches!(ch, '"' | '\'' | '“' | '‘' | '(' | '[');
        }
    }
    true
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{3000}'..='\u{303F}'
    )
}

fn required_names(schema: &Value) -> Vec<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn display_name(name: &str) -> String {
    if is_ident(name) {
        name.to_string()
    } else {
        serde_json::to_string(name).unwrap_or_else(|_| "\"?\"".to_string())
    }
}

fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(ch) if ch == '_' || ch.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn type_stub(schema: &Value, depth: usize) -> String {
    if depth > 8 {
        return "value".to_string();
    }
    let Some(schema) = schema.as_object() else {
        return "value".to_string();
    };
    if let Some(text) = small_enum(schema.get("enum")) {
        return text;
    }
    if let Some(value) = schema.get("const") {
        return json_literal(value);
    }
    if let Some(text) = union_stub(schema, depth) {
        return text;
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return ref_label(reference);
    }
    match schema.get("type") {
        Some(Value::String(ty)) => typed(ty, schema, depth),
        Some(Value::Array(types)) => join_unique(types.iter().filter_map(Value::as_str)),
        _ if schema.contains_key("properties") => "object".to_string(),
        _ => "value".to_string(),
    }
}

fn typed(ty: &str, schema: &Map<String, Value>, depth: usize) -> String {
    match ty {
        "array" => array_stub(schema.get("items"), depth),
        "object" => "object".to_string(),
        "integer" | "number" | "boolean" | "string" | "null" => ty.to_string(),
        other => other.to_string(),
    }
}

fn array_stub(items: Option<&Value>, depth: usize) -> String {
    let Some(items) = items else {
        return "array".to_string();
    };
    let inner = type_stub(items, depth + 1);
    if inner.contains([' ', '|']) {
        format!("({inner})[]")
    } else {
        format!("{inner}[]")
    }
}

fn union_stub(schema: &Map<String, Value>, depth: usize) -> Option<String> {
    let branches = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))?
        .as_array()?;
    if branches.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for branch in branches {
        let stub = type_stub(branch, depth + 1);
        if !parts.contains(&stub) {
            parts.push(stub);
        }
    }
    Some(parts.join(" | "))
}

fn small_enum(enum_value: Option<&Value>) -> Option<String> {
    let values = enum_value?.as_array()?;
    if values.is_empty() || values.len() > 6 {
        return None;
    }
    if !values.iter().all(is_scalar) {
        return None;
    }
    let rendered = values
        .iter()
        .map(json_literal)
        .collect::<Vec<_>>()
        .join(" | ");
    (rendered.chars().count() <= 80).then_some(rendered)
}

fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
    )
}

fn json_literal(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "value".to_string())
}

fn ref_label(reference: &str) -> String {
    reference
        .rsplit(['/', '#'])
        .find(|part| !part.is_empty())
        .unwrap_or(reference)
        .to_string()
}

fn join_unique<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let mut parts = Vec::new();
    for item in items {
        if !parts.contains(&item) {
            parts.push(item);
        }
    }
    if parts.is_empty() {
        "value".to_string()
    } else {
        parts.join(" | ")
    }
}

fn project_node(value: &Value, name: Option<&str>) -> Value {
    let Some(map) = value.as_object() else {
        return value.clone();
    };
    let mut out = Map::new();
    for (key, child) in map {
        if let Some(projected) = project_entry(key, child, name) {
            out.insert(key.clone(), projected);
        }
    }
    Value::Object(out)
}

fn project_entry(key: &str, value: &Value, name: Option<&str>) -> Option<Value> {
    if strip_entry(key, value, name) {
        return None;
    }
    Some(match key {
        "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => {
            project_named(value)
        }
        "items" => project_items(value),
        "additionalProperties"
        | "unevaluatedItems"
        | "unevaluatedProperties"
        | "contains"
        | "propertyNames"
        | "not"
        | "if"
        | "then"
        | "else" => project_node(value, None),
        "oneOf" | "anyOf" | "allOf" | "prefixItems" => project_schema_list(value),
        "description" => match value.as_str() {
            Some(text) => Value::String(cap_text(text)),
            None => value.clone(),
        },
        _ => value.clone(),
    })
}

fn strip_entry(key: &str, value: &Value, name: Option<&str>) -> bool {
    match key {
        "$schema" | "$id" | "example" | "examples" => true,
        "title" => name.is_some_and(|field| value.as_str() == Some(field)),
        "description" => value.as_str().is_some_and(|text| text.trim().is_empty()),
        "additionalProperties" => value.as_bool() == Some(true),
        _ => false,
    }
}

fn project_named(value: &Value) -> Value {
    let Some(map) = value.as_object() else {
        return value.clone();
    };
    let mut out = Map::new();
    for (key, child) in map {
        out.insert(key.clone(), project_node(child, Some(key)));
    }
    Value::Object(out)
}

fn project_items(value: &Value) -> Value {
    match value {
        Value::Array(_) => project_schema_list(value),
        other => project_node(other, None),
    }
}

fn project_schema_list(value: &Value) -> Value {
    let Some(items) = value.as_array() else {
        return value.clone();
    };
    Value::Array(items.iter().map(|item| project_node(item, None)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cap_text_keeps_the_first_sentence_and_limits_length() {
        assert_eq!(cap_text("Search the web. More detail."), "Search the web.");
        assert_eq!(cap_text("搜索网页。更多说明。"), "搜索网页。");
        assert_eq!(
            cap_text("Use e.g. this filter. Next."),
            "Use e.g. this filter."
        );
        assert_eq!(cap_text("v1.2 is current."), "v1.2 is current.");
        assert_eq!(cap_text("First line\nsecond line"), "First line");

        let capped = cap_text(&"a".repeat(201));
        assert_eq!(capped.chars().count(), TEXT_CAP);
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn compact_signature_stubs_nested_values_and_marks_optional_params() {
        assert_eq!(compact_signature(&json!({"type": "object"})), "");
        let signature = compact_signature(&json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer"},
                "body": {"type": "object", "properties": {"n": {"type": "integer"}}},
                "tags": {"type": "array", "items": {"type": "string"}},
                "flags": {"type": "array", "items": {"enum": ["a", "b"]}},
                "mode": {"type": "string", "enum": ["fast", "slow"]},
                "note": {"oneOf": [{"type": "string"}, {"type": "null"}]},
                "payload": {"oneOf": [{"type": "object"}, {"type": "object", "properties": {"a": {"type": "string"}}}]},
                "filter": {"$ref": "#/$defs/Filter"},
                "status": {"type": "string", "enum": ["v0", "v1", "v2", "v3", "v4", "v5", "v6"]}
            },
            "required": ["query", "mode"]
        }));
        assert_eq!(
            signature,
            "body?: object, filter?: Filter, flags?: (\"a\" | \"b\")[], limit?: integer, mode: \"fast\" | \"slow\", note?: string | null, payload?: object, query: string, status?: string, tags?: string[]"
        );
    }

    #[test]
    fn project_schema_drops_boilerplate_and_keeps_constraints() {
        let raw = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.test/schema",
            "title": "Input",
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "query": {
                    "type": "string",
                    "title": "query",
                    "description": "The query. Extra detail.",
                    "format": "date-time",
                    "pattern": "^q",
                    "minLength": 1,
                    "examples": ["rust"],
                    "example": "rust"
                },
                "limit": {
                    "type": "integer",
                    "title": "Limit",
                    "description": "   ",
                    "minimum": 1,
                    "maximum": 10,
                    "default": 5
                },
                "mode": {
                    "enum": ["a", "b"],
                    "description": ""
                }
            },
            "required": ["query"],
            "$defs": {
                "Mode": {
                    "title": "Mode",
                    "type": "string",
                    "enum": ["a", "b"]
                }
            }
        });
        let before = raw.clone();
        assert_eq!(
            project_schema(&raw),
            json!({
                "title": "Input",
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The query.",
                        "format": "date-time",
                        "pattern": "^q",
                        "minLength": 1
                    },
                    "limit": {
                        "type": "integer",
                        "title": "Limit",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 5
                    },
                    "mode": {
                        "enum": ["a", "b"]
                    }
                },
                "required": ["query"],
                "$defs": {
                    "Mode": {
                        "type": "string",
                        "enum": ["a", "b"]
                    }
                }
            })
        );
        assert_eq!(raw, before);
    }

    #[test]
    fn project_schema_keeps_closed_objects_unions_and_refs() {
        let raw = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "choice": {
                    "oneOf": [
                        {"type": "string", "examples": ["a"]},
                        {"$ref": "#/$defs/Box"}
                    ]
                }
            },
            "$defs": {
                "Box": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {"n": {"type": "integer"}}
                }
            }
        });
        let projected = project_schema(&raw);
        assert!(
            projected["properties"]["choice"]["oneOf"][0]
                .get("examples")
                .is_none()
        );
        assert_eq!(
            projected["properties"]["choice"]["oneOf"][1]["$ref"],
            "#/$defs/Box"
        );
        assert_eq!(projected["additionalProperties"], false);
        assert_eq!(projected["$defs"]["Box"]["additionalProperties"], false);
    }
}
