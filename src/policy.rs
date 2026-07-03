use crate::config::ProxyKey;
use crate::naming::ToolName;
use regex::RegexSet;
use thiserror::Error;

pub fn key_can_use_tool(key: &ProxyKey, tool_name: &ToolName) -> Result<bool, PolicyError> {
    let full_name = tool_name.as_mcp_name();
    if !key.denied_tools.is_empty() && RegexSet::new(&key.denied_tools)?.is_match(&full_name) {
        return Ok(false);
    }
    if key.allowed_tools.is_empty() {
        return Ok(false);
    }
    Ok(RegexSet::new(&key.allowed_tools)?.is_match(&full_name))
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("invalid tool scope regex: {0}")]
    InvalidRegex(#[from] regex::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(allowed_tools: Vec<&str>, denied_tools: Vec<&str>) -> ProxyKey {
        ProxyKey {
            id: "agent-dev".to_string(),
            display_name: "Agent Dev".to_string(),
            allowed_tools: allowed_tools.into_iter().map(str::to_string).collect(),
            denied_tools: denied_tools.into_iter().map(str::to_string).collect(),
            default_tool_page_size: 20,
        }
    }

    #[test]
    fn allows_matching_tools() {
        let key = key(vec![r"^search:.*"], vec![]);
        let tool = "search:tavily:post".parse().unwrap();
        assert!(key_can_use_tool(&key, &tool).unwrap());
    }

    #[test]
    fn deny_rules_override_allow_rules() {
        let key = key(vec![r"^search:.*"], vec![r"^search:exa:.*"]);
        let tool = "search:exa:post".parse().unwrap();
        assert!(!key_can_use_tool(&key, &tool).unwrap());
    }

    #[test]
    fn empty_allow_list_denies_by_default() {
        let key = key(vec![], vec![]);
        let tool = "search:tavily:post".parse().unwrap();
        assert!(!key_can_use_tool(&key, &tool).unwrap());
    }
}
