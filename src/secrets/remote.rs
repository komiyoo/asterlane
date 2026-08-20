//! Vault / Infisical HTTP 瞬时分类与 `backon` 重试。
//!
//! 可重试：reqwest 超时 / 连接失败 / HTTP 5xx。
//! 不可重试：401/403/404/400、KV 缺 key、JSON 损坏、backend 未配置。
//! 错误 `detail` 不回显 reqwest 原文（可能含完整 URL / secret 路径段）。

use std::future::Future;
use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use reqwest::StatusCode;

use crate::secrets::SecretString;
use crate::secrets::error::SecretError;

/// 缺省瞬时重试次数（首次失败后再试这么多次；与 `SecretsConfig` 缺省一致）。
pub const DEFAULT_REMOTE_RETRIES: u32 = 2;

/// 将 reqwest 发送失败分类为瞬时（超时/连接）或永久失败。
pub fn map_reqwest_send_error(ref_uri: &str, err: &reqwest::Error) -> SecretError {
    if err.is_timeout() {
        SecretError::transient(ref_uri, "request timed out")
    } else if err.is_connect() {
        SecretError::transient(ref_uri, "connection failed")
    } else {
        SecretError::backend(ref_uri, "request failed")
    }
}

/// HTTP 状态：5xx 瞬时；其余（含 4xx）不重试。
pub fn map_http_status(ref_uri: &str, backend: &str, status: StatusCode) -> SecretError {
    let detail = format!("{backend} returned {status}");
    if status.is_server_error() {
        SecretError::transient(ref_uri, detail)
    } else {
        SecretError::backend(ref_uri, detail)
    }
}

/// 按 `retries` 对瞬时错误做指数退避（含 jitter）。`0` 只尝试一次。
pub async fn retry_remote<F, Fut>(retries: u32, op: F) -> Result<SecretString, SecretError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<SecretString, SecretError>>,
{
    op.retry(
        ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(20))
            .with_max_delay(Duration::from_millis(200))
            .with_jitter()
            .with_max_times(retries as usize),
    )
    .when(SecretError::is_retryable)
    .notify(|_, dur| {
        tracing::warn!(delay_ms = dur.as_millis() as u64, "remote secret retry");
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_5xx_is_retryable_and_redacts_path() {
        let err = map_http_status(
            "secret://vault/myapp/api-key",
            "vault",
            StatusCode::INTERNAL_SERVER_ERROR,
        );
        assert!(err.is_retryable());
        let display = err.to_string();
        let debug = format!("{err:?}");
        assert!(display.contains("500"));
        assert!(display.contains("secret://vault/"));
        assert!(!display.contains("myapp"));
        assert!(!display.contains("api-key"));
        assert!(!debug.contains("myapp"));
        assert!(!debug.contains("api-key"));
    }

    #[test]
    fn http_404_and_403_are_not_retryable() {
        let not_found = map_http_status("secret://vault/hidden", "vault", StatusCode::NOT_FOUND);
        let forbidden = map_http_status(
            "secret://infisical/SECRET",
            "infisical",
            StatusCode::FORBIDDEN,
        );
        assert!(!not_found.is_retryable());
        assert!(!forbidden.is_retryable());
        assert!(not_found.to_string().contains("404"));
        assert!(forbidden.to_string().contains("403"));
        assert!(!not_found.to_string().contains("hidden"));
        assert!(!forbidden.to_string().contains("SECRET"));
    }

    #[test]
    fn http_400_is_not_retryable() {
        let err = map_http_status("secret://vault/hidden", "vault", StatusCode::BAD_REQUEST);
        assert!(!err.is_retryable());
    }

    #[tokio::test]
    async fn retry_zero_does_not_retry_transient() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let calls = AtomicU32::new(0);
        let err = retry_remote(0, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(SecretError::transient(
                "secret://vault/hidden",
                "vault returned 500",
            ))
        })
        .await
        .expect_err("must fail");
        assert!(err.is_retryable());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_skips_non_transient() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let calls = AtomicU32::new(0);
        let err = retry_remote(2, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(SecretError::backend(
                "secret://vault/hidden",
                "vault returned 404",
            ))
        })
        .await
        .expect_err("must fail");
        assert!(!err.is_retryable());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
