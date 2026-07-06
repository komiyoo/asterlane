//! 平台内置 MCP preset 静态表（纯数据模块，不依赖 axum/sqlx/rmcp）。
//!
//! 一行配置 `builtin_mcp: [exa]` 即可启用免鉴权 hosted MCP server，
//! 展开语义见 [`crate::config::GatewayConfig::expand_builtin_mcp`]，
//! 设计契约见 docs/tool-debugging-and-cli.md「内置 MCP Presets」。

/// 单个内置 MCP preset：展开为 `McpServerConfig` 所需的静态描述。
///
/// `id` 同时用作展开后的 `McpServerConfig.id`；全部 preset 免鉴权（`auth: none`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpPreset {
    /// preset id，也是展开后的 mcp server id。
    pub id: &'static str,
    /// wire name 首段 domain。
    pub domain: &'static str,
    /// wire name 中段 provider。
    pub provider: &'static str,
    /// hosted MCP server 的 Streamable HTTP endpoint。
    pub url: &'static str,
    /// 面向控制台/代理的一句话描述。
    pub description: &'static str,
}

/// 返回全部内置 preset（全部免鉴权 `auth: none`）。
pub fn builtin_presets() -> &'static [McpPreset] {
    &[
        McpPreset {
            id: "exa",
            domain: "search",
            provider: "exa",
            url: "https://mcp.exa.ai/mcp",
            description: "Exa hosted MCP server (web search).",
        },
        McpPreset {
            id: "deepwiki",
            domain: "docs",
            provider: "deepwiki",
            url: "https://mcp.deepwiki.com/mcp",
            description: "DeepWiki hosted MCP server (GitHub repository docs).",
        },
        McpPreset {
            id: "context7",
            domain: "docs",
            provider: "context7",
            url: "https://mcp.context7.com/mcp",
            description: "Context7 hosted MCP server (library documentation).",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_ids_unique_and_urls_https() {
        let presets = builtin_presets();
        let mut ids: Vec<&str> = presets.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), presets.len(), "preset ids must be unique");
        for p in presets {
            assert!(p.url.starts_with("https://"), "{} url must be https", p.id);
            assert!(!p.domain.is_empty() && !p.provider.is_empty());
        }
    }
}
