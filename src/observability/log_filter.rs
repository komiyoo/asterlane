//! 日志过滤的固定上限：不管 `RUST_LOG` 怎么设，凭据相关的第三方日志都不会打印
//! 授权 code 或 token。
//!
//! rmcp 3.1.2 的 OAuth 实现（target `rmcp::transport::auth`）在 `debug` 级会打印：
//! - 用 code 换 token 时 `start exchange code for token: "<code>"`（授权 code 明文）；
//! - 换到 token 后 `exchange token result: {:?}`，以及 client-credentials 的
//!   `client credentials token result: {:?}`。access / refresh token 在 `Debug` 里是
//!   `[redacted]`，但 token 响应里的非标准字段（例如 `id_token`）会按原值输出。
//!
//! 所以给这个 target 固定一条 `info` 级上限：它的 `info` 及以上（例如「Refreshed access
//! token」）照常输出，`debug` 与 `trace` 一律丢弃。上限做成与 `RUST_LOG` 的 `EnvFilter`
//! 叠加的全局过滤层：两者都放行才输出，所以 `RUST_LOG=debug` 或 `rmcp=trace` 都抬不高
//! 它，而 `RUST_LOG=warn` 这类更严的设置照常生效。

use tracing::{Level, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::filter_fn;

/// 受上限约束的 target 前缀：rmcp 的 OAuth 实现及其接入 HTTP client 的适配。
const CAPPED_TARGETS: [&str; 2] = ["rmcp::transport::auth", "rmcp::transport::common::auth"];

/// 凭据日志上限层：与 `EnvFilter` 一起 `.with(...)` 到 `registry()` 上（见 `main.rs`）。
pub fn credential_log_cap<S: Subscriber>() -> impl Layer<S> {
    filter_fn(allowed as fn(&Metadata<'_>) -> bool)
}

/// 受限 target 只放行 `info` 及以上；其他事件不在这里过滤。
fn allowed(metadata: &Metadata<'_>) -> bool {
    let capped = CAPPED_TARGETS
        .iter()
        .any(|target| metadata.target().starts_with(target));
    !capped || *metadata.level() <= Level::INFO
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
        type Writer = Buffer;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// 用 `rust_log` 作为 `RUST_LOG`（与 `main.rs` 同样的层叠方式），分别向 rmcp 的两个
    /// 受限 target 与一个无关 target 发 trace / debug / info 事件，返回输出。
    fn output_with(rust_log: &str) -> String {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new(rust_log))
            .with(credential_log_cap())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(buffer.clone())
                    .with_ansi(false),
            );
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target: "rmcp::transport::auth", "AUTH-TRACE");
            tracing::debug!(target: "rmcp::transport::auth", "AUTH-DEBUG code=abc");
            tracing::info!(target: "rmcp::transport::auth", "AUTH-INFO");
            tracing::debug!(target: "rmcp::transport::common::auth::streamable_http_client", "COMMON-DEBUG");
            tracing::debug!(target: "asterlane::other", "OTHER-DEBUG");
        });
        let bytes = buffer.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn debug_and_trace_of_rmcp_auth_are_dropped_whatever_rust_log_says() {
        for rust_log in [
            "debug",
            "trace",
            "rmcp=trace",
            "rmcp::transport=debug",
            "rmcp::transport::auth=trace",
            "warn,rmcp::transport::auth=debug",
        ] {
            let text = output_with(rust_log);
            assert!(!text.contains("AUTH-TRACE"), "{rust_log}: {text}");
            assert!(!text.contains("AUTH-DEBUG"), "{rust_log}: {text}");
            assert!(!text.contains("COMMON-DEBUG"), "{rust_log}: {text}");
        }
    }

    #[test]
    fn info_of_rmcp_auth_and_other_targets_are_unaffected() {
        let text = output_with("debug");
        assert!(text.contains("AUTH-INFO"), "{text}");
        assert!(text.contains("OTHER-DEBUG"), "{text}");
    }

    #[test]
    fn a_stricter_rust_log_still_applies() {
        let text = output_with("warn");
        assert!(!text.contains("AUTH-INFO"), "{text}");
        assert!(!text.contains("OTHER-DEBUG"), "{text}");
    }
}
