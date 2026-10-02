//! 跨 server 的 wire name 去重：refresh、新增与替换 server 共用同一口径，
//! 重复的工具跳过并告警，不中断整体刷新。

use std::collections::HashSet;

use tracing::warn;

use crate::mcp::registry::McpServerEntry;

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
pub(super) fn dedup_against_others(
    guard: &[McpServerEntry],
    skip: Option<usize>,
    entry: &mut McpServerEntry,
) {
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
