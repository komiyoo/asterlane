//! 下游 `tools/list_changed` 通知：兼容 session 推送与 `subscriptions/listen`。

use std::sync::Arc;

use rmcp::model::{ProtocolVersion, SubscriptionFilter};
use rmcp::service::{RequestContext, SubscriptionContext, SubscriptionSink};
use rmcp::{Peer, RoleServer};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// 活跃下游通知通道，供后台 refresh 推送 `tools/list_changed`。
pub type ToolListChangedPeers = Arc<RwLock<Vec<ToolListChangedTarget>>>;

/// 一条可被后台 refresh 唤醒的下游通知通道。
#[derive(Debug, Clone)]
pub enum ToolListChangedTarget {
    /// `2025-11-25` 及更早：session 上的 `Peer::notify_tool_list_changed`。
    LegacySession(Peer<RoleServer>),
    /// `2026-07-28`：客户端 `subscriptions/listen` 接受的 sink。
    Subscription(SubscriptionSink),
}

/// 网关接受的订阅类别：只推工具目录变更。
pub fn accepted_tools_list_changed_filter(requested: &SubscriptionFilter) -> SubscriptionFilter {
    requested.intersection(&SubscriptionFilter::builder().tools_list_changed().build())
}

pub fn is_legacy_protocol(context: &RequestContext<RoleServer>) -> bool {
    context
        .protocol_version()
        .is_none_or(|version| version.as_str() < ProtocolVersion::V_2026_07_28.as_str())
}

/// 注册 session peer。同一 peer 身份只保留一条。
pub async fn register_legacy_peer(peers: &ToolListChangedPeers, peer: Peer<RoleServer>) {
    let key = format!("{peer:?}");
    let mut guard = peers.write().await;
    if guard.iter().any(|target| match target {
        ToolListChangedTarget::LegacySession(existing) => format!("{existing:?}") == key,
        ToolListChangedTarget::Subscription(_) => false,
    }) {
        return;
    }
    guard.push(ToolListChangedTarget::LegacySession(peer));
}

/// 注册 `subscriptions/listen` sink。同一 listen 请求 ID 只保留一条。
pub async fn register_subscription(peers: &ToolListChangedPeers, sink: SubscriptionSink) {
    let id = sink.id().clone();
    let mut guard = peers.write().await;
    if guard.iter().any(|target| match target {
        ToolListChangedTarget::Subscription(existing) => existing.id() == &id,
        ToolListChangedTarget::LegacySession(_) => false,
    }) {
        return;
    }
    guard.push(ToolListChangedTarget::Subscription(sink));
}

/// 挂起一条 listen 流，直到客户端取消。
pub async fn listen_tools_list_changed(peers: &ToolListChangedPeers, context: SubscriptionContext) {
    if context.accepted().tools_list_changed == Some(true) {
        register_subscription(peers, context.sink().clone()).await;
    }
    context.cancelled().await;
}

/// 向仍存活的下游通道发送 `notifications/tools/list_changed`。
///
/// 失败通道（session 关闭或 listen 结束）被移除。
pub async fn notify_peers_tool_list_changed(peers: &ToolListChangedPeers) {
    let mut guard = peers.write().await;
    let mut alive = Vec::with_capacity(guard.len());
    for target in guard.drain(..) {
        let result = match &target {
            ToolListChangedTarget::LegacySession(peer) => peer
                .notify_tool_list_changed()
                .await
                .map_err(|error| error.to_string()),
            ToolListChangedTarget::Subscription(sink) => sink
                .notify_tool_list_changed()
                .await
                .map_err(|error| error.to_string()),
        };
        match result {
            Ok(()) => {
                debug!("notified tools/list_changed to client session");
                alive.push(target);
            }
            Err(error) => {
                warn!(error = %error, "notify_tool_list_changed failed, dropping listener");
            }
        }
    }
    *guard = alive;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_filter_only_keeps_tools_list_changed() {
        let requested = SubscriptionFilter::builder()
            .tools_list_changed()
            .prompts_list_changed()
            .build();
        let accepted = accepted_tools_list_changed_filter(&requested);
        assert_eq!(accepted.tools_list_changed, Some(true));
        assert_ne!(accepted.prompts_list_changed, Some(true));
    }
}
