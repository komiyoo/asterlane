use anyhow::{Result, anyhow};
use serde_json::Value;
use std::io::{self, IsTerminal};

use crate::render::{self, ResponseFormat};

pub(super) fn resolve_cli_format(flag: Option<&str>) -> Result<ResponseFormat> {
    let env = std::env::var("ASTERLANE_FORMAT").ok();
    resolve_from(flag, env.as_deref(), io::stdout().is_terminal())
}

fn resolve_from(
    flag: Option<&str>,
    env: Option<&str>,
    is_terminal: bool,
) -> Result<ResponseFormat> {
    match flag.or(env) {
        Some(value) => value
            .parse::<ResponseFormat>()
            .map_err(|error| anyhow!(error)),
        None if is_terminal => Ok(ResponseFormat::Markdown),
        None => Ok(ResponseFormat::Json),
    }
}

pub(super) fn format_value(value: &Value, format: ResponseFormat) -> String {
    render::render(value, format).unwrap_or_else(|| pretty(value))
}

pub(super) fn emit(value: &Value, format: ResponseFormat) {
    println!("{}", format_value(value, format));
}

pub(super) fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_priority_is_flag_then_env_then_terminal() {
        assert_eq!(
            resolve_from(Some("yaml"), Some("json"), true).unwrap(),
            ResponseFormat::Yaml
        );
        assert_eq!(
            resolve_from(None, Some("json"), true).unwrap(),
            ResponseFormat::Json
        );
        assert_eq!(
            resolve_from(None, None, true).unwrap(),
            ResponseFormat::Markdown
        );
        assert_eq!(
            resolve_from(None, None, false).unwrap(),
            ResponseFormat::Json
        );
        assert!(resolve_from(Some("xml"), None, false).is_err());
    }

    #[test]
    fn value_formatting_reuses_render_module() {
        let value = json!({"ok": true});
        assert!(format_value(&value, ResponseFormat::Json).contains("\"ok\""));
        assert!(format_value(&value, ResponseFormat::Yaml).contains("ok: true"));
        assert!(format_value(&value, ResponseFormat::Markdown).contains("**ok**"));
    }
}
