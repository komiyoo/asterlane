//! `AppState` 的启动期恢复与后台刷新编排。
//!
//! 进程入口（`main.rs`）装配完成后调用这里的函数：入口只保留后台任务的循环骨架，
//! 每个 tick 做的事收敛为一次调用。

use tracing::{info, warn};

use super::AppState;
use crate::store::ToolMetadataRepository;

impl AppState {
    /// 启动期从 store 全量加载工具介绍 override 进 catalog overlay
    /// （agent 可见描述 = override ?? 上游原始，见 docs/runtime/mcp-governance-and-key-limits.md §5）。
    ///
    /// 无 store 时跳过；读取失败只告警，不阻断启动。
    pub async fn load_description_overrides(&self) {
        let Some(repo) = &self.event_repo else {
            return;
        };
        match repo.list_tool_metadata().await {
            Ok(rows) if !rows.is_empty() => {
                let count = rows.len();
                let overrides = rows
                    .into_iter()
                    .map(|row| (row.tool_name, row.description))
                    .collect();
                self.catalog
                    .write()
                    .await
                    .load_description_overrides(overrides);
                info!(count, "tool description overrides loaded");
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "failed to load tool description overrides"),
        }
    }

    /// 单次 MCP registry 刷新（后台 task 的一个 tick）。
    ///
    /// `reason` 标记触发源（`interval` / `upstream_list_changed`），`servers` 为
    /// 即时触发时上游通知的 server id（周期触发为 `None`）。步骤：
    /// 1. `registry.refresh_with_secrets()` 重新拉取上游 `tools/list`
    ///    （unreachable 的 server 用 secrets 自动重连，恢复后并入其工具）。
    /// 2. `catalog.replace_mcp_tools()` 更新工具快照。
    /// 3. `check_drift()` 检测 drift → 写 security event → 更新隔离集合 → rebase baseline。
    /// 4. `notify_peers_tool_list_changed()` 向 legacy session 与
    ///    `subscriptions/listen` 通道推送通知。
    ///
    /// 未注入 MCP registry 时直接返回。
    pub async fn refresh_mcp_tools(&self, reason: &'static str, servers: Option<&[String]>) {
        let Some(registry) = &self.mcp_registry else {
            return;
        };
        let result = registry.refresh_with_secrets(self.secrets.as_ref()).await;
        info!(
            reason,
            servers = ?servers,
            old_count = result.old_tool_count,
            new_count = result.new_tool_count,
            failed_servers = ?result.failed_server_ids,
            "mcp registry refreshed"
        );
        if !result.failed_server_ids.is_empty() {
            warn!(
                servers = ?result.failed_server_ids,
                "some mcp servers failed during refresh"
            );
        }

        let new_tools = registry.all_wrapped_tools();
        let mcp_ids = registry.mcp_resource_ids();
        self.catalog
            .write()
            .await
            .replace_mcp_tools(new_tools, &mcp_ids);

        let config = self.config_snapshot().await;
        crate::integrity::check_drift(
            registry,
            &config,
            &self.integrity_baseline,
            &self.quarantined_tools,
            &self.event_repo,
        )
        .await;

        crate::mcp::notify_peers_tool_list_changed(&self.tool_list_changed_peers).await;
    }
}
