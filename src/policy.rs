use crate::config::ProxyKey;
use crate::naming::ToolName;
use regex::RegexSet;
use thiserror::Error;

/// 把配置正则中的冒号段间分隔符翻译为 wire name 的双下划线。
///
/// 配置中可继续使用冒号形式（`^search:tavily:`），policy 层翻译为 wire name
/// 形式（`^search__tavily__`）再匹配。只翻译段间分隔符（把 `:` 替换为 `__`），
/// 不影响段内字符。同时支持已是 wire name 形式的正则（含 `__`）。
/// 详见 docs/naming-convention.md 与 docs/config-schema.md「Proxy Keys」。
fn translate_to_wire_regex(pattern: &str) -> String {
    pattern.replace(':', "__")
}

pub fn key_can_use_tool(key: &ProxyKey, tool_name: &ToolName) -> Result<bool, PolicyError> {
    let full_name = tool_name.to_wire_name();

    if !key.denied_tools.is_empty() {
        let denied: Vec<String> = key
            .denied_tools
            .iter()
            .map(|p| translate_to_wire_regex(p))
            .collect();
        if RegexSet::new(&denied)?.is_match(&full_name) {
            return Ok(false);
        }
    }
    if key.allowed_tools.is_empty() {
        return Ok(false);
    }
    let allowed: Vec<String> = key
        .allowed_tools
        .iter()
        .map(|p| translate_to_wire_regex(p))
        .collect();
    Ok(RegexSet::new(&allowed)?.is_match(&full_name))
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
            discovery_mode: None,
        }
    }

    #[test]
    fn allows_matching_tools_colon_form() {
        let key = key(vec![r"^search:.*"], vec![]);
        let tool = ToolName::new("search", "tavily", "web_search").unwrap();
        assert!(key_can_use_tool(&key, &tool).unwrap());
    }

    #[test]
    fn allows_matching_tools_wire_form() {
        let key = key(vec![r"^search__.*"], vec![]);
        let tool = ToolName::new("search", "tavily", "web_search").unwrap();
        assert!(key_can_use_tool(&key, &tool).unwrap());
    }

    #[test]
    fn deny_rules_override_allow_rules() {
        let key = key(vec![r"^search:.*"], vec![r"^search:exa:.*"]);
        let tool = ToolName::new("search", "exa", "neural_search").unwrap();
        assert!(!key_can_use_tool(&key, &tool).unwrap());
    }

    #[test]
    fn empty_allow_list_denies_by_default() {
        let key = key(vec![], vec![]);
        let tool = ToolName::new("search", "tavily", "web_search").unwrap();
        assert!(!key_can_use_tool(&key, &tool).unwrap());
    }

    #[test]
    fn colon_form_matches_specific_provider() {
        let key = key(vec![r"^search:tavily:"], vec![]);
        let allow = ToolName::new("search", "tavily", "web_search").unwrap();
        assert!(key_can_use_tool(&key, &allow).unwrap());
        let deny = ToolName::new("search", "exa", "neural_search").unwrap();
        assert!(!key_can_use_tool(&key, &deny).unwrap());
    }
}
