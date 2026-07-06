//! 在线配置持久化合并（见 docs/key-credentials-and-persistence.md K2）。
//!
//! 启动时把 admin CRUD 落库的 resources / mcp_servers / proxy_keys 回读并
//! 并入 YAML 配置：同 id 冲突 YAML 胜（git 为 source of truth，DB 行被
//! 遮蔽计入报告，由调用方 warn），DB 独有条目按 id 升序追加。
//!
//! 反向映射与写方向一一对应：`admin/crud.rs` 的 `to_db_resource` /
//! `to_db_proxy_key`、`admin/mcp.rs` 的 `to_db_record`。config_json /
//! scope_json 解析失败只跳过该行（计入报告并 warn，不打印原文），
//! 不让单行坏数据阻塞启动。
//!
//! `merge_db_into_config` 为纯函数（不做 IO），DB 行由调用方经
//! [`load_db_entries`] 读出传入。调用顺序：YAML 加载后、
//! `expand_builtin_mcp()` 展开前——DB 并入的同 id 条目会让 preset
//! 展开自然跳过（显式条目优先语义不变）。

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tracing::{info, warn};

use crate::config::{
    ApiResource, GatewayConfig, HealthCheckConfig, KeyLimits, McpServerConfig, ProxyKey,
    SecurityConfig, UpstreamAuth, UpstreamLimits,
};
use crate::render::ResponseFormat;
use crate::store::error::StoreError;
use crate::store::mcp_servers::{McpServerRecord, McpServerRepository};
use crate::store::repository::{ProxyKeyRecord, ProxyKeyRepository, Resource, ResourceRepository};
use crate::store::sqlite::SqliteRequestEventRepository;

/// 合并结果报告：wave 2（main.rs serve）据此打启动日志。
#[derive(Debug, Clone, Default)]
pub struct MergeReport {
    /// 从 DB 并入的 resource id（id 升序）。
    pub added_resources: Vec<String>,
    /// 从 DB 并入的 mcp server id（id 升序）。
    pub added_mcp_servers: Vec<String>,
    /// 从 DB 并入的 proxy key id（id 升序）。
    pub added_proxy_keys: Vec<String>,
    /// 被 YAML 同 id 条目遮蔽的 DB 行（`entity:id` 形式），调用方应 warn。
    pub shadowed: Vec<String>,
    /// config_json / scope_json 无法解析而跳过的 DB 行（`entity:id` 形式）。
    pub skipped_invalid: Vec<String>,
}

/// 一次读出三类可持久化配置实体（供 serve 启动合并一行调用）。
pub async fn load_db_entries(
    repo: &SqliteRequestEventRepository,
) -> Result<(Vec<Resource>, Vec<McpServerRecord>, Vec<ProxyKeyRecord>), StoreError> {
    Ok((
        repo.list_resources().await?,
        repo.list_mcp_servers().await?,
        repo.list_proxy_keys().await?,
    ))
}

/// serve() 启动合并入口：回读 DB 三类实体并入配置 + 启动日志 +
/// DB 并入 proxy key 的凭据校验。
///
/// 违规（token_ref/token_digest 互斥冲突、摘要格式非法）的 DB 并入条目
/// 仅剔除该条并 warn，不阻塞启动；YAML 条目违规由 load 阶段 fail fast 保证。
pub async fn merge_db_config(
    config: &mut GatewayConfig,
    repo: &SqliteRequestEventRepository,
) -> Result<MergeReport, StoreError> {
    let (resources, mcp_servers, proxy_keys) = load_db_entries(repo).await?;
    let report = merge_db_into_config(config, resources, mcp_servers, proxy_keys);
    for row in &report.shadowed {
        warn!(row = %row, "db config row shadowed by yaml entry (yaml wins)");
    }
    for row in &report.skipped_invalid {
        warn!(row = %row, "db config row skipped: invalid config/scope json");
    }
    info!(
        resources = report.added_resources.len(),
        mcp_servers = report.added_mcp_servers.len(),
        proxy_keys = report.added_proxy_keys.len(),
        "db config entries merged"
    );

    let added: HashSet<&str> = report.added_proxy_keys.iter().map(String::as_str).collect();
    // 单 key 探针：换入待检 key 后跑真实校验，不复制校验逻辑
    let mut probe = config.clone();
    config.proxy_keys.retain(|key| {
        if !added.contains(key.id.as_str()) {
            return true;
        }
        probe.proxy_keys = vec![key.clone()];
        match probe.validate_key_credentials() {
            Ok(()) => true,
            Err(error) => {
                warn!(proxy_key_id = %key.id, error = %error,
                    "dropping db-merged proxy key with invalid credentials");
                false
            }
        }
    });
    Ok(report)
}

/// 把 DB 行并入 YAML 配置：同 id YAML 胜，DB 独有条目按 id 升序追加。
///
/// 纯函数：不做 IO，可直接单测。坏行跳过并 warn（字段 entity/id/error，
/// 不含 config_json 原文）。
pub fn merge_db_into_config(
    config: &mut GatewayConfig,
    resources: Vec<Resource>,
    mcp_servers: Vec<McpServerRecord>,
    proxy_keys: Vec<ProxyKeyRecord>,
) -> MergeReport {
    let mut report = MergeReport::default();

    // ── resources ──
    let mut rows = resources;
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    for row in rows {
        if config.api_resources.iter().any(|r| r.id == row.id) {
            report.shadowed.push(format!("resource:{}", row.id));
            continue;
        }
        match resource_from_record(&row) {
            Ok(resource) => {
                config.api_resources.push(resource);
                report.added_resources.push(row.id);
            }
            Err(error) => {
                warn!(entity = "resource", id = %row.id, error = %error,
                    "skipping db config row with invalid config_json");
                report.skipped_invalid.push(format!("resource:{}", row.id));
            }
        }
    }

    // ── mcp servers ──
    let mut rows = mcp_servers;
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    for row in rows {
        if config.mcp_servers.iter().any(|s| s.id == row.id) {
            report.shadowed.push(format!("mcp_server:{}", row.id));
            continue;
        }
        match mcp_server_from_record(&row) {
            Ok(server) => {
                config.mcp_servers.push(server);
                report.added_mcp_servers.push(row.id);
            }
            Err(error) => {
                warn!(entity = "mcp_server", id = %row.id, error = %error,
                    "skipping db config row with invalid config_json");
                report
                    .skipped_invalid
                    .push(format!("mcp_server:{}", row.id));
            }
        }
    }

    // ── proxy keys ──
    let mut rows = proxy_keys;
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    for row in rows {
        if config.proxy_keys.iter().any(|k| k.id == row.id) {
            report.shadowed.push(format!("proxy_key:{}", row.id));
            continue;
        }
        match proxy_key_from_record(&row) {
            Ok(key) => {
                config.proxy_keys.push(key);
                report.added_proxy_keys.push(row.id);
            }
            Err(error) => {
                warn!(entity = "proxy_key", id = %row.id, error = %error,
                    "skipping db config row with invalid scope_json");
                report.skipped_invalid.push(format!("proxy_key:{}", row.id));
            }
        }
    }

    report
}

// ── 反向映射：record → config（写方向见 admin/crud.rs 与 admin/mcp.rs）──

/// `resources.config_json` 载荷（`to_db_resource` 的镜像；未知字段忽略）。
#[derive(Deserialize)]
struct ResourceConfigJson {
    #[serde(default)]
    limits: Option<UpstreamLimits>,
}

fn resource_from_record(row: &Resource) -> Result<ApiResource, serde_json::Error> {
    let extra: ResourceConfigJson = serde_json::from_str(&row.config_json)?;
    // CRUD 写路径未持久化的字段取与 api_resource_from_input 相同的缺省值
    Ok(ApiResource {
        id: row.id.clone(),
        domain: row.domain.clone(),
        provider: row.provider.clone(),
        base_url: row.base_url.clone(),
        description: row.description.clone().unwrap_or_default(),
        auth: UpstreamAuth::None,
        key_pool: None,
        endpoints: Vec::new(),
        discovery: None,
        security: SecurityConfig::default(),
        limits: extra.limits,
    })
}

/// `mcp_servers.config_json` 载荷（`admin/mcp.rs` `to_db_record` 的镜像）。
#[derive(Deserialize)]
struct McpServerConfigJson {
    #[serde(default)]
    auth: UpstreamAuth,
    #[serde(default)]
    security: SecurityConfig,
    #[serde(default)]
    limits: Option<UpstreamLimits>,
    #[serde(default)]
    health_check: HealthCheckConfig,
}

fn mcp_server_from_record(row: &McpServerRecord) -> Result<McpServerConfig, serde_json::Error> {
    let extra: McpServerConfigJson = serde_json::from_str(&row.config_json)?;
    Ok(McpServerConfig {
        id: row.id.clone(),
        domain: row.domain.clone(),
        provider: row.provider.clone(),
        url: row.url.clone(),
        description: row.description.clone().unwrap_or_default(),
        auth: extra.auth,
        security: extra.security,
        health_check: extra.health_check,
        limits: extra.limits,
    })
}

/// `proxy_keys.scope_json` 载荷（`to_db_proxy_key` 的镜像；往返测试见
/// `admin/crud.rs` tests）。凭据以 ref/摘要形态入库，无明文 token。
#[derive(Deserialize)]
struct ProxyKeyScopeJson {
    #[serde(default)]
    allowed_tools: Vec<String>,
    #[serde(default)]
    denied_tools: Vec<String>,
    #[serde(default)]
    allowed_servers: Vec<String>,
    #[serde(default)]
    allowed_tool_names: Vec<String>,
    #[serde(default)]
    limits: Option<KeyLimits>,
    #[serde(default)]
    token_ref: Option<String>,
    #[serde(default)]
    token_digest: Option<String>,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    discovery_mode: Option<String>,
    #[serde(default)]
    response_format: Option<ResponseFormat>,
}

fn proxy_key_from_record(row: &ProxyKeyRecord) -> Result<ProxyKey, serde_json::Error> {
    let scope: ProxyKeyScopeJson = serde_json::from_str(&row.scope_json)?;
    Ok(ProxyKey {
        id: row.id.clone(),
        display_name: row.display_name.clone(),
        allowed_tools: scope.allowed_tools,
        denied_tools: scope.denied_tools,
        allowed_servers: scope.allowed_servers,
        allowed_tool_names: scope.allowed_tool_names,
        limits: scope.limits,
        // 凭据字段随 scope_json 回读（签发落库后重启不丢；旧行缺字段回 None）
        token_ref: scope.token_ref,
        token_digest: scope.token_digest,
        expires_at: scope.expires_at,
        // 负值视为脏数据，回退 CRUD 缺省页大小
        default_tool_page_size: usize::try_from(row.default_tool_page_size).unwrap_or(20),
        discovery_mode: scope.discovery_mode,
        response_format: scope.response_format,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{in_memory_pool, run_migrations};

    fn base_config(yaml: &str) -> GatewayConfig {
        serde_norway::from_str(yaml).expect("valid test yaml")
    }

    fn db_resource(id: &str, config_json: &str) -> Resource {
        Resource {
            id: id.to_string(),
            domain: "search".to_string(),
            provider: id.to_string(),
            base_url: format!("https://{id}.example.com"),
            description: Some(format!("{id} from db")),
            config_json: config_json.to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn db_mcp(id: &str, config_json: &str) -> McpServerRecord {
        McpServerRecord {
            id: id.to_string(),
            domain: "search".to_string(),
            provider: id.to_string(),
            url: format!("https://{id}.example.com/mcp"),
            description: Some(format!("{id} from db")),
            config_json: config_json.to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn db_key(id: &str, scope_json: &str) -> ProxyKeyRecord {
        ProxyKeyRecord {
            id: id.to_string(),
            display_name: format!("key-{id}"),
            default_tool_page_size: 50,
            scope_json: scope_json.to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn db_only_entries_appended_in_id_order() {
        let mut config = base_config(
            r#"
api_resources:
  - id: aaa
    domain: search
    base_url: https://aaa.example.com
"#,
        );
        // 乱序传入，验证按 id 升序追加
        let report = merge_db_into_config(
            &mut config,
            vec![db_resource("zz", "{}"), db_resource("mm", "{}")],
            vec![db_mcp("beta", "{}"), db_mcp("alpha", "{}")],
            vec![db_key("k2", "{}"), db_key("k1", "{}")],
        );

        let resource_ids: Vec<&str> = config.api_resources.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(resource_ids, vec!["aaa", "mm", "zz"]);
        assert_eq!(report.added_resources, vec!["mm", "zz"]);

        let server_ids: Vec<&str> = config.mcp_servers.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(server_ids, vec!["alpha", "beta"]);
        assert_eq!(report.added_mcp_servers, vec!["alpha", "beta"]);

        let key_ids: Vec<&str> = config.proxy_keys.iter().map(|k| k.id.as_str()).collect();
        assert_eq!(key_ids, vec!["k1", "k2"]);
        assert_eq!(report.added_proxy_keys, vec!["k1", "k2"]);

        assert!(report.shadowed.is_empty());
        assert!(report.skipped_invalid.is_empty());
    }

    #[test]
    fn yaml_wins_on_id_conflict_across_all_entities() {
        let mut config = base_config(
            r#"
api_resources:
  - id: tavily
    domain: search
    base_url: https://yaml.example.com
mcp_servers:
  - id: exa
    domain: search
    provider: exa
    url: https://yaml.example.com/mcp
proxy_keys:
  - id: agent-a
    display_name: yaml-name
"#,
        );
        let report = merge_db_into_config(
            &mut config,
            vec![db_resource("tavily", "{}")],
            vec![db_mcp("exa", "{}")],
            vec![db_key("agent-a", "{}")],
        );

        // YAML 条目原样保留
        assert_eq!(config.api_resources.len(), 1);
        assert_eq!(config.api_resources[0].base_url, "https://yaml.example.com");
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers[0].url, "https://yaml.example.com/mcp");
        assert_eq!(config.proxy_keys.len(), 1);
        assert_eq!(config.proxy_keys[0].display_name, "yaml-name");

        assert_eq!(
            report.shadowed,
            vec!["resource:tavily", "mcp_server:exa", "proxy_key:agent-a"]
        );
        assert!(report.added_resources.is_empty());
        assert!(report.added_mcp_servers.is_empty());
        assert!(report.added_proxy_keys.is_empty());
    }

    #[test]
    fn invalid_json_rows_skipped_others_merged() {
        let mut config = base_config("api_resources: []");
        let report = merge_db_into_config(
            &mut config,
            vec![db_resource("bad", "not json"), db_resource("good", "{}")],
            vec![db_mcp("bad-mcp", "["), db_mcp("good-mcp", "{}")],
            vec![db_key("bad-key", "42"), db_key("good-key", "{}")],
        );

        assert_eq!(report.added_resources, vec!["good"]);
        assert_eq!(report.added_mcp_servers, vec!["good-mcp"]);
        assert_eq!(report.added_proxy_keys, vec!["good-key"]);
        assert_eq!(
            report.skipped_invalid,
            vec!["resource:bad", "mcp_server:bad-mcp", "proxy_key:bad-key"]
        );
        assert_eq!(config.api_resources.len(), 1);
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.proxy_keys.len(), 1);
    }

    #[test]
    fn reverse_mapping_restores_persisted_fields() {
        let mut config = base_config("api_resources: []");
        let report = merge_db_into_config(
            &mut config,
            vec![db_resource("r1", r#"{"limits":{"rps":10,"rpm":300}}"#)],
            vec![db_mcp(
                "m1",
                r#"{"auth":{"type":"bearer","token_ref":"secret://env/M1"},
                    "security":{"defense":{"enabled":true}},
                    "limits":{"rps":5},
                    "health_check":{"enabled":false}}"#,
            )],
            vec![db_key(
                "k1",
                r#"{"allowed_tools":["search.*"],"denied_tools":["admin.*"],
                    "allowed_servers":["m1"],"allowed_tool_names":["search__m1__go"],
                    "limits":{"rpm":60,"max_calls":100,"max_calls_per_day":10}}"#,
            )],
        );
        assert!(report.skipped_invalid.is_empty());

        let r = config.resource("r1").expect("resource merged");
        assert_eq!(r.description, "r1 from db");
        let limits = r.limits.as_ref().expect("limits");
        assert_eq!(limits.rps, Some(10));
        assert_eq!(limits.rpm, Some(300));
        assert!(r.auth.is_none());

        let m = config.mcp_server("m1").expect("mcp merged");
        assert_eq!(m.auth.bearer_ref(), Some("secret://env/M1"));
        assert!(m.security.defense.enabled);
        assert!(!m.health_check.enabled);
        assert_eq!(m.limits.as_ref().and_then(|l| l.rps), Some(5));

        let k = config.proxy_key("k1").expect("key merged");
        assert_eq!(k.display_name, "key-k1");
        assert_eq!(k.default_tool_page_size, 50);
        assert_eq!(k.allowed_tools, vec!["search.*"]);
        assert_eq!(k.denied_tools, vec!["admin.*"]);
        assert_eq!(k.allowed_servers, vec!["m1"]);
        assert_eq!(k.allowed_tool_names, vec!["search__m1__go"]);
        let kl = k.limits.as_ref().expect("key limits");
        assert_eq!(kl.rpm, Some(60));
        assert_eq!(kl.max_calls, Some(100));
        assert_eq!(kl.max_calls_per_day, Some(10));
        assert!(k.token_ref.is_none());
        assert!(k.token_digest.is_none());
        assert!(k.expires_at.is_none());
    }

    #[test]
    fn negative_page_size_falls_back_to_default() {
        let mut config = base_config("proxy_keys: []");
        let mut row = db_key("k1", "{}");
        row.default_tool_page_size = -1;
        merge_db_into_config(&mut config, Vec::new(), Vec::new(), vec![row]);
        assert_eq!(config.proxy_keys[0].default_tool_page_size, 20);
    }

    #[test]
    fn merge_before_expand_lets_db_entry_shadow_preset() {
        // 合并发生在 expand_builtin_mcp 前：DB 有 preset 同 id 条目时，
        // 展开按「显式同 id 条目优先」自然跳过该 preset。
        let mut config = base_config("builtin_mcp: [exa]");
        let report = merge_db_into_config(
            &mut config,
            Vec::new(),
            vec![db_mcp("exa", "{}")],
            Vec::new(),
        );
        assert_eq!(report.added_mcp_servers, vec!["exa"]);

        config.expand_builtin_mcp().expect("expand");
        assert_eq!(config.mcp_servers.len(), 1, "preset 不重复展开");
        assert_eq!(config.mcp_servers[0].url, "https://exa.example.com/mcp");
    }

    #[tokio::test]
    async fn merge_db_config_drops_invalid_credential_keys_only() {
        let pool = in_memory_pool().await.expect("pool");
        run_migrations(&pool).await.expect("migrations");
        let repo = SqliteRequestEventRepository::new(pool);

        // 违规：token_ref 与 token_digest 互斥冲突（模拟手改 DB 的脏数据）
        repo.insert_proxy_key(&db_key(
            "bad-key",
            r#"{"token_ref":"secret://env/T","token_digest":"deadbeef"}"#,
        ))
        .await
        .expect("insert bad key");
        repo.insert_proxy_key(&db_key("good-key", "{}"))
            .await
            .expect("insert good key");

        let mut config = base_config("proxy_keys:\n  - id: yaml-key\n");
        let report = merge_db_config(&mut config, &repo)
            .await
            .expect("merge db config");
        assert_eq!(report.added_proxy_keys, vec!["bad-key", "good-key"]);

        // 违规 DB 条目被剔除；YAML 条目与合法 DB 条目保留
        let ids: Vec<&str> = config.proxy_keys.iter().map(|k| k.id.as_str()).collect();
        assert_eq!(ids, vec!["yaml-key", "good-key"]);
    }

    #[tokio::test]
    async fn load_db_entries_reads_all_three_tables() {
        let pool = in_memory_pool().await.expect("pool");
        run_migrations(&pool).await.expect("migrations");
        let repo = SqliteRequestEventRepository::new(pool);

        repo.insert_resource(&db_resource("r1", "{}"))
            .await
            .expect("insert resource");
        repo.insert_mcp_server(&db_mcp("m1", "{}"))
            .await
            .expect("insert mcp server");
        repo.insert_proxy_key(&db_key("k1", "{}"))
            .await
            .expect("insert proxy key");

        let (resources, mcp_servers, proxy_keys) =
            load_db_entries(&repo).await.expect("load entries");
        assert_eq!(resources.len(), 1);
        assert_eq!(mcp_servers.len(), 1);
        assert_eq!(proxy_keys.len(), 1);

        let mut config = base_config("api_resources: []");
        let report = merge_db_into_config(&mut config, resources, mcp_servers, proxy_keys);
        assert_eq!(report.added_resources, vec!["r1"]);
        assert_eq!(report.added_mcp_servers, vec!["m1"]);
        assert_eq!(report.added_proxy_keys, vec!["k1"]);
    }
}
