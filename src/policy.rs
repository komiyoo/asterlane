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

/// Key scope 有效判定（见 docs/mcp-governance-and-key-limits.md §2）：
///
/// 1. `denied_tools` 正则命中 → 拒绝（最高优先）；
/// 2. 允许 = `allowed_tools` 正则命中 ∨ `resource_id ∈ allowed_servers`
///    ∨ wire name ∈ `allowed_tool_names`；
/// 3. 三个允许列表全空 → 全拒绝。
///
/// `resource_id` 由调用方从 catalog `WrappedTool.resource_id` 传入。
pub fn key_can_use_tool(
    key: &ProxyKey,
    tool_name: &ToolName,
    resource_id: &str,
) -> Result<bool, PolicyError> {
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
    if key.allowed_servers.iter().any(|s| s == resource_id)
        || key.allowed_tool_names.iter().any(|n| n == &full_name)
    {
        return Ok(true);
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
            response_format: None,
            allowed_servers: Vec::new(),
            allowed_tool_names: Vec::new(),
            limits: None,
            token_ref: None,
            token_digest: None,
            expires_at: None,
        }
    }

    #[test]
    fn allows_matching_tools_colon_form() {
        let key = key(vec![r"^search:.*"], vec![]);
        let tool = ToolName::new("search", "tavily", "web_search").unwrap();
        assert!(key_can_use_tool(&key, &tool, "tavily").unwrap());
    }

    #[test]
    fn allows_matching_tools_wire_form() {
        let key = key(vec![r"^search__.*"], vec![]);
        let tool = ToolName::new("search", "tavily", "web_search").unwrap();
        assert!(key_can_use_tool(&key, &tool, "tavily").unwrap());
    }

    #[test]
    fn deny_rules_override_allow_rules() {
        let key = key(vec![r"^search:.*"], vec![r"^search:exa:.*"]);
        let tool = ToolName::new("search", "exa", "neural_search").unwrap();
        assert!(!key_can_use_tool(&key, &tool, "exa").unwrap());
    }

    #[test]
    fn empty_allow_lists_deny_by_default() {
        let key = key(vec![], vec![]);
        let tool = ToolName::new("search", "tavily", "web_search").unwrap();
        assert!(!key_can_use_tool(&key, &tool, "tavily").unwrap());
    }

    #[test]
    fn colon_form_matches_specific_provider() {
        let key = key(vec![r"^search:tavily:"], vec![]);
        let allow = ToolName::new("search", "tavily", "web_search").unwrap();
        assert!(key_can_use_tool(&key, &allow, "tavily").unwrap());
        let deny = ToolName::new("search", "exa", "neural_search").unwrap();
        assert!(!key_can_use_tool(&key, &deny, "exa").unwrap());
    }

    // ── 结构化范围（§2）──

    #[test]
    fn allowed_servers_grant_all_tools_of_that_resource() {
        let mut key = key(vec![], vec![]);
        key.allowed_servers = vec!["exa-mcp".to_string()];
        let tool = ToolName::new("search", "exa", "web_search_exa").unwrap();
        assert!(key_can_use_tool(&key, &tool, "exa-mcp").unwrap());
        // 其他 resource 不放行
        assert!(!key_can_use_tool(&key, &tool, "tavily").unwrap());
    }

    #[test]
    fn allowed_tool_names_grant_exact_wire_name() {
        let mut key = key(vec![], vec![]);
        key.allowed_tool_names = vec!["search__exa__web_search_exa".to_string()];
        let exact = ToolName::new("search", "exa", "web_search_exa").unwrap();
        assert!(key_can_use_tool(&key, &exact, "exa-mcp").unwrap());
        let other = ToolName::new("search", "exa", "crawl").unwrap();
        assert!(!key_can_use_tool(&key, &other, "exa-mcp").unwrap());
    }

    #[test]
    fn allow_is_union_of_regex_and_structured_scopes() {
        let mut key = key(vec![r"^reader:.*"], vec![]);
        key.allowed_servers = vec!["exa-mcp".to_string()];
        // 正则命中
        let reader = ToolName::new("reader", "jina", "reader").unwrap();
        assert!(key_can_use_tool(&key, &reader, "jina").unwrap());
        // server 白名单命中（正则未覆盖）
        let exa = ToolName::new("search", "exa", "crawl").unwrap();
        assert!(key_can_use_tool(&key, &exa, "exa-mcp").unwrap());
        // 两者都未命中
        let tavily = ToolName::new("search", "tavily", "web_search").unwrap();
        assert!(!key_can_use_tool(&key, &tavily, "tavily").unwrap());
    }

    #[test]
    fn denied_regex_overrides_structured_scopes() {
        let mut key = key(vec![], vec![r"^search:exa:crawl$"]);
        key.allowed_servers = vec!["exa-mcp".to_string()];
        key.allowed_tool_names = vec!["search__exa__crawl".to_string()];
        let tool = ToolName::new("search", "exa", "crawl").unwrap();
        assert!(!key_can_use_tool(&key, &tool, "exa-mcp").unwrap());
    }
}
