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
        // request_id 第一阶段为 None；后续由请求中间件生成并注入。
        let body = json!({
            "error": {
                "code": view.code.as_str(),
                "message": view.message,
                "request_id": view.request_id,
            }
        });
        (status, Json(body)).into_response()
    }
}
