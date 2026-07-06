//! admin API HTTP 客户端层：连接解析、认证、URL/query 组装、
//! 响应体解析、错误与退出码映射（docs/error-model.md「CLI 边界」）。
//!
//! token 以 [`SecretString`] 持有且不实现 Debug；任何输出不回显 token。

use anyhow::{Context, Result, anyhow};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use std::time::Duration;

/// 缺省 admin API 地址（`serve` 的默认 bind）。
const DEFAULT_SERVER: &str = "http://127.0.0.1:3000";
/// 非 JSON 响应体的 stderr 预览预算（字符数，UTF-8 安全）。
const RAW_BODY_PREVIEW_CHARS: usize = 2000;

/// CLI 错误：区分服务端错误响应与本地错误，用于退出码映射。
pub(super) enum CliError {
    /// 非 2xx 响应；body 已解析（或包装）为错误 JSON。
    Api { body: Value },
    /// 本地错误（token 缺失、参数不合法、网络/连接失败等）→ 退出码 1。
    Local(anyhow::Error),
}

impl From<anyhow::Error> for CliError {
    fn from(err: anyhow::Error) -> Self {
        Self::Local(err)
    }
}

impl CliError {
    fn exit_code(&self) -> i32 {
        match self {
            Self::Api { body } => body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .map_or(1, exit_code_for_code),
            Self::Local(_) => 1,
        }
    }

    /// stderr 输出错误并返回退出码（不回显 token）。
    pub(super) fn report(self) -> i32 {
        let code = self.exit_code();
        match self {
            Self::Api { body } => eprintln!("{}", pretty(&body)),
            Self::Local(err) => eprintln!("error: {err:#}"),
        }
        code
    }
}

/// 按错误码类别映射 CLI 退出码（docs/error-model.md「CLI 边界」）。
fn exit_code_for_code(code: &str) -> i32 {
    match code.split('.').next().unwrap_or_default() {
        "config" => 2,
        "auth" | "admin" => 3,
        "catalog" | "mcp" => 4,
        "store" => 5,
        "proxy" => 6,
        "limit" => 7,
        "transform" => 8,
        _ => 1,
    }
}

pub(super) fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// admin API HTTP 客户端。
pub(super) struct AdminClient {
    http: reqwest::Client,
    /// 服务器地址，末尾无 `/`。
    base: String,
    token: SecretString,
}

impl AdminClient {
    pub(super) fn new(server: Option<String>, token_env: &str) -> Result<Self> {
        let base = resolve_server(server, std::env::var("ASTERLANE_SERVER").ok());
        let token = std::env::var(token_env)
            .ok()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "admin token not found: set env {token_env} \
                     (token is only read from the environment; \
                     there is no --token flag because argv is visible via ps)"
                )
            })?;
        let http = reqwest::Client::builder()
            // invoke 会等待上游工具调用完成，给足预算
            .timeout(Duration::from_secs(120))
            .build()
            .context("failed to build http client")?;
        Ok(Self {
            http,
            base,
            token: SecretString::from(token),
        })
    }

    pub(super) async fn get(
        &self,
        path: &str,
        query: &[(&'static str, String)],
    ) -> Result<Value, CliError> {
        self.send(self.http.get(build_url(&self.base, path, query)))
            .await
    }

    pub(super) async fn put_json(&self, path: &str, body: &Value) -> Result<Value, CliError> {
        self.send(self.http.put(build_url(&self.base, path, &[])).json(body))
            .await
    }

    pub(super) async fn delete(&self, path: &str) -> Result<Value, CliError> {
        self.send(self.http.delete(build_url(&self.base, path, &[])))
            .await
    }

    pub(super) async fn post_json(
        &self,
        path: &str,
        query: &[(&'static str, String)],
        body: &Value,
    ) -> Result<Value, CliError> {
        self.send(
            self.http
                .post(build_url(&self.base, path, query))
                .json(body),
        )
        .await
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, CliError> {
        let resp = req
            .bearer_auth(self.token.expose_secret())
            .send()
            .await
            .context("admin api request failed (is the gateway running?)")?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .context("failed to read admin api response")?;
        let body = parse_body(status, &text);
        if (200..300).contains(&status) {
            Ok(body)
        } else {
            Err(CliError::Api { body })
        }
    }
}

/// 组装完整请求 URL：`{base}{path}?k=v&…`，value 经百分号编码。
///
/// 本 crate 的 reqwest 未启用 `query` feature（`RequestBuilder::query` 不可用），
/// query 组装在此手动完成；key 均为静态安全标识符，不需编码。
fn build_url(base: &str, path: &str, query: &[(&'static str, String)]) -> String {
    let mut url = format!("{base}{path}");
    for (i, (key, value)) in query.iter().enumerate() {
        url.push(if i == 0 { '?' } else { '&' });
        url.push_str(key);
        url.push('=');
        url.push_str(&encode_query_component(value));
    }
    url
}

/// query 组件百分号编码：RFC 3986 unreserved 之外的字节全部 `%XX` 编码
/// （过度编码无害；RFC3339 时间戳里的 `+`/`:` 必须编码才能安全过服务端解码）。
fn encode_query_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

/// server 解析优先级：`--server` > env `ASTERLANE_SERVER` > 默认；去掉尾部 `/`。
fn resolve_server(flag: Option<String>, env: Option<String>) -> String {
    flag.or(env)
        .unwrap_or_else(|| DEFAULT_SERVER.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// 响应体解析：空体 → `{"ok": …}`；非 JSON 体截断预览后按 status 包装。
fn parse_body(status: u16, text: &str) -> Value {
    if text.trim().is_empty() {
        return json!({ "ok": status < 400, "http_status": status });
    }
    if let Ok(value) = serde_json::from_str(text) {
        return value;
    }
    let preview: String = text.chars().take(RAW_BODY_PREVIEW_CHARS).collect();
    if status < 400 {
        json!({ "raw": preview })
    } else {
        json!({ "error": { "code": "internal.unexpected", "message": preview, "http_status": status } })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_server_prefers_flag_then_env_then_default() {
        assert_eq!(
            resolve_server(Some("http://a:1/".into()), Some("http://b:2".into())),
            "http://a:1"
        );
        assert_eq!(
            resolve_server(None, Some("http://b:2".into())),
            "http://b:2"
        );
        assert_eq!(resolve_server(None, None), "http://127.0.0.1:3000");
    }

    #[test]
    fn build_url_appends_and_encodes_query() {
        assert_eq!(
            build_url("http://h:1", "/admin/stats", &[]),
            "http://h:1/admin/stats"
        );
        let url = build_url(
            "http://h:1",
            "/admin/events",
            &[
                ("tool_name", "search__exa__web_search_exa".to_string()),
                ("from", "2026-07-01T00:00:00+08:00".to_string()),
                ("limit", "5".to_string()),
            ],
        );
        assert_eq!(
            url,
            "http://h:1/admin/events?tool_name=search__exa__web_search_exa\
             &from=2026-07-01T00%3A00%3A00%2B08%3A00&limit=5"
        );
    }

    #[test]
    fn encode_query_component_covers_reserved_and_utf8() {
        assert_eq!(encode_query_component("abc-_.~09"), "abc-_.~09");
        assert_eq!(encode_query_component("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(encode_query_component("时"), "%E6%97%B6");
    }

    #[test]
    fn exit_codes_follow_error_model_categories() {
        assert_eq!(exit_code_for_code("config.invalid_yaml"), 2);
        assert_eq!(exit_code_for_code("auth.forbidden_tool"), 3);
        assert_eq!(exit_code_for_code("admin.unauthorized"), 3);
        assert_eq!(exit_code_for_code("admin.not_found"), 3);
        assert_eq!(exit_code_for_code("catalog.unknown_tool"), 4);
        assert_eq!(exit_code_for_code("mcp.upstream_mcp_failure"), 4);
        assert_eq!(exit_code_for_code("store.unavailable"), 5);
        assert_eq!(exit_code_for_code("proxy.upstream_timeout"), 6);
        assert_eq!(exit_code_for_code("limit.quota_exceeded"), 7);
        assert_eq!(exit_code_for_code("transform.invalid_pointer"), 8);
        assert_eq!(exit_code_for_code("internal.unexpected"), 1);
        assert_eq!(exit_code_for_code(""), 1);
    }

    #[test]
    fn api_error_exit_code_reads_body_error_code() {
        let err = CliError::Api {
            body: json!({ "error": { "code": "admin.unauthorized", "message": "x" } }),
        };
        assert_eq!(err.exit_code(), 3);
        let no_code = CliError::Api { body: json!({}) };
        assert_eq!(no_code.exit_code(), 1);
        assert_eq!(CliError::Local(anyhow!("boom")).exit_code(), 1);
    }

    #[test]
    fn parse_body_handles_json_empty_and_raw() {
        assert_eq!(parse_body(200, "{\"a\":1}"), json!({"a":1}));
        assert_eq!(
            parse_body(204, "  "),
            json!({"ok": true, "http_status": 204})
        );
        assert_eq!(parse_body(200, "plain"), json!({"raw": "plain"}));
        let wrapped = parse_body(502, "bad gateway");
        assert_eq!(wrapped["error"]["code"], "internal.unexpected");
        assert_eq!(wrapped["error"]["http_status"], 502);
    }
}
