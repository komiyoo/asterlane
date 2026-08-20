//! 上游 `tools/list_changed`：session 回调与 `subscriptions/listen`。
//!
//! 收到通知后只投递 `server_id`；拉 `tools/list`、同步 catalog、向下游
//! 推送仍由 serve 后台 task 执行（与周期 refresh 共用）。轮询是兜底，
//! 多数托管 MCP（RollingGo / Exa）不一定会发变更通知。

use std::fmt::{Debug, Formatter};

use rmcp::handler::client::ClientHandler;
use rmcp::model::{ServerNotification, SubscriptionFilter};
use rmcp::service::{NotificationContext, Peer, RoleClient};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// 上游工具目录变更信号（只带 server id，不含工具内容）。
#[derive(Clone)]
pub struct UpstreamListChanged {
    tx: Option<mpsc::Sender<String>>,
}

impl Debug for UpstreamListChanged {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstreamListChanged")
            .field("active", &self.tx.is_some())
            .finish()
    }
}

impl Default for UpstreamListChanged {
    fn default() -> Self {
        Self::noop()
    }
}

impl UpstreamListChanged {
    /// 生产通道：handler / listen 投递，serve 后台 task 接收。
    pub fn channel() -> (Self, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(32);
        (Self { tx: Some(tx) }, rx)
    }

    /// 测试与无订阅路径：信号丢弃。
    pub fn noop() -> Self {
        Self { tx: None }
    }

    pub fn is_active(&self) -> bool {
        self.tx.is_some()
    }

    /// 非阻塞投递。队列满时丢弃（随后周期 refresh 仍会兜底）。
    pub fn signal(&self, server_id: &str) {
        let Some(tx) = &self.tx else {
            return;
        };
        match tx.try_send(server_id.to_string()) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                debug!(server_id, "upstream list_changed signal dropped (busy)");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }
}

/// 作为 rmcp client handler：legacy session 推送走回调。
#[derive(Clone, Debug)]
pub struct UpstreamNotifyHandler {
    server_id: String,
    notify: UpstreamListChanged,
}

impl UpstreamNotifyHandler {
    pub fn new(server_id: impl Into<String>, notify: UpstreamListChanged) -> Self {
        Self {
            server_id: server_id.into(),
            notify,
        }
    }
}

impl ClientHandler for UpstreamNotifyHandler {
    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        self.notify.signal(&self.server_id);
        std::future::ready(())
    }
}

/// 现代上游：`subscriptions/listen`。通知只进此流，不重复回调 handler。
///
/// 上游不支持 listen（`-32601` / HTTP 500）时安静返回，靠 session 回调与轮询。
pub fn spawn_listen_task(
    peer: Peer<RoleClient>,
    server_id: String,
    notify: UpstreamListChanged,
) -> tokio::task::AbortHandle {
    tokio::spawn(async move {
        let filter = SubscriptionFilter::builder().tools_list_changed().build();
        let mut subscription = match peer.listen(filter).await {
            Ok(subscription) => {
                info!(server_id = %server_id, "subscribed to upstream tools/list_changed");
                subscription
            }
            Err(error) => {
                debug!(
                    server_id = %server_id,
                    error = %error,
                    "upstream subscriptions/listen unavailable; session notify + poll remain"
                );
                return;
            }
        };
        loop {
            match subscription.next().await {
                Ok(Some(ServerNotification::ToolListChangedNotification(_))) => {
                    notify.signal(&server_id);
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    debug!(server_id = %server_id, "upstream list_changed listen ended");
                    break;
                }
                Err(error) => {
                    warn!(
                        server_id = %server_id,
                        error = %error,
                        "upstream list_changed listen failed"
                    );
                    break;
                }
            }
        }
    })
    .abort_handle()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_does_not_signal() {
        let notify = UpstreamListChanged::noop();
        assert!(!notify.is_active());
        notify.signal("srv");
    }

    #[tokio::test]
    async fn signal_reaches_receiver() {
        let (notify, mut rx) = UpstreamListChanged::channel();
        notify.signal("rollinggo-hotel");
        assert_eq!(rx.recv().await.as_deref(), Some("rollinggo-hotel"));
    }

    #[tokio::test]
    async fn handler_signals_server_id() {
        let (notify, mut rx) = UpstreamListChanged::channel();
        let handler = UpstreamNotifyHandler::new("exa", notify);
        // NotificationContext 构造成本高；直接测 signal 契约，handler 只是转发。
        handler.notify.signal(&handler.server_id);
        assert_eq!(rx.recv().await.as_deref(), Some("exa"));
    }
}
