//! 平台内置 MCP preset 静态表（纯数据模块，不依赖 axum/sqlx/rmcp）。
//!
//! keyless preset（`PresetAuth::None`）一行配置 `builtin_mcp: [exa]` 即可零配置
//! 启用；keyed preset 只作为「填 key 即用」的目录条目，须在 `mcp_servers` 显式
//! 配置 secret ref。展开语义见 [`crate::config::GatewayConfig::expand_builtin_mcp`]，
//! 设计契约见 docs/tool-debugging-and-cli.md「内置 MCP Presets」。

/// preset 的默认凭据形态：表达展开为 `McpServerConfig` 时的 `UpstreamAuth` 方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetAuth {
    /// 免鉴权（`auth: none`），可经 `builtin_mcp` 零配置启用。
    None,
    /// Bearer token（`auth: bearer` + secret ref）。
    Bearer,
    /// 自定义 header（`auth: header` + secret ref），`name` 为 header 名。
    Header { name: &'static str },
}

/// 单个内置 MCP preset：展开为 `McpServerConfig` 所需的静态描述。
///
/// `id` 同时用作展开后的 `McpServerConfig.id`。`auth` 表达启用该 preset 默认
/// 需要的凭据形态；`apply_url` 指向申请原始 key 的地址（keyless 为 `None`）。
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
    /// 默认凭据形态：启用该 preset 所需的鉴权方式。
    pub auth: PresetAuth,
    /// 申请原始 key 的地址；keyless preset 为 `None`。
    pub apply_url: Option<&'static str>,
}

impl McpPreset {
    /// 是否需要凭据（派生自 `auth`）。keyless preset 可经 `builtin_mcp` 零配置启用。
    pub fn requires_key(&self) -> bool {
        !matches!(self.auth, PresetAuth::None)
    }
}

/// 返回全部内置 preset（keyless + keyed 混合，凭据形态见各条 `auth`）。
pub fn builtin_presets() -> &'static [McpPreset] {
    &[
        McpPreset {
            id: "exa",
            domain: "search",
            provider: "exa",
            url: "https://mcp.exa.ai/mcp",
            description: "Exa hosted MCP server (web search).",
            auth: PresetAuth::None,
            apply_url: None,
        },
        McpPreset {
            id: "deepwiki",
            domain: "docs",
            provider: "deepwiki",
            url: "https://mcp.deepwiki.com/mcp",
            description: "DeepWiki hosted MCP server (GitHub repository docs).",
            auth: PresetAuth::None,
            apply_url: None,
        },
        McpPreset {
            id: "context7",
            domain: "docs",
            provider: "context7",
            url: "https://mcp.context7.com/mcp",
            description: "Context7 hosted MCP server (library documentation).",
            auth: PresetAuth::None,
            apply_url: None,
        },
        McpPreset {
            id: "rollinggo-hotel",
            domain: "hotel",
            provider: "rollinggo",
            url: "https://mcp.rollinggo.cn/mcp",
            description: "RollingGo hosted MCP server (hotel booking, Bearer key).",
            auth: PresetAuth::Bearer,
            apply_url: Some("https://rollinggo.store/apply"),
        },
        McpPreset {
            id: "rollinggo-flight",
            domain: "flight",
            provider: "rollinggo",
            url: "https://mcp.rollinggo.cn/mcp/flight",
            description: "RollingGo hosted MCP server (flight booking, Bearer key).",
            auth: PresetAuth::Bearer,
            apply_url: Some("https://rollinggo.store/apply"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_ids_unique_urls_https_and_apply_url_matches_key_requirement() {
        let presets = builtin_presets();
        let mut ids: Vec<&str> = presets.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), presets.len(), "preset ids must be unique");
        for p in presets {
            assert!(p.url.starts_with("https://"), "{} url must be https", p.id);
            assert!(!p.domain.is_empty() && !p.provider.is_empty());
            // keyed preset 必须有申请地址；keyless 必须无（避免误导免费集成去申请 key）
            if p.requires_key() {
                assert!(
                    p.apply_url.is_some(),
                    "keyed preset {} must have apply_url",
                    p.id
                );
            } else {
                assert!(
                    p.apply_url.is_none(),
                    "keyless preset {} must not have apply_url",
                    p.id
                );
            }
        }
    }

    #[test]
    fn rollinggo_presets_are_bearer_keyed() {
        let presets = builtin_presets();
        for id in ["rollinggo-hotel", "rollinggo-flight"] {
            let p = presets.iter().find(|p| p.id == id).expect("preset present");
            assert_eq!(p.provider, "rollinggo");
            assert!(matches!(p.auth, PresetAuth::Bearer));
            assert!(p.requires_key());
            assert_eq!(p.apply_url, Some("https://rollinggo.store/apply"));
        }
    }
}
