//! Declarative request transformation (header/body rules with conditions).

use crate::error::{AsterlaneError, ErrorCode};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

const DANGEROUS_HEADERS: &[&str] = &["authorization", "host", "cookie", "proxy-authorization"];

#[derive(Debug, Error)]
pub enum TransformError {
    #[error("transform blocked: header `{0}` is dangerous (set allow_dangerous to override)")]
    DangerousHeader(String),
    #[error("transform failed: invalid JSON pointer `{0}`")]
    InvalidPointer(String),
    #[error("transform failed: invalid header value")]
    InvalidHeaderValue,
}

impl From<TransformError> for AsterlaneError {
    fn from(err: TransformError) -> Self {
        let code = match &err {
            TransformError::DangerousHeader(_) => ErrorCode::TransformDangerousHeader,
            TransformError::InvalidPointer(_) => ErrorCode::TransformInvalidPointer,
            TransformError::InvalidHeaderValue => ErrorCode::TransformInvalidPointer,
        };
        AsterlaneError::internal(code, err.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOp {
    Eq,
    NotEq,
    Contains,
    Exists,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransformCondition {
    #[default]
    Always,
    IfHeader {
        name: String,
        op: ConditionOp,
        #[serde(default)]
        value: String,
    },
    IfBody {
        pointer: String,
        op: ConditionOp,
        #[serde(default)]
        value: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TransformRule {
    SetHeader {
        name: String,
        value_template: String,
        #[serde(default)]
        allow_dangerous: bool,
    },
    RemoveHeader {
        name: String,
    },
    SetJsonBody {
        pointer: String,
        value: serde_json::Value,
    },
    RemoveJsonBody {
        pointer: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConditionalRule {
    #[serde(default)]
    pub condition: TransformCondition,
    #[serde(flatten)]
    pub rule: TransformRule,
}

pub type TransformConfig = Vec<ConditionalRule>;

#[derive(Debug)]
pub struct TransformContext {
    pub vars: HashMap<String, String>,
}

impl TransformContext {
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
        }
    }

    fn render_template(&self, template: &str) -> String {
        let mut result = template.to_string();
        for (key, val) in &self.vars {
            let placeholder = format!("${{{{{key}}}}}");
            result = result.replace(&placeholder, val);
        }
        result
    }
}

impl Default for TransformContext {
    fn default() -> Self {
        Self::new()
    }
}

pub fn apply_transforms(
    headers: &mut HeaderMap,
    body: &mut Option<serde_json::Value>,
    rules: &[ConditionalRule],
    ctx: &TransformContext,
) -> Result<(), TransformError> {
    for entry in rules {
        if !eval_condition(&entry.condition, headers, body) {
            continue;
        }
        apply_rule(&entry.rule, headers, body, ctx)?;
    }
    Ok(())
}

fn eval_condition(
    cond: &TransformCondition,
    headers: &HeaderMap,
    body: &Option<serde_json::Value>,
) -> bool {
    match cond {
        TransformCondition::Always => true,
        TransformCondition::IfHeader { name, op, value } => {
            let header_val = headers.get(name.as_str()).and_then(|v| v.to_str().ok());
            match op {
                ConditionOp::Exists => header_val.is_some(),
                ConditionOp::Eq => header_val == Some(value.as_str()),
                ConditionOp::NotEq => header_val != Some(value.as_str()),
                ConditionOp::Contains => header_val.is_some_and(|v| v.contains(value.as_str())),
            }
        }
        TransformCondition::IfBody { pointer, op, value } => {
            let Some(b) = body else { return false };
            let found = b.pointer(pointer);
            match op {
                ConditionOp::Exists => found.is_some(),
                ConditionOp::Eq => found == Some(value),
                ConditionOp::NotEq => found != Some(value),
                ConditionOp::Contains => match (found, value) {
                    (Some(serde_json::Value::String(s)), serde_json::Value::String(needle)) => {
                        s.contains(needle.as_str())
                    }
                    _ => false,
                },
            }
        }
    }
}

fn apply_rule(
    rule: &TransformRule,
    headers: &mut HeaderMap,
    body: &mut Option<serde_json::Value>,
    ctx: &TransformContext,
) -> Result<(), TransformError> {
    match rule {
        TransformRule::SetHeader {
            name,
            value_template,
            allow_dangerous,
        } => {
            if !allow_dangerous && DANGEROUS_HEADERS.contains(&name.to_lowercase().as_str()) {
                return Err(TransformError::DangerousHeader(name.clone()));
            }
            let rendered = ctx.render_template(value_template);
            let header_name: HeaderName = name
                .parse()
                .map_err(|_| TransformError::DangerousHeader(name.clone()))?;
            let header_value =
                HeaderValue::from_str(&rendered).map_err(|_| TransformError::InvalidHeaderValue)?;
            headers.insert(header_name, header_value);
            Ok(())
        }
        TransformRule::RemoveHeader { name } => {
            if let Ok(header_name) = name.parse::<HeaderName>() {
                headers.remove(header_name);
            }
            Ok(())
        }
        TransformRule::SetJsonBody { pointer, value } => {
            if !pointer.starts_with('/') {
                return Err(TransformError::InvalidPointer(pointer.clone()));
            }
            let b = body.get_or_insert(serde_json::Value::Object(serde_json::Map::new()));
            set_at_pointer(b, pointer, value.clone())?;
            Ok(())
        }
        TransformRule::RemoveJsonBody { pointer } => {
            if !pointer.starts_with('/') {
                return Err(TransformError::InvalidPointer(pointer.clone()));
            }
            if let Some(b) = body {
                remove_at_pointer(b, pointer);
            }
            Ok(())
        }
    }
}

// ponytail: naive pointer walk, covers flat and one-level nesting; deep arbitrary nesting via recursive split
fn set_at_pointer(
    root: &mut serde_json::Value,
    pointer: &str,
    value: serde_json::Value,
) -> Result<(), TransformError> {
    let parts: Vec<&str> = pointer[1..].split('/').collect();
    let mut current = root;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if let Some(obj) = current.as_object_mut() {
                obj.insert((*part).to_string(), value);
                return Ok(());
            }
            return Err(TransformError::InvalidPointer(pointer.to_string()));
        }
        if !current.as_object().is_some_and(|o| o.contains_key(*part)) {
            if let Some(obj) = current.as_object_mut() {
                obj.insert(
                    (*part).to_string(),
                    serde_json::Value::Object(serde_json::Map::new()),
                );
            }
        }
        current = current
            .pointer_mut(&format!("/{part}"))
            .ok_or_else(|| TransformError::InvalidPointer(pointer.to_string()))?;
    }
    Ok(())
}

fn remove_at_pointer(root: &mut serde_json::Value, pointer: &str) {
    let parts: Vec<&str> = pointer[1..].split('/').collect();
    if parts.is_empty() {
        return;
    }
    if parts.len() == 1 {
        if let Some(obj) = root.as_object_mut() {
            obj.remove(parts[0]);
        }
        return;
    }
    let parent_pointer = format!("/{}", parts[..parts.len() - 1].join("/"));
    if let Some(parent) = root.pointer_mut(&parent_pointer) {
        if let Some(obj) = parent.as_object_mut() {
            obj.remove(parts[parts.len() - 1]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx_with(vars: &[(&str, &str)]) -> TransformContext {
        let mut ctx = TransformContext::new();
        for (k, v) in vars {
            ctx.vars.insert((*k).to_string(), (*v).to_string());
        }
        ctx
    }

    #[test]
    fn set_header_with_template() {
        let mut headers = HeaderMap::new();
        let mut body = None;
        let rules = vec![ConditionalRule {
            condition: TransformCondition::Always,
            rule: TransformRule::SetHeader {
                name: "x-custom".to_string(),
                value_template: "Bearer ${{token}}".to_string(),
                allow_dangerous: false,
            },
        }];
        let ctx = ctx_with(&[("token", "abc123")]);
        apply_transforms(&mut headers, &mut body, &rules, &ctx).unwrap();
        assert_eq!(
            headers.get("x-custom").unwrap().to_str().unwrap(),
            "Bearer abc123"
        );
    }

    #[test]
    fn remove_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-remove-me", HeaderValue::from_static("val"));
        let mut body = None;
        let rules = vec![ConditionalRule {
            condition: TransformCondition::Always,
            rule: TransformRule::RemoveHeader {
                name: "x-remove-me".to_string(),
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert!(headers.get("x-remove-me").is_none());
    }

    #[test]
    fn set_json_body_at_pointer() {
        let mut headers = HeaderMap::new();
        let mut body = Some(json!({"a": 1}));
        let rules = vec![ConditionalRule {
            condition: TransformCondition::Always,
            rule: TransformRule::SetJsonBody {
                pointer: "/b".to_string(),
                value: json!(2),
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert_eq!(body.unwrap(), json!({"a": 1, "b": 2}));
    }

    #[test]
    fn set_json_body_nested() {
        let mut headers = HeaderMap::new();
        let mut body = Some(json!({}));
        let rules = vec![ConditionalRule {
            condition: TransformCondition::Always,
            rule: TransformRule::SetJsonBody {
                pointer: "/deep/key".to_string(),
                value: json!("val"),
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert_eq!(body.unwrap(), json!({"deep": {"key": "val"}}));
    }

    #[test]
    fn remove_json_body_at_pointer() {
        let mut headers = HeaderMap::new();
        let mut body = Some(json!({"a": 1, "b": 2}));
        let rules = vec![ConditionalRule {
            condition: TransformCondition::Always,
            rule: TransformRule::RemoveJsonBody {
                pointer: "/b".to_string(),
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert_eq!(body.unwrap(), json!({"a": 1}));
    }

    #[test]
    fn dangerous_header_blocked() {
        let mut headers = HeaderMap::new();
        let mut body = None;
        let rules = vec![ConditionalRule {
            condition: TransformCondition::Always,
            rule: TransformRule::SetHeader {
                name: "Authorization".to_string(),
                value_template: "Bearer x".to_string(),
                allow_dangerous: false,
            },
        }];
        let err = apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new())
            .unwrap_err();
        assert!(matches!(err, TransformError::DangerousHeader(_)));
    }

    #[test]
    fn dangerous_header_allowed_with_flag() {
        let mut headers = HeaderMap::new();
        let mut body = None;
        let rules = vec![ConditionalRule {
            condition: TransformCondition::Always,
            rule: TransformRule::SetHeader {
                name: "Authorization".to_string(),
                value_template: "Bearer x".to_string(),
                allow_dangerous: true,
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert_eq!(
            headers.get("authorization").unwrap().to_str().unwrap(),
            "Bearer x"
        );
    }

    #[test]
    fn condition_if_header_eq() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        let mut body = Some(json!({}));
        let rules = vec![ConditionalRule {
            condition: TransformCondition::IfHeader {
                name: "content-type".to_string(),
                op: ConditionOp::Eq,
                value: "application/json".to_string(),
            },
            rule: TransformRule::SetJsonBody {
                pointer: "/injected".to_string(),
                value: json!(true),
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert_eq!(body.unwrap().pointer("/injected"), Some(&json!(true)));
    }

    #[test]
    fn condition_if_header_not_matched_skips() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("text/plain"));
        let mut body = Some(json!({}));
        let rules = vec![ConditionalRule {
            condition: TransformCondition::IfHeader {
                name: "content-type".to_string(),
                op: ConditionOp::Eq,
                value: "application/json".to_string(),
            },
            rule: TransformRule::SetJsonBody {
                pointer: "/injected".to_string(),
                value: json!(true),
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert_eq!(body.unwrap().pointer("/injected"), None);
    }

    #[test]
    fn condition_if_header_contains() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "accept",
            HeaderValue::from_static("text/html, application/json"),
        );
        let mut body = None;
        let rules = vec![ConditionalRule {
            condition: TransformCondition::IfHeader {
                name: "accept".to_string(),
                op: ConditionOp::Contains,
                value: "json".to_string(),
            },
            rule: TransformRule::SetHeader {
                name: "x-json-detected".to_string(),
                value_template: "true".to_string(),
                allow_dangerous: false,
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert_eq!(
            headers.get("x-json-detected").unwrap().to_str().unwrap(),
            "true"
        );
    }

    #[test]
    fn condition_if_header_exists() {
        let mut headers = HeaderMap::new();
        headers.insert("x-trace-id", HeaderValue::from_static("abc"));
        let mut body = None;
        let rules = vec![ConditionalRule {
            condition: TransformCondition::IfHeader {
                name: "x-trace-id".to_string(),
                op: ConditionOp::Exists,
                value: String::new(),
            },
            rule: TransformRule::SetHeader {
                name: "x-traced".to_string(),
                value_template: "yes".to_string(),
                allow_dangerous: false,
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert!(headers.get("x-traced").is_some());
    }

    #[test]
    fn condition_if_body_eq() {
        let mut headers = HeaderMap::new();
        let mut body = Some(json!({"model": "gpt-4"}));
        let rules = vec![ConditionalRule {
            condition: TransformCondition::IfBody {
                pointer: "/model".to_string(),
                op: ConditionOp::Eq,
                value: json!("gpt-4"),
            },
            rule: TransformRule::SetHeader {
                name: "x-model".to_string(),
                value_template: "gpt-4".to_string(),
                allow_dangerous: false,
            },
        }];
        apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new()).unwrap();
        assert!(headers.get("x-model").is_some());
    }

    #[test]
    fn invalid_pointer_rejected() {
        let mut headers = HeaderMap::new();
        let mut body = Some(json!({}));
        let rules = vec![ConditionalRule {
            condition: TransformCondition::Always,
            rule: TransformRule::SetJsonBody {
                pointer: "no-leading-slash".to_string(),
                value: json!(1),
            },
        }];
        let err = apply_transforms(&mut headers, &mut body, &rules, &TransformContext::new())
            .unwrap_err();
        assert!(matches!(err, TransformError::InvalidPointer(_)));
    }

    #[test]
    fn transform_error_converts_to_asterlane_error() {
        let err = TransformError::DangerousHeader("Authorization".to_string());
        let ae: AsterlaneError = err.into();
        assert_eq!(ae.error_code(), ErrorCode::TransformDangerousHeader);
        assert_eq!(ae.exit_code(), 8);
    }

    #[test]
    fn serde_roundtrip() {
        let rule = ConditionalRule {
            condition: TransformCondition::IfHeader {
                name: "content-type".to_string(),
                op: ConditionOp::Contains,
                value: "json".to_string(),
            },
            rule: TransformRule::SetHeader {
                name: "x-format".to_string(),
                value_template: "json".to_string(),
                allow_dangerous: false,
            },
        };
        let yaml = serde_json::to_string(&rule).unwrap();
        let back: ConditionalRule = serde_json::from_str(&yaml).unwrap();
        assert_eq!(rule, back);
    }
}
