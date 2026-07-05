//! 重试循环：构造请求 → 发送 → 判定可重试 → 退避 → failover。

use super::auth::apply_auth;
use super::error::ProxyError;
use super::executor::{InvokeResult, ProxyExecutor};
use crate::catalog::ParamLocations;
use crate::config::{HttpMethod, UpstreamAuth};
use crate::keys::LoadBalanceStrategy;
use crate::secrets::{SecretStore, SecretString};
use crate::store::{RequestEventRepository, SecurityEventRepository};
use backon::BackoffBuilder;
use std::time::Duration;

/// 可重试的上游状态码白名单（见 architecture.md Retry And Failover）。
const RETRYABLE_STATUSES: &[u16] = &[429, 500, 502, 503, 504];

/// 内部执行错误：携带 `ProxyError` 与观测字段（retry_count、upstream_key_ref），
/// 供 `invoke` 记录 `RequestEvent` 后再提取 `ProxyError` 返回。
#[derive(Debug)]
pub(super) struct ExecutionError {
    pub(super) proxy_error: ProxyError,
    pub(super) retry_count: u8,
    pub(super) upstream_key_ref: String,
}

impl From<ProxyError> for ExecutionError {
    fn from(e: ProxyError) -> Self {
        Self {
            proxy_error: e,
            retry_count: 0,
            upstream_key_ref: "<none>".to_string(),
        }
    }
}

impl<S: SecretStore, R: RequestEventRepository + SecurityEventRepository> ProxyExecutor<S, R> {
    /// 重试循环：构造请求 → 发送 → 判定可重试 → 退避 → failover。
    ///
    /// 第一阶段 failover 基础实现：重试时 mark_cooling 当前 key + acquire 新 key。
    /// 完整 failover 策略（per-key 凭据映射、权重感知轮换）留后续。
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_with_retry(
        &self,
        http_method: HttpMethod,
        upstream_path: &str,
        base_url: &str,
        auth: &UpstreamAuth,
        args: &serde_json::Value,
        secret: &Option<SecretString>,
        param_locations: Option<&ParamLocations>,
    ) -> Result<(InvokeResult, u8, String), ExecutionError> {
        let method = http_method.to_reqwest();
        let url = build_url(base_url, upstream_path, args, param_locations);
        let is_get = http_method == HttpMethod::Get;

        let backoff_builder = backon::ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(100))
            .with_max_delay(Duration::from_secs(10))
            .with_jitter()
            .with_max_times((self.max_attempts.saturating_sub(1)) as usize);
        let mut backoff = backoff_builder.build();

        let mut retry_count: u8 = 0;
        let mut upstream_key_ref = "<none>".to_string();

        for attempt in 1..=self.max_attempts {
            let key_guard = if let Some(pool) = &self.keys {
                match pool.acquire(LoadBalanceStrategy::RoundRobin) {
                    Ok(guard) => {
                        upstream_key_ref = guard.key_id().to_string();
                        Some(guard)
                    }
                    Err(e) => {
                        return Err(ExecutionError {
                            proxy_error: ProxyError::from(e),
                            retry_count,
                            upstream_key_ref,
                        });
                    }
                }
            } else {
                None
            };

            let mut builder = self.http.request(method.clone(), &url);
            builder = apply_auth(auth, secret.as_ref(), builder);
            builder = apply_params(builder, args, param_locations, is_get);

            // 发送（带超时包裹）
            let send_future = builder.send();
            let send_result = tokio::time::timeout(self.request_timeout, send_future).await;

            let response_result = match send_result {
                Ok(r) => r,
                Err(_elapsed) => {
                    // tokio::time::timeout 超时
                    if let (Some(pool), Some(guard)) = (&self.keys, &key_guard) {
                        pool.mark_cooling(guard.key_id(), None);
                    }
                    drop(key_guard);
                    if attempt < self.max_attempts {
                        if let Some(delay) = backoff.next() {
                            tokio::time::sleep(delay).await;
                        }
                        retry_count += 1;
                        continue;
                    }
                    return Err(ExecutionError {
                        proxy_error: ProxyError::UpstreamTimeout {
                            ms: self.request_timeout.as_millis() as u64,
                        },
                        retry_count,
                        upstream_key_ref,
                    });
                }
            };

            match response_result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let content_type = response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());
                    let content_length = response.content_length();

                    let body = match response.bytes().await {
                        Ok(b) => b.to_vec(),
                        Err(_) => Vec::new(),
                    };

                    // 日志用脱敏摘要（不记录响应体内容）
                    let _summary = crate::observability::redact_body(status, content_length);

                    if (200..300).contains(&status) {
                        return Ok((
                            InvokeResult {
                                status,
                                body,
                                content_type,
                                content_defense_flag: false,
                                shaped: false,
                                rendered_format: None,
                            },
                            retry_count,
                            upstream_key_ref,
                        ));
                    }

                    // 判定可重试
                    if attempt < self.max_attempts && is_retryable_status(status) {
                        // failover: mark cooling 当前 key
                        if let (Some(pool), Some(guard)) = (&self.keys, &key_guard) {
                            // TODO: 从 response header 解析 Retry-After
                            pool.mark_cooling(guard.key_id(), None);
                        }
                        drop(key_guard);
                        if let Some(delay) = backoff.next() {
                            tokio::time::sleep(delay).await;
                        }
                        retry_count += 1;
                        continue;
                    }

                    // 不可重试或最后一次
                    drop(key_guard);
                    let proxy_error = if retry_count > 0 && is_retryable_status(status) {
                        ProxyError::RetryExhausted { attempts: attempt }
                    } else {
                        ProxyError::UpstreamError(status)
                    };
                    return Err(ExecutionError {
                        proxy_error,
                        retry_count,
                        upstream_key_ref,
                    });
                }
                Err(e) => {
                    if let (Some(pool), Some(guard)) = (&self.keys, &key_guard) {
                        pool.mark_cooling(guard.key_id(), None);
                    }
                    drop(key_guard);
                    if e.is_timeout() {
                        if attempt < self.max_attempts {
                            if let Some(delay) = backoff.next() {
                                tokio::time::sleep(delay).await;
                            }
                            retry_count += 1;
                            continue;
                        }
                        return Err(ExecutionError {
                            proxy_error: ProxyError::UpstreamTimeout {
                                ms: self.request_timeout.as_millis() as u64,
                            },
                            retry_count,
                            upstream_key_ref,
                        });
                    }
                    // 连接失败（DNS/TCP/TLS）或其他请求错误
                    if attempt < self.max_attempts {
                        if let Some(delay) = backoff.next() {
                            tokio::time::sleep(delay).await;
                        }
                        retry_count += 1;
                        continue;
                    }
                    return Err(ExecutionError {
                        proxy_error: ProxyError::ConnectionFailed,
                        retry_count,
                        upstream_key_ref,
                    });
                }
            }
        }

        // 循环结束仍未成功（重试耗尽）
        Err(ExecutionError {
            proxy_error: ProxyError::RetryExhausted {
                attempts: self.max_attempts,
            },
            retry_count,
            upstream_key_ref,
        })
    }
}

/// 拼接 base_url 与 upstream_path，替换路径参数 `{xxx}` 并追加 query params。
fn build_url(
    base_url: &str,
    path: &str,
    args: &serde_json::Value,
    param_locations: Option<&ParamLocations>,
) -> String {
    let mut resolved = path.to_string();
    if let Some(obj) = args.as_object() {
        for (key, value) in obj {
            let placeholder = format!("{{{key}}}");
            if resolved.contains(&placeholder) {
                let s = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                resolved = resolved.replace(&placeholder, &s);
            }
        }
    }
    let mut url = format!("{base_url}{resolved}");

    if let (Some(pl), Some(obj)) = (param_locations, args.as_object()) {
        let mut first = true;
        for name in &pl.query_params {
            if let Some(v) = obj.get(name) {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                url.push(if first { '?' } else { '&' });
                url.push_str(name);
                url.push('=');
                url.push_str(&s);
                first = false;
            }
        }
    }

    url
}

/// Build request with parameter decomposition per ParamLocations.
///
/// When `param_locations` is Some (OpenAPI-discovered tool), args are decomposed:
/// - query_params → query string
/// - header_params → request headers
/// - body key → JSON body
///   When None (hand-written endpoint), falls back to legacy behavior:
///   non-GET sends entire args as JSON body.
fn apply_params(
    mut builder: reqwest::RequestBuilder,
    args: &serde_json::Value,
    param_locations: Option<&ParamLocations>,
    is_get: bool,
) -> reqwest::RequestBuilder {
    let obj = args.as_object();

    match param_locations {
        Some(pl) => {
            if let Some(obj) = obj {
                // Query params — append to URL
                // (handled in build_url via param_locations)

                // Header params
                for (field_name, header_name) in &pl.header_params {
                    if let Some(v) = obj.get(field_name).and_then(|v| v.as_str()) {
                        if let Ok(hv) = reqwest::header::HeaderValue::from_str(v) {
                            if let Ok(hn) =
                                reqwest::header::HeaderName::from_bytes(header_name.as_bytes())
                            {
                                builder = builder.header(hn, hv);
                            }
                        }
                    }
                }

                // Body
                if pl.has_body {
                    if let Some(body) = obj.get("body") {
                        builder = builder.json(body);
                    }
                }
            }
        }
        None => {
            // Legacy: non-GET sends entire args as JSON body
            if !is_get {
                builder = builder.json(args);
            }
        }
    }

    builder
}

/// 判断状态码是否在可重试白名单中。
fn is_retryable_status(status: u16) -> bool {
    RETRYABLE_STATUSES.contains(&status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_url_replaces_path_params() {
        let url = build_url(
            "https://api.example.com",
            "/{url}",
            &serde_json::json!({"url": "https://docs.rs"}),
            None,
        );
        assert_eq!(url, "https://api.example.com/https://docs.rs");
    }

    #[test]
    fn build_url_no_params_keeps_placeholder() {
        let url = build_url(
            "https://api.example.com",
            "/{url}",
            &serde_json::json!({}),
            None,
        );
        assert_eq!(url, "https://api.example.com/{url}");
    }

    #[test]
    fn is_retryable_status_covers_default_whitelist() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(!is_retryable_status(200));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(404));
    }
}
