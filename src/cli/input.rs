use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::PathBuf;

pub(super) fn load_json_object(
    args: Option<String>,
    file: Option<PathBuf>,
) -> Result<Option<Value>> {
    let raw = match (args, file) {
        (Some(inline), _) => inline,
        (None, Some(path)) => std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read args file {}", path.display()))?,
        (None, None) => return Ok(None),
    };
    let value: Value = serde_json::from_str(&raw).context("args must be valid JSON")?;
    if !value.is_object() {
        bail!("args must be a JSON object");
    }
    Ok(Some(value))
}

/// 读取 admin 写操作 body：`--json` 字面量或 `--from-file`（JSON / YAML object）。
pub(super) fn load_object_document(json: Option<String>, file: Option<PathBuf>) -> Result<Value> {
    match (json, file) {
        (Some(_), Some(_)) => bail!("--json and --from-file are mutually exclusive"),
        (None, None) => bail!("requires --json or --from-file"),
        (Some(raw), None) => parse_object_document(&raw),
        (None, Some(path)) => {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            parse_object_document(&raw)
                .with_context(|| format!("invalid object in {}", path.display()))
        }
    }
}

/// JSON 优先，失败再按 YAML 解析；必须是 object。
pub(super) fn parse_object_document(raw: &str) -> Result<Value> {
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => ensure_object(value),
        Err(json_err) => match serde_norway::from_str::<Value>(raw) {
            Ok(value) => ensure_object(value),
            Err(_) => Err(json_err).context("body must be a JSON or YAML object"),
        },
    }
}

/// PUT 路径 id 覆盖 body `id`，避免文件里的 id 与 URL 不一致。
pub(super) fn overlay_id(mut body: Value, id: &str) -> Result<Value> {
    let obj = body
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("body must be a JSON object"))?;
    obj.insert("id".to_string(), Value::String(id.to_string()));
    Ok(body)
}

fn ensure_object(value: Value) -> Result<Value> {
    if !value.is_object() {
        bail!("body must be a JSON object");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_object_and_rejects_invalid_shapes() {
        assert_eq!(load_json_object(None, None).unwrap(), None);
        assert_eq!(
            load_json_object(Some(r#"{"q":"rust"}"#.into()), None).unwrap(),
            Some(json!({"q": "rust"}))
        );
        assert!(load_json_object(Some("[1,2]".into()), None).is_err());
        assert!(load_json_object(Some("not json".into()), None).is_err());
    }

    #[test]
    fn parse_object_document_accepts_json_and_yaml() {
        assert_eq!(
            parse_object_document(r#"{"id":"mock"}"#).unwrap(),
            json!({"id": "mock"})
        );
        assert_eq!(
            parse_object_document("id: mock\ndomain: search\n").unwrap()["id"],
            "mock"
        );
        assert!(parse_object_document("[1]").is_err());
        assert!(parse_object_document("not: [unterminated").is_err());
    }

    #[test]
    fn overlay_id_replaces_body_id() {
        let out = overlay_id(json!({"id": "old", "domain": "search"}), "new").unwrap();
        assert_eq!(out["id"], "new");
        assert_eq!(out["domain"], "search");
    }

    #[test]
    fn load_object_document_requires_a_source() {
        assert!(load_object_document(None, None).is_err());
    }
}
