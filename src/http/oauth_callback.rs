//! `GET /oauth/callback`：授权码流程的浏览器回调。
//!
//! 授权服务器把管理员的浏览器重定向到这里，带 `code` 与 `state`。这是顶层路由，
//! 不挂 admin 认证（浏览器跳转带不上 admin token），唯一的凭证是 `state`：
//! 只对 `POST /admin/mcp-servers/{id}/oauth/authorize` 发起过、10 分钟内且未使用过的
//! state 有效，校验与换 token 见 [`crate::mcp::UpstreamOAuth::complete_authorization`]。
//!
//! 响应是极简 HTML，不显示任何 token，也不把 query 参数或授权服务器返回的内容写进
//! 页面（页面文字都是固定文案，唯一的动态内容 `request_id` 与 server id 经转义）。
//! 授权服务器返回的错误细节只进 tracing。响应带 `Cache-Control: no-store`。

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use tracing::warn;

use super::AppState;
use super::request_id::error_request_id;
use crate::mcp::{CallbackError, CallbackParams};

/// 页面不加载任何外部资源，也不允许被嵌入。
const PAGE_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const PAGE_STYLE: &str = "body{font-family:system-ui,sans-serif;max-width:32rem;margin:15vh auto;\
padding:0 1rem;line-height:1.6}h1{font-size:1.4rem}code{background:rgba(127,127,127,.18);\
padding:.1em .35em;border-radius:3px}@media(prefers-color-scheme:dark){body{background:#16181c;\
color:#e6e6e6}}";

/// `GET /oauth/callback?code=&state=`（或授权服务器的 `error=`）。
pub(super) async fn callback(
    State(state): State<AppState>,
    query: Result<Query<CallbackParams>, QueryRejection>,
) -> Response {
    let request_id = error_request_id();
    // 路由只在装配了 OAuth 服务时挂载；这里仍按「无效请求」处理，不 panic
    let Some(oauth) = state.upstream_oauth.clone() else {
        warn!(
            reason = "oauth_unavailable",
            "upstream OAuth callback rejected"
        );
        return error_page(CallbackError::InvalidState, &request_id);
    };
    let Ok(Query(params)) = query else {
        warn!(
            reason = "malformed_query",
            "upstream OAuth callback rejected"
        );
        return error_page(CallbackError::InvalidState, &request_id);
    };
    let server_id = match oauth.complete_authorization(params).await {
        Ok(server_id) => server_id,
        Err(error) => return error_page(error, &request_id),
    };
    // 凭据已保存；重连让该 server 立即可用。重连失败不撤销授权，只在页面上说明
    let connected = match state.reconnect_mcp_server(&server_id).await {
        Ok(health) => !health.status.is_unavailable(),
        Err(error) => {
            warn!(server_id = %server_id, %error, "reconnect after OAuth authorization failed");
            false
        }
    };
    success_page(&server_id, connected)
}

fn success_page(server_id: &str, connected: bool) -> Response {
    let server = escape_html(server_id);
    let body = if connected {
        format!(
            "<h1>授权完成</h1><p>已为 MCP 服务 <code>{server}</code> 完成授权，网关已保存凭据并重新连接。</p>\
             <p>可以关闭此页面。</p>"
        )
    } else {
        format!(
            "<h1>授权完成</h1><p>已为 MCP 服务 <code>{server}</code> 完成授权，网关已保存凭据，\
             但暂时无法连接该服务。请在控制台查看服务状态，或稍后点击「探测」。</p>\
             <p>可以关闭此页面。</p>"
        )
    };
    html_page(StatusCode::OK, "授权完成", &body)
}

fn error_page(error: CallbackError, request_id: &str) -> Response {
    let (status, advice) = match error {
        CallbackError::InvalidState => (
            StatusCode::BAD_REQUEST,
            "授权请求无效，或已经使用过。请回到控制台重新发起授权。",
        ),
        CallbackError::Expired => (
            StatusCode::BAD_REQUEST,
            "授权请求已过期。请回到控制台重新发起授权，并在有效期内完成。",
        ),
        CallbackError::NotGranted => (
            StatusCode::BAD_REQUEST,
            "授权服务器没有授予访问权限（可能在授权页面选择了拒绝）。如需授权，请回到控制台重新发起。",
        ),
        CallbackError::MissingCode => (
            StatusCode::BAD_REQUEST,
            "授权服务器的响应不完整。请回到控制台重新发起授权。",
        ),
        CallbackError::ExchangeFailed => (
            StatusCode::BAD_GATEWAY,
            "向授权服务器换取令牌失败。请回到控制台重新发起授权；反复失败时，凭请求编号查看网关日志。",
        ),
    };
    let body = format!(
        "<h1>授权失败</h1><p>{advice}</p><p>请求编号：<code>{}</code></p>",
        escape_html(request_id)
    );
    html_page(status, "授权失败", &body)
}

fn html_page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title}</title><style>{PAGE_STYLE}</style></head><body><main>{body}</main></body></html>"
    );
    (
        status,
        [
            (CONTENT_TYPE, "text/html; charset=utf-8"),
            (CACHE_CONTROL, "no-store"),
            (CONTENT_SECURITY_POLICY, PAGE_CSP),
        ],
        html,
    )
        .into_response()
}

/// 转义 HTML 文本与属性值里的特殊字符。
fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn text(response: Response) -> (StatusCode, axum::http::HeaderMap, String) {
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        (
            parts.status,
            parts.headers,
            String::from_utf8(bytes.to_vec()).unwrap(),
        )
    }

    #[test]
    fn escape_covers_html_special_characters() {
        assert_eq!(
            escape_html(r#"<img src=x onerror="a('b')">&"#),
            "&lt;img src=x onerror=&quot;a(&#39;b&#39;)&quot;&gt;&amp;"
        );
    }

    #[tokio::test]
    async fn error_page_escapes_the_request_id_and_forbids_caching() {
        let (status, headers, body) = text(error_page(
            CallbackError::InvalidState,
            "<script>alert(1)</script>",
        ))
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(headers[CACHE_CONTROL], "no-store");
        assert!(
            headers[CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("default-src 'none'")
        );
        assert!(!body.contains("<script>"));
        assert!(body.contains("&lt;script&gt;"));
    }

    #[tokio::test]
    async fn exchange_failure_is_a_bad_gateway_and_other_failures_are_bad_requests() {
        for (error, expected) in [
            (CallbackError::InvalidState, StatusCode::BAD_REQUEST),
            (CallbackError::Expired, StatusCode::BAD_REQUEST),
            (CallbackError::NotGranted, StatusCode::BAD_REQUEST),
            (CallbackError::MissingCode, StatusCode::BAD_REQUEST),
            (CallbackError::ExchangeFailed, StatusCode::BAD_GATEWAY),
        ] {
            let (status, _, body) = text(error_page(error, "req-1")).await;
            assert_eq!(status, expected, "{error}");
            assert!(body.contains("req-1"));
        }
    }

    #[tokio::test]
    async fn success_page_escapes_the_server_id() {
        let (status, headers, body) = text(success_page("<b>x</b>", true)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[CACHE_CONTROL], "no-store");
        assert!(body.contains("授权完成"));
        assert!(!body.contains("<b>x</b>"));
        let (_, _, degraded) = text(success_page("srv", false)).await;
        assert!(degraded.contains("暂时无法连接"));
    }
}
