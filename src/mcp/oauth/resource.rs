//! RFC 9728 受保护资源元数据里的 `resource`，用作 RFC 8707 token 请求的 `resource`。
//!
//! rmcp 3.1 做了授权服务器发现，但不公开它读到的 `resource`，而
//! client-credentials 的换取要求调用方显式给出。这里只读取这一个字段，
//! 授权服务器本身的发现仍然交给 rmcp。发现不到时调用方回退到 server URL。

use std::time::Duration;

use reqwest::Url;
use reqwest::header::WWW_AUTHENTICATE;
use rmcp::transport::WWWAuthenticateParams;

/// 单次元数据请求的超时。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 按 rmcp 的发现顺序找同源的受保护资源元数据：未带 token 请求 MCP URL 时
/// 401 的 `WWW-Authenticate: resource_metadata=…` 指针、路径插入式 well-known、
/// 根 well-known。返回元数据里对该 server 合法的 `resource`。
pub(super) async fn discover_resource(http: &reqwest::Client, server_url: &Url) -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(pointer) = challenge_pointer(http, server_url).await {
        candidates.push(pointer);
    }
    candidates.extend(well_known_urls(server_url));
    for candidate in candidates {
        if let Some(resource) = fetch_resource(http, server_url, &candidate).await {
            return Some(resource);
        }
    }
    None
}

/// 401 响应里 `resource_metadata` 指向的 URL（`WWWAuthenticateParams` 已要求同源）。
async fn challenge_pointer(http: &reqwest::Client, server_url: &Url) -> Option<Url> {
    let response = http
        .get(server_url.clone())
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .ok()?;
    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return None;
    }
    response
        .headers()
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| WWWAuthenticateParams::parse(value, server_url).resource_metadata_url)
}

/// RFC 9728 §3：把 `/.well-known/oauth-protected-resource` 插在 host 与 path 之间；
/// path 非空时再试根位置。
fn well_known_urls(server_url: &Url) -> Vec<Url> {
    let path = server_url.path().trim_end_matches('/');
    let mut urls = Vec::new();
    for suffix in [path, ""] {
        let mut url = server_url.clone();
        url.set_query(None);
        url.set_fragment(None);
        url.set_path(&format!("/.well-known/oauth-protected-resource{suffix}"));
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

async fn fetch_resource(
    http: &reqwest::Client,
    server_url: &Url,
    metadata_url: &Url,
) -> Option<String> {
    if !same_origin(server_url, metadata_url) {
        return None;
    }
    let response = http
        .get(metadata_url.clone())
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let document: serde_json::Value = response.json().await.ok()?;
    let resource = document.get("resource")?.as_str()?;
    is_identifier_for(server_url, resource).then(|| resource.to_string())
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// `resource` 必须是该 server 的标识：同源、无 fragment，且 path 是 server path 的前缀
/// （与 rmcp 校验受保护资源元数据的口径一致）。
fn is_identifier_for(server_url: &Url, resource: &str) -> bool {
    let Ok(resource) = Url::parse(resource) else {
        return false;
    };
    let prefix = resource.path().trim_end_matches('/');
    resource.fragment().is_none()
        && same_origin(server_url, &resource)
        && (server_url.path() == resource.path()
            || server_url
                .path()
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn well_known_inserts_the_document_path_between_host_and_path() {
        let urls = well_known_urls(&url("https://mcp.example.com/mcp?x=1"));
        assert_eq!(
            urls,
            vec![
                url("https://mcp.example.com/.well-known/oauth-protected-resource/mcp"),
                url("https://mcp.example.com/.well-known/oauth-protected-resource"),
            ]
        );
        // 根路径只有一个候选
        assert_eq!(well_known_urls(&url("https://mcp.example.com/")).len(), 1);
    }

    #[test]
    fn resource_must_identify_this_server() {
        let server = url("https://mcp.example.com/api/mcp");
        for ok in [
            "https://mcp.example.com/api/mcp",
            "https://mcp.example.com/api",
            "https://mcp.example.com",
            "https://mcp.example.com/",
        ] {
            assert!(is_identifier_for(&server, ok), "{ok}");
        }
        for bad in [
            "https://other.example.com/api/mcp",
            "http://mcp.example.com/api/mcp",
            "https://mcp.example.com:8443/api/mcp",
            "https://mcp.example.com/ap",
            "https://mcp.example.com/api/mcp#frag",
            "not a url",
        ] {
            assert!(!is_identifier_for(&server, bad), "{bad}");
        }
    }
}
