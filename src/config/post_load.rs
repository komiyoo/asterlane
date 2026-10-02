//! 反序列化之后的配置处理：校验与 builtin preset 展开。
//!
//! `main.rs` 的 `load_config` / `parse_config_file` 读入 YAML 后依次调用；
//! 任一步失败都 fail fast，不带着非法配置启动。

use super::{GatewayConfig, HealthCheckConfig, McpServerConfig, SecurityConfig, UpstreamAuth};
use crate::error::{AsterlaneError, ErrorCode};

impl GatewayConfig {
    /// 校验入站 HTTP 护栏：`max_body_bytes` 必须大于 0。
    pub fn validate_http(&self) -> Result<(), AsterlaneError> {
        if self.http.max_body_bytes == 0 {
            return Err(AsterlaneError::internal(
                ErrorCode::ConfigInvalidYaml,
                "http.max_body_bytes must be greater than 0",
            ));
        }
        Ok(())
    }

    /// 校验 proxy key 凭据字段（见 docs/runtime/key-credentials-and-persistence.md K1）。
    ///
    /// - `token_ref` 与 `token_digest` 互斥；
    /// - `token_digest` 必须为 64 位小写 hex（SHA-256）。
    ///
    /// 配置加载后调用（`main.rs` 的 `load_config`），失败 fail fast。
    pub fn validate_key_credentials(&self) -> Result<(), AsterlaneError> {
        for key in &self.proxy_keys {
            if let Some(mode) = key.discovery_mode.as_deref()
                && !matches!(mode, "lazy" | "full")
            {
                return Err(AsterlaneError::internal(
                    ErrorCode::ConfigInvalidYaml,
                    format!("proxy key {}: discovery_mode must be lazy or full", key.id),
                ));
            }
            if key.token_ref.is_some() && key.token_digest.is_some() {
                return Err(AsterlaneError::internal(
                    ErrorCode::ConfigInvalidYaml,
                    format!(
                        "proxy key {}: token_ref and token_digest are mutually exclusive",
                        key.id
                    ),
                ));
            }
            if let Some(digest) = &key.token_digest
                && !(digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            {
                return Err(AsterlaneError::internal(
                    ErrorCode::ConfigInvalidYaml,
                    format!(
                        "proxy key {}: token_digest must be 64 lowercase hex chars",
                        key.id
                    ),
                ));
            }
        }
        Ok(())
    }

    /// 把 `builtin_mcp` 中的 preset 展开为 [`McpServerConfig`] 追加进 `mcp_servers`。
    ///
    /// 配置加载后调用（`main.rs` 的 `load_config`）。展开语义
    /// （见 docs/admin/tool-debugging-and-cli.md「内置 MCP Presets」）：
    ///
    /// - 显式 `mcp_servers` 已有同 id 条目时跳过该 preset（显式配置优先，
    ///   可用于覆盖 security 等字段）；`builtin_mcp` 列表内重复 id 只展开一次；
    /// - 未知 preset id 返回 `config.unknown_resource` 错误 fail fast，
    ///   错误信息列出可用 preset id；
    /// - `builtin_mcp` 简写仅对 keyless preset 合法；引用 keyed preset
    ///   （`auth ≠ none`）返回 `config.invalid_yaml` 错误 fail fast——避免静默
    ///   生成一个无凭据的坏 server，需改用 `mcp_servers` 显式配置 secret ref。
    pub fn expand_builtin_mcp(&mut self) -> Result<(), AsterlaneError> {
        for id in &self.builtin_mcp {
            let preset = crate::presets::builtin_presets()
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| {
                    let available: Vec<&str> = crate::presets::builtin_presets()
                        .iter()
                        .map(|p| p.id)
                        .collect();
                    AsterlaneError::internal(
                        ErrorCode::ConfigUnknownResource,
                        format!(
                            "unknown builtin_mcp preset: {id} (available: {})",
                            available.join(", ")
                        ),
                    )
                })?;
            // keyed preset 不能经 builtin_mcp 简写零配置启用（会生成 auth:none 的坏
            // server）；须在 mcp_servers 显式配置 auth: bearer/header + secret ref
            if preset.requires_key() {
                return Err(AsterlaneError::internal(
                    ErrorCode::ConfigInvalidYaml,
                    format!(
                        "builtin_mcp preset '{id}' requires a key: configure it under \
                         mcp_servers with `auth: bearer` and a `secret://…` ref instead of \
                         listing it in builtin_mcp"
                    ),
                ));
            }
            // 显式同 id 条目优先；首次展开后 id 已入列，天然去重列表内重复
            if self.mcp_servers.iter().any(|s| s.id == preset.id) {
                continue;
            }
            self.mcp_servers.push(McpServerConfig {
                id: preset.id.to_string(),
                domain: preset.domain.to_string(),
                provider: preset.provider.to_string(),
                url: preset.url.to_string(),
                description: preset.description.to_string(),
                auth: UpstreamAuth::None,
                security: SecurityConfig::default(),
                health_check: HealthCheckConfig::default(),
                limits: None,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_support::parse;

    #[test]
    fn expands_builtin_presets_into_mcp_servers() {
        let mut config = parse("builtin_mcp: [exa, deepwiki]");
        config.expand_builtin_mcp().expect("expand");
        assert_eq!(config.mcp_servers.len(), 2);
        let exa = config.mcp_server("exa").expect("exa expanded");
        assert_eq!(exa.domain, "search");
        assert_eq!(exa.provider, "exa");
        assert_eq!(exa.url, "https://mcp.exa.ai/mcp");
        assert!(exa.auth.is_none());
        assert_eq!(exa.security, SecurityConfig::default());
        assert!(config.mcp_server("deepwiki").is_some());
    }

    #[test]
    fn explicit_mcp_server_with_same_id_wins() {
        let mut config = parse(
            r#"
builtin_mcp: [exa]
mcp_servers:
  - id: exa
    domain: custom
    provider: exa
    url: https://example.test/mcp
"#,
        );
        config.expand_builtin_mcp().expect("expand");
        assert_eq!(config.mcp_servers.len(), 1);
        let exa = config.mcp_server("exa").expect("exa kept");
        assert_eq!(exa.domain, "custom");
        assert_eq!(exa.url, "https://example.test/mcp");
    }

    #[test]
    fn duplicate_ids_in_builtin_list_expand_once() {
        let mut config = parse("builtin_mcp: [context7, context7]");
        config.expand_builtin_mcp().expect("expand");
        assert_eq!(config.mcp_servers.len(), 1);
    }

    #[test]
    fn unknown_preset_id_fails_with_config_code() {
        let mut config = parse("builtin_mcp: [nope]");
        let err = config.expand_builtin_mcp().expect_err("must fail");
        match err {
            AsterlaneError::Internal { code, message, .. } => {
                assert_eq!(code, ErrorCode::ConfigUnknownResource);
                assert!(message.contains("nope"));
                // 错误信息列出可用 preset id
                assert!(message.contains("exa"));
                assert!(message.contains("deepwiki"));
                assert!(message.contains("context7"));
            }
            other => panic!("unexpected error variant: {other}"),
        }
    }

    #[test]
    fn keyed_preset_in_builtin_mcp_fails_fast() {
        let mut config = parse("builtin_mcp: [rollinggo-hotel]");
        let err = config
            .expand_builtin_mcp()
            .expect_err("keyed preset must fail");
        match err {
            AsterlaneError::Internal { code, message, .. } => {
                assert_eq!(code, ErrorCode::ConfigInvalidYaml);
                assert!(message.contains("rollinggo-hotel"));
                assert!(message.contains("requires a key"));
                assert!(message.contains("mcp_servers"));
            }
            other => panic!("unexpected error variant: {other}"),
        }
        // 未静默生成坏 server
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn discovery_mode_rejects_unknown_values() {
        let config = parse("proxy_keys:\n  - id: agent-a\n    discovery_mode: unknown\n");
        let error = config
            .validate_key_credentials()
            .expect_err("invalid discovery mode");
        assert_eq!(error.error_code(), ErrorCode::ConfigInvalidYaml);
        assert!(error.to_string().contains("discovery_mode"));
    }

    #[test]
    fn token_ref_and_digest_are_mutually_exclusive() {
        let config = parse(&format!(
            "proxy_keys:\n  - id: agent-a\n    token_ref: secret://env/T\n    token_digest: \"{}\"\n",
            "a".repeat(64)
        ));
        let err = config.validate_key_credentials().expect_err("must fail");
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn token_digest_must_be_lowercase_hex64() {
        for bad in ["abc", &"A".repeat(64), &"g".repeat(64)] {
            let config = parse(&format!(
                "proxy_keys:\n  - id: agent-a\n    token_digest: \"{bad}\"\n"
            ));
            assert!(
                config.validate_key_credentials().is_err(),
                "digest {bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn http_zero_body_limit_is_rejected() {
        let config = parse("http:\n  max_body_bytes: 0");
        let err = config.validate_http().expect_err("zero body must fail");
        match err {
            AsterlaneError::Internal { code, message, .. } => {
                assert_eq!(code, ErrorCode::ConfigInvalidYaml);
                assert!(message.contains("max_body_bytes"));
            }
            other => panic!("unexpected error variant: {other}"),
        }
    }

    #[test]
    fn http_zero_timeout_disables_without_failing_validation() {
        let config = parse("http:\n  request_timeout_secs: 0");
        assert_eq!(config.http.request_timeout_secs, 0);
        assert!(config.validate_http().is_ok());
    }
}
