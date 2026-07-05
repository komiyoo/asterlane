use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use thiserror::Error;

/// Wire name 最大长度（字符数）。
///
/// Claude Code 把 MCP 工具展开为 `mcp__<server>__<tool>`，总长上限 64 字符。
/// 假设注册的 server 名为 `asterlane`（11 字符），前缀 `mcp__asterlane__` 占 17 字符，
/// 剩余 47 字符给工具名。这里取 64 作为绝对上限，超长直接报配置错误。
/// 详见 docs/naming-convention.md「长度预算」。
const MAX_WIRE_NAME_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolName {
    pub domain: String,
    pub provider: String,
    pub tool: String,
}

impl ToolName {
    pub fn new(
        domain: impl Into<String>,
        provider: impl Into<String>,
        tool: impl Into<String>,
    ) -> Result<Self, ToolNameError> {
        let name = Self {
            domain: normalize_segment(domain.into())?,
            provider: normalize_segment(provider.into())?,
            tool: normalize_segment(tool.into())?,
        };
        let wire = name.to_wire_name();
        if wire.len() > MAX_WIRE_NAME_LEN {
            return Err(ToolNameError::Overlong(wire));
        }
        Ok(name)
    }

    /// 输出对外 wire name: `domain__provider__tool`
    ///（双下划线 `__` 分段，段内单词用单下划线如 `web_search`）。
    pub fn to_wire_name(&self) -> String {
        format!("{}__{}__{}", self.domain, self.provider, self.tool)
    }
}

impl Display for ToolName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_wire_name())
    }
}

impl FromStr for ToolName {
    type Err = ToolNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = value.split("__").collect();
        if parts.len() != 3 {
            return Err(ToolNameError::InvalidShape(value.to_string()));
        }
        ToolName::new(parts[0], parts[1], parts[2])
    }
}

fn normalize_segment(value: String) -> Result<String, ToolNameError> {
    let trimmed = value.trim().to_ascii_lowercase().replace(' ', "-");
    if trimmed.is_empty() {
        return Err(ToolNameError::EmptySegment);
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ToolNameError::InvalidSegment(trimmed));
    }
    Ok(trimmed)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolNameError {
    #[error("tool name must be domain__provider__tool, got {0}")]
    InvalidShape(String),
    #[error("tool name segment cannot be empty")]
    EmptySegment,
    #[error("tool name segment contains unsupported characters: {0}")]
    InvalidSegment(String),
    #[error("tool name exceeds {max} characters: {0}", max = MAX_WIRE_NAME_LEN)]
    Overlong(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tool_name_segments() {
        let name = ToolName::new("Search", "Tavily", "Web Search").unwrap();
        assert_eq!(name.to_wire_name(), "search__tavily__web-search");
    }

    #[test]
    fn parses_three_segment_wire_names() {
        let name: ToolName = "search__exa__neural_search".parse().unwrap();
        assert_eq!(name.domain, "search");
        assert_eq!(name.provider, "exa");
        assert_eq!(name.tool, "neural_search");
    }

    #[test]
    fn displays_as_wire_name() {
        let name = ToolName::new("mcp", "github", "list_issues").unwrap();
        assert_eq!(name.to_string(), "mcp__github__list_issues");
    }

    #[test]
    fn rejects_invalid_shapes() {
        let error = "search:exa:post".parse::<ToolName>().unwrap_err();
        assert_eq!(
            error,
            ToolNameError::InvalidShape("search:exa:post".to_string())
        );
    }

    #[test]
    fn rejects_too_few_segments() {
        let error = "search__exa".parse::<ToolName>().unwrap_err();
        assert_eq!(
            error,
            ToolNameError::InvalidShape("search__exa".to_string())
        );
    }

    #[test]
    fn rejects_overlong_wire_name() {
        // 3×20 + 4 (two `__` separators) = 64, so 3×21 + 4 = 67 > 64
        let seg = "a".repeat(21);
        let error = ToolName::new(&seg, &seg, &seg).unwrap_err();
        assert!(matches!(error, ToolNameError::Overlong(_)));
    }

    #[test]
    fn accepts_boundary_length() {
        // 3×20 + 4 = 64 ≤ 64
        let seg = "a".repeat(20);
        let name = ToolName::new(&seg, &seg, &seg).unwrap();
        assert_eq!(name.to_wire_name().len(), 64);
    }
}
