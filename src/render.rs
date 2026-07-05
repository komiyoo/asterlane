//! Response Rendering（结果再呈现）：把上游 JSON 结果重呈现为 agent 友好格式。
//!
//! 设计见 `docs/response-rendering.md`。表示层纯函数，不增删语义信息：
//! - `json`：透传（现状行为）
//! - `yaml`：`serde_norway` 1:1 重序列化，无损
//! - `markdown`：确定性投影，面向 LLM 阅读，有损
//!
//! 管线位置：defense 扫描之后、shaping 截断之前（`proxy::executor`）。

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

use crate::error::{AsterlaneError, ErrorCode};

/// 目标输出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    /// 原样透传（默认，等价于不启用 rendering）。
    #[default]
    Json,
    /// JSON 值 1:1 重序列化为 YAML，无损。
    Yaml,
    /// 面向 LLM/人类阅读的确定性 markdown 投影，有损。
    Markdown,
}

impl ResponseFormat {
    /// 格式的规范字符串（与配置/请求参数值一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Markdown => "markdown",
        }
    }

    /// 渲染后响应体的 `Content-Type`。
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Yaml => "application/yaml",
            Self::Markdown => "text/markdown; charset=utf-8",
        }
    }
}

impl fmt::Display for ResponseFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ResponseFormat {
    type Err = RenderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "yaml" => Ok(Self::Yaml),
            "markdown" => Ok(Self::Markdown),
            other => Err(RenderError::UnknownFormat(other.to_string())),
        }
    }
}

/// 格式解析/协商错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    /// 请求级 override 给出未知格式值（fail fast，不静默降级）。
    #[error("unknown response format `{0}` (expected json | yaml | markdown)")]
    UnknownFormat(String),
}

impl From<RenderError> for AsterlaneError {
    fn from(err: RenderError) -> Self {
        AsterlaneError::internal(ErrorCode::McpInvalidToolCall, err.to_string())
    }
}

/// 从请求上下文解析格式：request override > key config > global default > `json`。
///
/// override 为未知值时报错（fail fast）；配置级值经 serde 校验，此处直接信任。
pub fn resolve_format(
    request_override: Option<&str>,
    key_format: Option<ResponseFormat>,
    global_default: Option<ResponseFormat>,
) -> Result<ResponseFormat, RenderError> {
    if let Some(raw) = request_override {
        return raw.parse();
    }
    Ok(key_format.or(global_default).unwrap_or_default())
}

/// 从 HTTP `Accept` header 推导格式覆盖值。
///
/// 返回规范格式字符串，供 `resolve_format` 消费；无可识别媒体类型时返回 None
/// （回退渠道/全局配置）。
// ponytail: contains 匹配，不做 q-value 协商；需要完整内容协商时换 mediatype crate
pub fn format_from_accept(accept: &str) -> Option<&'static str> {
    if accept.contains("application/yaml") || accept.contains("text/yaml") {
        Some("yaml")
    } else if accept.contains("text/markdown") {
        Some("markdown")
    } else if accept.contains("application/json") {
        Some("json")
    } else {
        None
    }
}

/// 将 JSON 值渲染为目标格式文本。
///
/// `Json` 返回 None（透传语义，调用方保留原文）；`Yaml` 序列化失败时
/// 记 warn 日志并返回 None（表示层失败不应让成功调用变成失败）。
pub fn render(value: &serde_json::Value, format: ResponseFormat) -> Option<String> {
    match format {
        ResponseFormat::Json => None,
        ResponseFormat::Yaml => match serde_norway::to_string(value) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(error = %e, "yaml render failed, falling back to json passthrough");
                None
            }
        },
        ResponseFormat::Markdown => Some(markdown(value)),
    }
}

// ── Markdown 投影 ──
//
// 确定性规则（见 docs/response-rendering.md）：
// - 同构扁平对象数组 → 表格（列 = 键并集，单元格转义 `|` 与换行）
// - 标量数组 → 无序列表
// - 对象 → `**key**:` 键值列表，嵌套递归为缩进子列表
// - 多行字符串 → code fence
// - 超深（嵌套 > 4 层）/ 异构数组 → 该子树回退为 yaml code fence

/// markdown 投影的最大嵌套深度，超过则子树回退 yaml fence。
const MAX_DEPTH: usize = 4;

fn markdown(value: &serde_json::Value) -> String {
    let mut out = String::new();
    md_value(value, 0, &mut out);
    out.trim_end().to_string()
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

/// 单行标量的内联呈现。字符串原样（不带引号），调用方保证单行语境。
fn inline(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn is_scalar(v: &serde_json::Value) -> bool {
    !matches!(
        v,
        serde_json::Value::Object(_) | serde_json::Value::Array(_)
    )
}

fn is_multiline(v: &serde_json::Value) -> bool {
    matches!(v, serde_json::Value::String(s) if s.contains('\n'))
}

/// 表格单元格转义：`|` 与换行不能出现在 cell 中。
fn cell(v: &serde_json::Value) -> String {
    inline(v).replace('|', "\\|").replace('\n', " ")
}

fn md_value(v: &serde_json::Value, depth: usize, out: &mut String) {
    if depth >= MAX_DEPTH && !is_scalar(v) {
        yaml_fence(v, depth, out);
        return;
    }
    match v {
        serde_json::Value::Object(map) if map.is_empty() => {
            out.push_str(&format!("{}{{}}\n", indent(depth)));
        }
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                if is_multiline(val) {
                    out.push_str(&format!("{}- **{}**:\n", indent(depth), k));
                    text_fence(val.as_str().unwrap_or_default(), depth + 1, out);
                } else if is_scalar(val) {
                    out.push_str(&format!("{}- **{}**: {}\n", indent(depth), k, inline(val)));
                } else if is_empty_container(val) {
                    out.push_str(&format!(
                        "{}- **{}**: {}\n",
                        indent(depth),
                        k,
                        empty_literal(val)
                    ));
                } else {
                    out.push_str(&format!("{}- **{}**:\n", indent(depth), k));
                    md_value(val, depth + 1, out);
                }
            }
        }
        serde_json::Value::Array(items) if items.is_empty() => {
            out.push_str(&format!("{}[]\n", indent(depth)));
        }
        serde_json::Value::Array(items) => {
            if items.iter().all(|i| is_scalar(i) && !is_multiline(i)) {
                for item in items {
                    out.push_str(&format!("{}- {}\n", indent(depth), inline(item)));
                }
            } else if table_able(items) {
                table(items, depth, out);
            } else {
                // 异构 / 含嵌套元素的数组：与其伪表格不如局部降级 yaml
                yaml_fence(v, depth, out);
            }
        }
        scalar => {
            if is_multiline(scalar) {
                text_fence(scalar.as_str().unwrap_or_default(), depth, out);
            } else {
                out.push_str(&format!("{}{}\n", indent(depth), inline(scalar)));
            }
        }
    }
}

fn is_empty_container(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Object(m) => m.is_empty(),
        serde_json::Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

fn empty_literal(v: &serde_json::Value) -> &'static str {
    if v.is_object() { "{}" } else { "[]" }
}

/// 表格化条件：全部为对象且所有值为标量（键集可不同，列取并集）。
fn table_able(items: &[serde_json::Value]) -> bool {
    items.iter().all(|i| match i {
        serde_json::Value::Object(m) => m.values().all(is_scalar),
        _ => false,
    })
}

fn table(items: &[serde_json::Value], depth: usize, out: &mut String) {
    // 列 = 键并集，按首次出现顺序（serde_json Map 迭代序确定，整体确定性）
    let mut cols: Vec<&str> = Vec::new();
    for item in items {
        if let serde_json::Value::Object(m) = item {
            for k in m.keys() {
                if !cols.iter().any(|c| c == k) {
                    cols.push(k);
                }
            }
        }
    }
    let pad = indent(depth);
    out.push_str(&format!("{pad}| {} |\n", cols.join(" | ")));
    out.push_str(&format!(
        "{pad}|{}\n",
        cols.iter().map(|_| " --- |").collect::<String>()
    ));
    for item in items {
        if let serde_json::Value::Object(m) = item {
            let row: Vec<String> = cols
                .iter()
                .map(|c| m.get(*c).map(cell).unwrap_or_default())
                .collect();
            out.push_str(&format!("{pad}| {} |\n", row.join(" | ")));
        }
    }
}

/// 多行文本 code fence；文本含 ``` 时升级为四反引号围栏。
fn text_fence(text: &str, depth: usize, out: &mut String) {
    let pad = indent(depth);
    let fence = if text.contains("```") { "````" } else { "```" };
    out.push_str(&format!("{pad}{fence}\n"));
    for line in text.lines() {
        out.push_str(&format!("{pad}{line}\n"));
    }
    out.push_str(&format!("{pad}{fence}\n"));
}

/// 子树降级：整体序列化为 yaml 并包进 ```yaml fence。
fn yaml_fence(v: &serde_json::Value, depth: usize, out: &mut String) {
    let yaml = serde_norway::to_string(v).unwrap_or_else(|_| v.to_string());
    let pad = indent(depth);
    out.push_str(&format!("{pad}```yaml\n"));
    for line in yaml.lines() {
        out.push_str(&format!("{pad}{line}\n"));
    }
    out.push_str(&format!("{pad}```\n"));
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolve_precedence_override_wins() {
        let f = resolve_format(
            Some("markdown"),
            Some(ResponseFormat::Yaml),
            Some(ResponseFormat::Json),
        )
        .unwrap();
        assert_eq!(f, ResponseFormat::Markdown);
    }

    #[test]
    fn resolve_precedence_key_over_global() {
        let f = resolve_format(
            None,
            Some(ResponseFormat::Yaml),
            Some(ResponseFormat::Markdown),
        )
        .unwrap();
        assert_eq!(f, ResponseFormat::Yaml);
    }

    #[test]
    fn resolve_precedence_global_then_default_json() {
        let f = resolve_format(None, None, Some(ResponseFormat::Markdown)).unwrap();
        assert_eq!(f, ResponseFormat::Markdown);
        let f = resolve_format(None, None, None).unwrap();
        assert_eq!(f, ResponseFormat::Json);
    }

    #[test]
    fn resolve_unknown_override_fails_fast() {
        let err = resolve_format(Some("xml"), None, None).unwrap_err();
        assert_eq!(err, RenderError::UnknownFormat("xml".to_string()));
        let ae: AsterlaneError = err.into();
        assert_eq!(ae.error_code(), ErrorCode::McpInvalidToolCall);
    }

    #[test]
    fn format_from_accept_matches_media_types() {
        assert_eq!(format_from_accept("application/yaml"), Some("yaml"));
        assert_eq!(format_from_accept("text/markdown, */*"), Some("markdown"));
        assert_eq!(format_from_accept("application/json"), Some("json"));
        assert_eq!(format_from_accept("text/html, */*"), None);
    }

    #[test]
    fn render_json_is_passthrough() {
        assert_eq!(render(&json!({"a": 1}), ResponseFormat::Json), None);
    }

    #[test]
    fn render_yaml_roundtrips() {
        let v = json!({"a": 1, "b": ["x", "y"], "c": {"d": true}});
        let yaml = render(&v, ResponseFormat::Yaml).unwrap();
        let back: serde_json::Value = serde_norway::from_str(&yaml).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn markdown_homogeneous_object_array_becomes_table() {
        let v = json!([
            {"name": "a", "count": 1},
            {"name": "b|pipe", "count": 2, "extra": "e"}
        ]);
        let md = render(&v, ResponseFormat::Markdown).unwrap();
        assert!(md.contains("| count | name |"), "union columns: {md}");
        assert!(md.contains("| extra |"), "late column joins union: {md}");
        assert!(md.contains("b\\|pipe"), "pipe escaped: {md}");
        assert!(md.contains("| --- |"));
    }

    #[test]
    fn markdown_scalar_array_becomes_list() {
        let md = render(&json!(["x", 2, true]), ResponseFormat::Markdown).unwrap();
        assert_eq!(md, "- x\n- 2\n- true");
    }

    #[test]
    fn markdown_object_becomes_keyed_list_with_nesting() {
        let v = json!({"outer": {"inner": 1}, "plain": "v"});
        let md = render(&v, ResponseFormat::Markdown).unwrap();
        assert!(md.contains("- **outer**:\n  - **inner**: 1"), "{md}");
        assert!(md.contains("- **plain**: v"));
    }

    #[test]
    fn markdown_multiline_string_gets_fence() {
        let v = json!({"log": "line1\nline2"});
        let md = render(&v, ResponseFormat::Markdown).unwrap();
        assert!(
            md.contains("- **log**:\n  ```\n  line1\n  line2\n  ```"),
            "{md}"
        );
    }

    #[test]
    fn markdown_heterogeneous_array_falls_back_to_yaml_fence() {
        let v = json!([{"a": 1}, "scalar"]);
        let md = render(&v, ResponseFormat::Markdown).unwrap();
        assert!(md.starts_with("```yaml"), "{md}");
        assert!(md.contains("- a: 1"), "{md}");
    }

    #[test]
    fn markdown_deep_nesting_falls_back_to_yaml_fence() {
        let v = json!({"l1": {"l2": {"l3": {"l4": {"l5": 1}}}}});
        let md = render(&v, ResponseFormat::Markdown).unwrap();
        assert!(md.contains("```yaml"), "deep subtree fenced: {md}");
        assert!(md.contains("l5: 1"), "{md}");
    }

    #[test]
    fn markdown_deterministic() {
        let v = json!({"b": [1, 2], "a": {"x": null}});
        let one = render(&v, ResponseFormat::Markdown).unwrap();
        let two = render(&v, ResponseFormat::Markdown).unwrap();
        assert_eq!(one, two);
    }

    #[test]
    fn markdown_empty_containers_inline() {
        let md = render(&json!({"o": {}, "a": []}), ResponseFormat::Markdown).unwrap();
        assert!(md.contains("- **a**: []"));
        assert!(md.contains("- **o**: {}"));
    }

    #[test]
    fn format_serde_snake_case() {
        let f: ResponseFormat = serde_json::from_str("\"markdown\"").unwrap();
        assert_eq!(f, ResponseFormat::Markdown);
        assert_eq!(
            serde_json::to_string(&ResponseFormat::Yaml).unwrap(),
            "\"yaml\""
        );
    }
}
