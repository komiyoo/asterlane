use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolName {
    pub domain: String,
    pub tool: String,
    pub method: String,
}

impl ToolName {
    pub fn new(
        domain: impl Into<String>,
        tool: impl Into<String>,
        method: impl Into<String>,
    ) -> Result<Self, ToolNameError> {
        let name = Self {
            domain: normalize_segment(domain.into())?,
            tool: normalize_segment(tool.into())?,
            method: normalize_segment(method.into())?,
        };
        Ok(name)
    }

    pub fn as_mcp_name(&self) -> String {
        format!("{}:{}:{}", self.domain, self.tool, self.method)
    }
}

impl Display for ToolName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_mcp_name())
    }
}

impl FromStr for ToolName {
    type Err = ToolNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = value.split(':').collect();
        if parts.len() != 3 {
            return Err(ToolNameError::InvalidShape(value.to_string()));
        }
        ToolName::new(parts[0], parts[1], parts[2])
    }
}

fn normalize_segment(value: String) -> Result<String, ToolNameError> {
    let trimmed = value.trim().to_ascii_lowercase().replace([' ', '_'], "-");
    if trimmed.is_empty() {
        return Err(ToolNameError::EmptySegment);
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(ToolNameError::InvalidSegment(trimmed));
    }
    Ok(trimmed)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolNameError {
    #[error("tool name must be domain:tool:method, got {0}")]
    InvalidShape(String),
    #[error("tool name segment cannot be empty")]
    EmptySegment,
    #[error("tool name segment contains unsupported characters: {0}")]
    InvalidSegment(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tool_name_segments() {
        let name = ToolName::new("Web Search", "Tavily", "POST").unwrap();
        assert_eq!(name.as_mcp_name(), "web-search:tavily:post");
    }

    #[test]
    fn parses_three_segment_names() {
        let name: ToolName = "search:exa:post".parse().unwrap();
        assert_eq!(name.domain, "search");
        assert_eq!(name.tool, "exa");
        assert_eq!(name.method, "post");
    }

    #[test]
    fn rejects_invalid_shapes() {
        let error = "search:exa".parse::<ToolName>().unwrap_err();
        assert_eq!(error, ToolNameError::InvalidShape("search:exa".to_string()));
    }
}
