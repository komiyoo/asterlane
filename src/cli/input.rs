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
}
