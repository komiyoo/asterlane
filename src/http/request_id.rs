//! 入站 HTTP `request_id`：生成或接纳客户端 ID，写入 tracing span 与 task-local。
//!
//! `AsterlaneError` 的 `IntoResponse` 拿不到 `Request`，从 task-local 回填 JSON。

use axum::extract::Request;
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;

use crate::observability::next_request_id;

/// 客户端 `X-Request-Id` 最长保留长度（字符，不含控制字符）。
const MAX_CLIENT_REQUEST_ID_LEN: usize = 64;

tokio::task_local! {
    static CURRENT_REQUEST_ID: String;
}

/// 当前任务上由中间件写入的 request_id。
pub(super) fn current_request_id() -> Option<String> {
    CURRENT_REQUEST_ID.try_with(Clone::clone).ok()
}

/// 错误 JSON 用的 request_id：优先 task-local，否则新生成，保证非空。
pub(super) fn error_request_id() -> String {
    current_request_id().unwrap_or_else(next_request_id)
}

/// 在进入 handler 之前解析/生成 request_id，写入 span 字段与 task-local。
pub(super) async fn attach_request_id(req: Request, next: Next) -> Response {
    let id = request_id_from_headers(req.headers()).unwrap_or_else(next_request_id);
    let span = tracing::debug_span!("http.request", request_id = %id);
    CURRENT_REQUEST_ID
        .scope(id, next.run(req).instrument(span))
        .await
}

fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    // HTTP 头大小写不敏感：`X-Request-Id` 与 `X-Request-ID` 同一字段。
    let raw = headers.get("x-request-id")?.to_str().ok()?;
    sanitize_client_request_id(raw)
}

fn sanitize_client_request_id(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut count = 0usize;
    for c in raw.trim().chars() {
        if c.is_control() {
            continue;
        }
        out.push(c);
        count += 1;
        if count >= MAX_CLIENT_REQUEST_ID_LEN {
            break;
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::{MAX_CLIENT_REQUEST_ID_LEN, sanitize_client_request_id};

    #[test]
    fn sanitize_strips_controls_and_truncates() {
        assert_eq!(
            sanitize_client_request_id("  client-id\u{0001}  ").as_deref(),
            Some("client-id")
        );
        let long = "a".repeat(80);
        let got = sanitize_client_request_id(&long).expect("kept");
        assert_eq!(got.len(), MAX_CLIENT_REQUEST_ID_LEN);
        assert!(sanitize_client_request_id("\n\t").is_none());
        assert!(sanitize_client_request_id("   ").is_none());
    }
}
