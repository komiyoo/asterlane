//! HTTP 边界错误转换：把 `AsterlaneError` 转换为 axum 响应。
//!
//! JSON body 形态见 `docs/error-model.md`：
//! ```json
//! { "error": { "code": "...", "message": "...", "request_id": "..." } }
//! ```

use crate::error::AsterlaneError;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

impl IntoResponse for AsterlaneError {
    fn into_response(self) -> Response {
        let view = self.http_response();
        let status = StatusCode::from_u16(view.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = json!({
            "error": {
                "code": view.code.as_str(),
                "message": view.message,
                "request_id": view.request_id,
            }
        });
        let mut response = (status, Json(body)).into_response();
        if let Some(dur) = view.retry_after {
            let secs = dur.as_secs().max(1).to_string();
            if let Ok(val) = axum::http::HeaderValue::from_str(&secs) {
                response.headers_mut().insert("retry-after", val);
            }
        }
        response
    }
}
