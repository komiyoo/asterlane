//! MCP server 健康模型与运行期增删改（契约见 docs/mcp-governance-and-key-limits.md §4）。
//!
//! 状态机：`ok`（最近一次探测成功）| `unreachable`（最近一次探测失败）|
//! `unknown`（尚未探测）| `disabled`（`health_check.enabled: false`，不参与
//! 周期探测；按需 `probe` 仍可用，但状态恒为 `disabled`）。
//!
//! 探测口径：未连接时先连接 + `tools/list`（latency 覆盖两步）；已连接时仅
//! `tools/list`，与 refresh 同口径。健康簿记与 entries 同一把锁，全部在
//! 同步临界区内完成——不持锁跨 await，读路径（`call_tool` /
//! `all_wrapped_tools`）成本不变。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::{debug, warn};

use crate::config::McpServerConfig;
use crate::mcp::error::McpError;
use crate::mcp::registry::{
    McpServerEntry, McpServerRegistry, PeerConnector, RefreshResult, RemoteMcpPeer,
    transport_config, wrap_tools,
};
use crate::secrets::{SecretError, SecretRef, SecretStore, SecretString};

/// MCP server 健康状态（serde 小写，供 wave 2 admin JSON 直接输出）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum HealthStatus {
    /// 最近一次探测成功。
    Ok,
    /// 最近一次探测（含连接）失败。
    Unreachable,
    /// 尚未探测。
    Unknown,
    /// `health_check.enabled: false`，不参与周期探测。
    Disabled,
}

/// 单个 MCP server 的健康快照（契约字段，wave 2 admin 端点直接消费）。
#[derive(Debug, Clone, Serialize)]
pub struct ServerHealth {
    pub server_id: String,
    pub status: HealthStatus,
    /// 最近一次探测时间（无论成败）。
    pub last_check_at: Option<DateTime<Utc>>,
    /// 最近一次探测成功时间。
    pub last_ok_at: Option<DateTime<Utc>>,
    /// 最近一次成功探测耗时（重连路径含连接耗时）；失败不覆盖。
    pub latency_ms: Option<u64>,
    /// 连续失败次数，探测成功后清零。
    pub consecutive_failures: u32,
    /// 最近一次失败的脱敏 message（`McpError` Display 不含凭据）。
    pub last_error: Option<String>,
    /// 当前工具快照数量（上游不可达时为 stale 快照数）。
    pub tool_count: usize,
}

impl ServerHealth {
    /// 初始健康态：disabled 配置为 `Disabled`，其余为 `Unknown`（尚未探测）。
    pub(super) fn initial(config: &McpServerConfig) -> Self {
        let status = if config.health_check.enabled {
            HealthStatus::Unknown
        } else {
            HealthStatus::Disabled
        };
        Self {
            server_id: config.id.clone(),
            status,
            last_check_at: None,
            last_ok_at: None,
            latency_ms: None,
            consecutive_failures: 0,
            last_error: None,
            tool_count: 0,
        }
    }
}

/// `refresh()`（无 secrets 变体）用的占位 SecretStore：该路径下无 peer 的
/// entry 直接跳过重连，因此永不会被调用。
#[derive(Debug)]
struct NoSecrets;

impl SecretStore for NoSecrets {
    fn resolve(
        &self,
        _secret_ref: &SecretRef,
    ) -> impl std::future::Future<Output = Result<SecretString, SecretError>> + Send {
        std::future::ready(Err(SecretError::not_found(
            "secret://none/refresh-without-secrets",
        )))
    }
}

impl McpServerRegistry {
    /// 所有 server 的健康快照（`tool_count` 以当前工具快照实时计算）。
    pub fn health_snapshot(&self) -> Vec<ServerHealth> {
        self.read_entries()
            .iter()
            .map(McpServerEntry::health_view)
            .collect()
    }

    /// 重新对每个上游调 `list_tools`，更新工具快照与健康态。
    ///
    /// 无 secrets 变体：无 peer（启动/重连失败）的 entry 跳过重连，只计入
    /// `failed_server_ids`，健康态不变（未实际探测不累计失败）。wave 2 把
    /// main.rs 调用点切到 [`Self::refresh_with_secrets`] 后再收敛。
    pub async fn refresh(&self) -> RefreshResult {
        self.refresh_inner(None::<&NoSecrets>).await
    }

    /// 同 [`Self::refresh`]，另对无 peer 的 entry 用 `secrets` 重连
    /// （探测 = 连接 + `tools/list`），成功后自动转 `ok` 并合并其工具。
    pub async fn refresh_with_secrets<S: SecretStore>(&self, secrets: &S) -> RefreshResult {
        self.refresh_inner(Some(secrets)).await
    }

    /// refresh 公共实现。
    ///
    /// 流程：读锁内 clone 快照 → 释放锁 → 逐个探测/重连 → 写锁内按 id
    /// 合并。不持锁跨 await。
    ///
    /// - `disabled` 的 server 跳过探测，工具沿用 stale 快照，状态恒 `disabled`；
    /// - 上游不可达或包装失败时保留上一次成功快照（stale cache），不污染
    ///   integrity baseline；`RefreshResult.failed_server_ids` 记录失败上游；
    /// - wire name 跨 server 去重：重复的工具跳过并告警，不中断整体刷新。
    async fn refresh_inner<S: SecretStore>(&self, secrets: Option<&S>) -> RefreshResult {
        // 读锁内 clone entry 快照（不持锁跨 await）
        let snapshots: Vec<McpServerEntry> = self.read_entries().iter().cloned().collect();

        let mut refreshed_entries = Vec::with_capacity(snapshots.len());
        let mut seen_wire_names = HashSet::new();
        let mut failed_ids = Vec::new();
        let mut total_tools = 0usize;

        for snapshot in snapshots {
            // disabled：跳过探测，工具沿用 stale 快照
            if !snapshot.config.health_check.enabled {
                total_tools +=
                    push_deduped_entry(snapshot, &mut refreshed_entries, &mut seen_wire_names);
                continue;
            }
            let server_id = snapshot.config.id.clone();
            let refreshed = match (snapshot.peer.clone(), secrets) {
                // 已连接：直接探测（失败保留 stale 快照）
                (Some(peer), _) => probe_entry(snapshot, peer, Instant::now()).await,
                // 无 peer 且有 secrets：重连 + 探测
                (None, Some(secrets)) => {
                    establish_entry(
                        snapshot.config,
                        Some(snapshot.health),
                        secrets,
                        self.connector.as_ref(),
                    )
                    .await
                }
                // 无 peer 且无 secrets：跳过重连，只记失败
                (None, None) => {
                    failed_ids.push(server_id);
                    total_tools +=
                        push_deduped_entry(snapshot, &mut refreshed_entries, &mut seen_wire_names);
                    continue;
                }
            };
            if refreshed.health.status == HealthStatus::Unreachable {
                failed_ids.push(server_id);
            }
            total_tools +=
                push_deduped_entry(refreshed, &mut refreshed_entries, &mut seen_wire_names);
        }

        // 写锁内按 id 合并（不跨 await）：refresh 期间被并发 add 的 entry
        // 不丢弃、被并发 remove 的不复活。
        let old_count = {
            let mut guard = self.write_entries();
            let old = guard.iter().map(|e| e.tools.len()).sum::<usize>();
            for refreshed in refreshed_entries {
                if let Some(pos) = guard
                    .iter()
                    .position(|e| e.config.id == refreshed.config.id)
                {
                    guard[pos] = refreshed;
                }
            }
            old
        };

        RefreshResult {
            old_tool_count: old_count,
            new_tool_count: total_tools,
            failed_server_ids: failed_ids,
        }
    }

    /// 立即探测单个 server：未连接则先用 `secrets` 建连，成功后更新健康态
    /// 与工具快照，返回最新健康态。unknown id 返回 [`McpError::UnknownServer`]。
    pub async fn probe<S: SecretStore>(
        &self,
        server_id: &str,
        secrets: &S,
    ) -> Result<ServerHealth, McpError> {
        let snapshot = self
            .read_entries()
            .iter()
            .find(|e| e.config.id == server_id)
            .cloned()
            .ok_or_else(|| McpError::unknown_server(server_id))?;
        let refreshed = match snapshot.peer.clone() {
            Some(peer) => probe_entry(snapshot, peer, Instant::now()).await,
            None => {
                establish_entry(
                    snapshot.config,
                    Some(snapshot.health),
                    secrets,
                    self.connector.as_ref(),
                )
                .await
            }
        };
        // 探测期间被并发 remove 时不复活该 entry
        self.replace_entry(refreshed)
            .ok_or_else(|| McpError::unknown_server(server_id))
    }

    /// 登记新 server 并尝试连接 + 拉取工具。连接失败仍登记
    /// （`unreachable`，无 peer）并返回其健康态，不算 `Err`；
    /// id 已存在返回 `mcp.invalid_tool_call`。
    pub async fn add_server<S: SecretStore>(
        &self,
        config: McpServerConfig,
        secrets: &S,
    ) -> Result<ServerHealth, McpError> {
        let exists = self.read_entries().iter().any(|e| e.config.id == config.id);
        if exists {
            return Err(duplicate_server(&config.id));
        }
        let mut entry = establish_entry(config, None, secrets, self.connector.as_ref()).await;
        let mut guard = self.write_entries();
        // 连接期间可能已有同 id 并发登记
        if guard.iter().any(|e| e.config.id == entry.config.id) {
            return Err(duplicate_server(&entry.config.id));
        }
        dedup_against_others(&guard, None, &mut entry);
        let view = entry.health_view();
        guard.push(entry);
        Ok(view)
    }

    /// 替换 server 配置。url/auth 变化（或原本无连接）时重连，否则复用
    /// 既有连接重新拉取工具（domain/provider 变化会改变 wire name，需重算）。
    /// 重连失败仍保留 entry（`unreachable`）并返回健康态；unknown id 返回错误。
    pub async fn update_server<S: SecretStore>(
        &self,
        config: McpServerConfig,
        secrets: &S,
    ) -> Result<ServerHealth, McpError> {
        let server_id = config.id.clone();
        let old = self
            .read_entries()
            .iter()
            .find(|e| e.config.id == server_id)
            .cloned()
            .ok_or_else(|| McpError::unknown_server(&server_id))?;
        let needs_reconnect =
            old.peer.is_none() || old.config.url != config.url || old.config.auth != config.auth;
        let refreshed = match (old.peer.clone(), needs_reconnect) {
            (Some(peer), false) => {
                let entry = McpServerEntry {
                    config,
                    peer: Some(peer.clone()),
                    tools: old.tools,
                    descriptors: old.descriptors,
                    health: old.health,
                };
                probe_entry(entry, peer, Instant::now()).await
            }
            _ => establish_entry(config, Some(old.health), secrets, self.connector.as_ref()).await,
        };
        self.replace_entry(refreshed)
            .ok_or_else(|| McpError::unknown_server(&server_id))
    }

    /// 移除 server（entry 连同工具快照），返回是否存在。
    pub fn remove_server(&self, server_id: &str) -> bool {
        let mut guard = self.write_entries();
        let before = guard.len();
        guard.retain(|e| e.config.id != server_id);
        guard.len() != before
    }

    /// 写锁内按 id 替换 entry（先做跨 server wire name 去重）。
    /// id 不存在时（并发 remove）返回 `None`，不重新插入。
    fn replace_entry(&self, mut entry: McpServerEntry) -> Option<ServerHealth> {
        let mut guard = self.write_entries();
        let pos = guard.iter().position(|e| e.config.id == entry.config.id)?;
        dedup_against_others(&guard, Some(pos), &mut entry);
        let view = entry.health_view();
        guard[pos] = entry;
        Some(view)
    }
}

fn duplicate_server(server_id: &str) -> McpError {
    McpError::invalid_tool_call(format!("MCP server already registered: {server_id}"))
}

/// 连接 + `tools/list` + 包装，构造完整 entry。任何一步失败都降级为
/// 无 peer（工具为空）+ `unreachable` 健康态的 entry，绝不返回 `Err`
/// （降级启动契约）。`prev_health` 用于延续 `last_ok_at` 与失败计数。
pub(super) async fn establish_entry<S: SecretStore>(
    config: McpServerConfig,
    prev_health: Option<ServerHealth>,
    secrets: &S,
    connector: &dyn PeerConnector,
) -> McpServerEntry {
    let mut health = prev_health.unwrap_or_else(|| ServerHealth::initial(&config));
    let enabled = config.health_check.enabled;
    let started = Instant::now();
    let connected = match transport_config(&config, secrets).await {
        Ok(transport) => connector.connect(&config, transport).await,
        Err(e) => Err(e),
    };
    match connected {
        Ok(peer) => {
            let entry = McpServerEntry {
                config,
                peer: None,
                tools: Vec::new(),
                descriptors: Vec::new(),
                health,
            };
            probe_entry(entry, peer, started).await
        }
        Err(e) => {
            warn!(
                server_id = %config.id,
                status = "unreachable",
                error = %e,
                "MCP server 连接失败，登记为 unreachable"
            );
            mark_failed(&mut health, enabled, &e);
            McpServerEntry {
                config,
                peer: None,
                tools: Vec::new(),
                descriptors: Vec::new(),
                health,
            }
        }
    }
}

/// 用给定连接探测：`tools/list` + 重新包装。成功替换工具快照并记 `ok`；
/// 失败保留 stale 快照记 `unreachable`。`started` 供 latency 计时——重连
/// 路径传入连接前时刻，使耗时覆盖「连接 + tools/list」（契约口径）。
pub(super) async fn probe_entry(
    mut entry: McpServerEntry,
    peer: Arc<dyn RemoteMcpPeer>,
    started: Instant,
) -> McpServerEntry {
    let enabled = entry.config.health_check.enabled;
    let outcome = match peer.list_tools().await {
        Ok(upstream_tools) => wrap_tools(&entry.config, upstream_tools),
        Err(e) => Err(e),
    };
    match outcome {
        Ok((tools, descriptors)) => {
            let latency_ms = elapsed_ms(started);
            entry.tools = tools;
            entry.descriptors = descriptors;
            mark_ok(&mut entry.health, enabled, latency_ms, entry.tools.len());
            debug!(
                server_id = %entry.config.id,
                status = "ok",
                latency_ms,
                tool_count = entry.tools.len(),
                "MCP 探测成功"
            );
        }
        Err(e) => {
            warn!(
                server_id = %entry.config.id,
                status = "unreachable",
                error = %e,
                "MCP 探测失败，保留 stale 工具快照"
            );
            mark_failed(&mut entry.health, enabled, &e);
        }
    }
    entry.peer = Some(peer);
    entry
}

/// 探测成功：状态 `ok`（disabled 配置恒 `disabled`），失败计数清零。
pub(super) fn mark_ok(
    health: &mut ServerHealth,
    enabled: bool,
    latency_ms: u64,
    tool_count: usize,
) {
    let now = Utc::now();
    health.status = if enabled {
        HealthStatus::Ok
    } else {
        HealthStatus::Disabled
    };
    health.last_check_at = Some(now);
    health.last_ok_at = Some(now);
    health.latency_ms = Some(latency_ms);
    health.consecutive_failures = 0;
    health.last_error = None;
    health.tool_count = tool_count;
}

/// 探测失败：状态 `unreachable`（disabled 配置恒 `disabled`），失败计数
/// 累加；`latency_ms` / `last_ok_at` 保留最近成功值。`last_error` 只存
/// `McpError` 的脱敏 Display。
fn mark_failed(health: &mut ServerHealth, enabled: bool, error: &McpError) {
    health.status = if enabled {
        HealthStatus::Unreachable
    } else {
        HealthStatus::Disabled
    };
    health.last_check_at = Some(Utc::now());
    health.consecutive_failures = health.consecutive_failures.saturating_add(1);
    health.last_error = Some(error.to_string());
}

pub(super) fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// 把 entry 的工具按 `seen_wire_names` 去重后推入 `new_entries`，返回保留
/// 的工具数。重复 wire name 跳过并告警，不中断整体刷新。
pub(super) fn push_deduped_entry(
    mut entry: McpServerEntry,
    new_entries: &mut Vec<McpServerEntry>,
    seen_wire_names: &mut HashSet<String>,
) -> usize {
    dedup_entry_tools(&mut entry, seen_wire_names);
    let count = entry.tools.len();
    new_entries.push(entry);
    count
}

/// 与其他 entry 的既有 wire name 冲突（或自身重复）的工具跳过并告警，
/// 与 refresh 的去重口径一致。`skip` 为被替换 entry 自身的位置。
fn dedup_against_others(guard: &[McpServerEntry], skip: Option<usize>, entry: &mut McpServerEntry) {
    let mut seen: HashSet<String> = guard
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != skip)
        .flat_map(|(_, e)| e.tools.iter().map(|t| t.name.to_wire_name()))
        .collect();
    dedup_entry_tools(entry, &mut seen);
}

/// 就地去重：wire name 已在 `seen` 中的工具（含 entry 自身重复）丢弃并告警。
fn dedup_entry_tools(entry: &mut McpServerEntry, seen: &mut HashSet<String>) {
    let tools = std::mem::take(&mut entry.tools);
    let descriptors = std::mem::take(&mut entry.descriptors);
    let mut kept_tools = Vec::with_capacity(tools.len());
    let mut kept_descriptors = Vec::with_capacity(descriptors.len());
    for (tool, descriptor) in tools.into_iter().zip(descriptors) {
        let wire_name = tool.name.to_wire_name();
        if seen.insert(wire_name.clone()) {
            kept_tools.push(tool);
            kept_descriptors.push(descriptor);
        } else {
            warn!(
                wire_name = %wire_name,
                server_id = %entry.config.id,
                "duplicate wire name skipped"
            );
        }
    }
    entry.tools = kept_tools;
    entry.descriptors = kept_descriptors;
}
