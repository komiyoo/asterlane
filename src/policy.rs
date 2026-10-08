use crate::config::ProxyKey;
use crate::naming::ToolName;
use regex::RegexSet;
use thiserror::Error;

/// 把配置正则中的冒号段间分隔符翻译为 wire name 的双下划线。
///
/// 配置中可继续使用冒号形式（`^search:tavily:`），policy 层翻译为 wire name
/// 形式（`^search__tavily__`）再匹配。只翻译段间分隔符（把 `:` 替换为 `__`），
/// 不影响段内字符。同时支持已是 wire name 形式的正则（含 `__`）。
/// 详见 docs/architecture/naming-convention.md 与 docs/runtime/config-schema.md「Proxy Keys」。
fn translate_to_wire_regex(pattern: &str) -> String {
    pattern.replace(':', "__")
}

/// Key scope 有效判定（见 docs/runtime/mcp-governance-and-key-limits.md §2）。
///
/// `resource_id` 由调用方从 catalog `WrappedTool.resource_id` 传入。
/// 规则在 [`key_can_use_name`]：prompts 与 resources 传包装后的匹配名。
pub fn key_can_use_tool(
    key: &ProxyKey,
    tool_name: &ToolName,
    resource_id: &str,
) -> Result<bool, PolicyError> {
    key_can_use_name(key, &tool_name.to_wire_name(), resource_id)
}

/// 按字符串匹配名判权。工具、prompt、resource、template 共用：
///
/// 1. `denied_tools` 正则命中 → 拒绝（最高优先）；
/// 2. 允许 = `allowed_tools` 正则命中 ∨ `resource_id ∈ allowed_servers`
///    ∨ `full_name ∈ allowed_tool_names`；
/// 3. 三个允许列表全空 → 全拒绝。
///
/// `full_name` 是 `domain__provider__<name>`。`resource_id` 是上游 server id
/// （HTTP API 则是 resource id）。
pub fn key_can_use_name(
    key: &ProxyKey,
    full_name: &str,
    resource_id: &str,
) -> Result<bool, PolicyError> {
    if !key.denied_tools.is_empty() {
        let denied: Vec<String> = key
            .denied_tools
            .iter()
            .map(|p| translate_to_wire_regex(p))
            .collect();
        if RegexSet::new(&denied)?.is_match(full_name) {
            return Ok(false);
        }
    }
    if key.allowed_servers.iter().any(|s| s == resource_id)
        || key.allowed_tool_names.iter().any(|n| n == full_name)
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
    Ok(RegexSet::new(&allowed)?.is_match(full_name))
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

    // ── 按字符串名判权（prompt、resource、template 共用）──

    #[test]
    fn name_check_agrees_with_tool_check() {
        let tool = ToolName::new("docs", "wiki", "summarize").unwrap();
        let wire = tool.to_wire_name();
        let mut scoped = key(vec![r"^docs:.*"], vec![]);
        scoped.allowed_servers = vec!["docs".to_string()];
        let cases = [
            key(vec![r"^docs:.*"], vec![]),
            key(vec![r"^docs:.*"], vec![r"^docs:wiki:"]),
            key(vec![], vec![]),
            key(vec![r"^other:"], vec![]),
            scoped,
        ];
        for key in cases {
            for resource_id in ["docs", "other"] {
                assert_eq!(
                    key_can_use_tool(&key, &tool, resource_id).unwrap(),
                    key_can_use_name(&key, &wire, resource_id).unwrap()
                );
            }
        }
    }

    #[test]
    fn name_check_matches_raw_names_with_characters_tool_names_reject() {
        let name = "docs__wiki__guide/README.md v2";
        // 配置里的冒号形式与 wire 形式都按前缀匹配
        assert!(key_can_use_name(&key(vec![r"^docs:wiki:"], vec![]), name, "x").unwrap());
        assert!(key_can_use_name(&key(vec![r"^docs__wiki__guide/"], vec![]), name, "x").unwrap());
        assert!(!key_can_use_name(&key(vec![r"^docs:other:"], vec![]), name, "x").unwrap());
    }

    #[test]
    fn name_check_deny_wins_over_every_allow_rule() {
        let mut key = key(vec![r"^docs:.*"], vec![r"^docs:wiki:secret"]);
        key.allowed_servers = vec!["docs".to_string()];
        key.allowed_tool_names = vec!["docs__wiki__secret".to_string()];
        assert!(!key_can_use_name(&key, "docs__wiki__secret", "docs").unwrap());
        assert!(key_can_use_name(&key, "docs__wiki__public", "docs").unwrap());
    }

    #[test]
    fn name_check_structured_scopes_and_empty_lists() {
        let mut key = key(vec![], vec![]);
        // 三个允许列表全空：全拒绝
        assert!(!key_can_use_name(&key, "docs__wiki__a", "docs").unwrap());
        key.allowed_servers = vec!["docs".to_string()];
        assert!(key_can_use_name(&key, "docs__wiki__a", "docs").unwrap());
        assert!(!key_can_use_name(&key, "docs__wiki__a", "ops").unwrap());
        key.allowed_servers.clear();
        key.allowed_tool_names = vec!["docs__wiki__a".to_string()];
        assert!(key_can_use_name(&key, "docs__wiki__a", "ops").unwrap());
        assert!(!key_can_use_name(&key, "docs__wiki__b", "ops").unwrap());
    }

    #[test]
    fn name_check_reports_invalid_regex() {
        let allow = key(vec!["("], vec![]);
        assert!(key_can_use_name(&allow, "docs__wiki__a", "docs").is_err());
        let deny = key(vec![], vec!["("]);
        assert!(key_can_use_name(&deny, "docs__wiki__a", "docs").is_err());
    }
}
