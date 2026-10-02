//! 上游 prompts、resources、resource templates 的快照与下游命名空间。
//!
//! 可见范围与工具共用 [`crate::policy::key_can_use_name`]。resource 与 template
//! 的匹配名 `domain__provider__<上游 name>` 只用于判权，不放进下游响应。
//! 快照保留 rmcp 的完整条目（icons、annotations、`_meta` 等原样转发），只在
//! 对外时改写 prompt 名与 URI。

use std::collections::HashSet;

use rmcp::model::{Prompt, ReadResourceResult, Resource, ResourceContents, ResourceTemplate};
use tracing::warn;

use crate::config::McpServerConfig;
use crate::mcp::error::McpError;
use crate::mcp::peer::{McpFuture, RemoteMcpPeer};
use crate::naming::{ToolName, scope_match_name};

const URI_SCHEME: &str = "asterlane://";

/// 一个上游在某次探测后的 prompts / resources / templates。
/// 不变式：与工具快照一起刷新，连接断开（无 peer）时为空。
#[derive(Debug, Clone, Default)]
pub(super) struct SurfaceSnapshot {
    pub(super) prompts: Vec<PromptSnap>,
    pub(super) resources: Vec<ResourceSnap>,
    pub(super) templates: Vec<TemplateSnap>,
}

#[derive(Debug, Clone)]
pub(super) struct PromptSnap {
    /// 对下游暴露的名字 `domain__provider__<上游 prompt 名>`，同时是判权名。
    pub(super) wire_name: String,
    /// 上游原样的 prompt（`name` 是上游原名）。
    pub(super) prompt: Prompt,
}

#[derive(Debug, Clone)]
pub(super) struct ResourceSnap {
    /// `domain__provider__<上游 name>`，只用于判权。
    pub(super) scope_name: String,
    /// 上游原样的 resource（`uri` 是上游原 URI）。
    pub(super) resource: Resource,
}

#[derive(Debug, Clone)]
pub(super) struct TemplateSnap {
    pub(super) scope_name: String,
    /// 第一个 `{` 之前的字面量。模板以 `{` 开头时为空串，能匹配任意 URI。
    pub(super) literal_prefix: String,
    /// 上游原样的 template（`uri_template` 是上游原模板）。
    pub(super) template: ResourceTemplate,
}

/// `asterlane://{server_id}/{上游原 URI}`。template 的变量原样留在后半段。
pub(super) fn gateway_uri(server_id: &str, upstream: &str) -> String {
    format!("{URI_SCHEME}{server_id}/{upstream}")
}

/// 还原下游 URI：返回 `(server_id, 上游原 URI)`。server id 取第一个 `/` 之前的段。
pub(super) fn parse_gateway_uri(uri: &str) -> Option<(&str, &str)> {
    let (server_id, upstream) = uri.strip_prefix(URI_SCHEME)?.split_once('/')?;
    (!server_id.is_empty() && !upstream.is_empty()).then_some((server_id, upstream))
}

pub(super) fn wrap_prompts(config: &McpServerConfig, prompts: Vec<Prompt>) -> Vec<PromptSnap> {
    let wrapped = prompts.into_iter().filter_map(|prompt| {
        match ToolName::new(&config.domain, &config.provider, &prompt.name) {
            Ok(name) => Some(PromptSnap {
                wire_name: name.to_wire_name(),
                prompt,
            }),
            Err(error) => skip_unwrappable(config, "prompt", &prompt.name, &error),
        }
    });
    unique_by(config, "prompt", wrapped, |snap| snap.wire_name.clone())
}

pub(super) fn wrap_resources(
    config: &McpServerConfig,
    resources: Vec<Resource>,
) -> Vec<ResourceSnap> {
    let wrapped = resources.into_iter().filter_map(|resource| {
        if resource.uri.is_empty() {
            return skip_unwrappable(config, "resource", &resource.name, &"empty uri");
        }
        match scope_match_name(&config.domain, &config.provider, &resource.name) {
            Ok(scope_name) => Some(ResourceSnap {
                scope_name,
                resource,
            }),
            Err(error) => skip_unwrappable(config, "resource", &resource.name, &error),
        }
    });
    unique_by(config, "resource", wrapped, |snap| {
        snap.resource.uri.clone()
    })
}

pub(super) fn wrap_templates(
    config: &McpServerConfig,
    templates: Vec<ResourceTemplate>,
) -> Vec<TemplateSnap> {
    let wrapped = templates.into_iter().filter_map(|template| {
        if template.uri_template.is_empty() {
            return skip_unwrappable(config, "template", &template.name, &"empty uri template");
        }
        match scope_match_name(&config.domain, &config.provider, &template.name) {
            Ok(scope_name) => Some(TemplateSnap {
                scope_name,
                literal_prefix: literal_prefix(&template.uri_template).to_string(),
                template,
            }),
            Err(error) => skip_unwrappable(config, "template", &template.name, &error),
        }
    });
    unique_by(config, "template", wrapped, |snap| {
        snap.template.uri_template.clone()
    })
}

fn literal_prefix(uri_template: &str) -> &str {
    uri_template
        .split_once('{')
        .map_or(uri_template, |(prefix, _)| prefix)
}

/// 条目无法包装（名字不合法、URI 为空）：告警并跳过，不影响其余条目。
fn skip_unwrappable<T>(
    config: &McpServerConfig,
    kind: &str,
    name: &str,
    reason: &dyn std::fmt::Display,
) -> Option<T> {
    warn!(server_id = %config.id, kind, name, reason = %reason, "跳过无法包装的上游条目");
    None
}

/// 保留首个出现的条目；同一 server 内键重复的后来者告警后丢弃。
fn unique_by<T>(
    config: &McpServerConfig,
    kind: &str,
    items: impl Iterator<Item = T>,
    key: impl Fn(&T) -> String,
) -> Vec<T> {
    let mut seen = HashSet::new();
    items
        .filter(|item| {
            let key = key(item);
            let fresh = seen.insert(key.clone());
            if !fresh {
                warn!(server_id = %config.id, kind, key = %key, "跳过重复的上游条目");
            }
            fresh
        })
        .collect()
}

/// 对外的 prompt：名字换成包装名，其余原样。
pub(super) fn to_prompt(snap: &PromptSnap) -> Prompt {
    let mut prompt = snap.prompt.clone();
    prompt.name = snap.wire_name.clone();
    prompt
}

pub(super) fn to_resource(server_id: &str, snap: &ResourceSnap) -> Resource {
    let mut resource = snap.resource.clone();
    resource.uri = gateway_uri(server_id, &snap.resource.uri);
    resource
}

pub(super) fn to_template(server_id: &str, snap: &TemplateSnap) -> ResourceTemplate {
    let mut template = snap.template.clone();
    template.uri_template = gateway_uri(server_id, &snap.template.uri_template);
    template
}

/// 读取某个上游 URI 时应该用哪个判权名：先按 resource 的上游 URI 精确匹配；
/// 未命中再取字面前缀最长的 template；都不命中返回 `None`。resource 命中后只对
/// 它自己的名字判权，无权限时不会回落到 template 放行。
pub(super) fn scope_name_for_read<'a>(
    resources: &'a [ResourceSnap],
    templates: &'a [TemplateSnap],
    upstream_uri: &str,
) -> Option<&'a str> {
    if let Some(resource) = resources
        .iter()
        .find(|snap| snap.resource.uri == upstream_uri)
    {
        return Some(&resource.scope_name);
    }
    templates
        .iter()
        .filter(|snap| upstream_uri.starts_with(&snap.literal_prefix))
        .max_by_key(|snap| snap.literal_prefix.len())
        .map(|snap| snap.scope_name.as_str())
}

/// 把上游读回的 content URI 改写成下游命名空间。已经是该 server 前缀的不再叠加。
pub(super) fn rewrite_read_uris(server_id: &str, result: &mut ReadResourceResult) {
    let prefix = gateway_uri(server_id, "");
    for content in &mut result.contents {
        // `ResourceContents` 是 non_exhaustive：未知变体没有 URI 可改，原样放过。
        if let Some(uri) = content_uri_mut(content)
            && !uri.starts_with(&prefix)
        {
            *uri = gateway_uri(server_id, uri);
        }
    }
}

fn content_uri_mut(content: &mut ResourceContents) -> Option<&mut String> {
    match content {
        ResourceContents::TextResourceContents { uri, .. }
        | ResourceContents::BlobResourceContents { uri, .. } => Some(uri),
        _ => None,
    }
}

/// 按上游 capability 拉取 prompts / resources / templates。
///
/// 未声明 capability 时不调用对应方法，快照为空。声明了但拉取失败时告警并保留
/// `previous` 里的同类快照，不把整次工具探测判失败。
pub(super) async fn load_surface(
    config: &McpServerConfig,
    peer: &dyn RemoteMcpPeer,
    previous: &SurfaceSnapshot,
) -> SurfaceSnapshot {
    let (prompts, resources, templates) = tokio::join!(
        load_kind(
            config,
            "prompts",
            peer.supports_prompts(),
            || peer.list_prompts(),
            wrap_prompts,
            &previous.prompts
        ),
        load_kind(
            config,
            "resources",
            peer.supports_resources(),
            || peer.list_resources(),
            wrap_resources,
            &previous.resources
        ),
        load_kind(
            config,
            "templates",
            peer.supports_resources(),
            || peer.list_resource_templates(),
            wrap_templates,
            &previous.templates
        ),
    );
    SurfaceSnapshot {
        prompts,
        resources,
        templates,
    }
}

async fn load_kind<'a, U, T: Clone>(
    config: &McpServerConfig,
    kind: &str,
    supported: bool,
    list: impl FnOnce() -> McpFuture<'a, Result<Vec<U>, McpError>>,
    wrap: fn(&McpServerConfig, Vec<U>) -> Vec<T>,
    previous: &[T],
) -> Vec<T> {
    if !supported {
        return Vec::new();
    }
    match list().await {
        Ok(items) => wrap(config, items),
        Err(error) => {
            warn!(
                server_id = %config.id,
                kind,
                error = %error,
                "拉取上游列表失败，保留上一次快照"
            );
            previous.to_vec()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> McpServerConfig {
        McpServerConfig {
            id: "docs".to_string(),
            domain: "Docs".to_string(),
            provider: "wiki".to_string(),
            url: "https://example.test/mcp".to_string(),
            description: String::new(),
            auth: crate::config::UpstreamAuth::None,
            security: crate::config::SecurityConfig::default(),
            health_check: crate::config::HealthCheckConfig::default(),
            limits: None,
        }
    }

    #[test]
    fn uri_round_trip_keeps_template_variables() {
        let template = "file:///{path}";
        let gateway = gateway_uri("docs", template);
        assert_eq!(gateway, "asterlane://docs/file:///{path}");
        assert_eq!(parse_gateway_uri(&gateway), Some(("docs", template)));
    }

    #[test]
    fn malformed_gateway_uris_do_not_parse() {
        for uri in [
            "",
            "file:///notes.txt",
            "asterlane://",
            "asterlane://docs",
            "asterlane://docs/",
            "asterlane:///file:///x",
            "ASTERLANE://docs/x",
        ] {
            assert_eq!(parse_gateway_uri(uri), None, "{uri}");
        }
    }

    #[test]
    fn illegal_prompt_names_are_skipped_and_duplicates_keep_the_first() {
        let prompts = wrap_prompts(
            &config(),
            vec![
                Prompt::new("summarize", Some("first"), None),
                Prompt::new("bad/name", Some("nope"), None),
                Prompt::new("Summarize", Some("same wire name"), None),
            ],
        );
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].wire_name, "docs__wiki__summarize");
        assert_eq!(prompts[0].prompt.name, "summarize");
        assert_eq!(to_prompt(&prompts[0]).name, "docs__wiki__summarize");
        assert_eq!(to_prompt(&prompts[0]).description.as_deref(), Some("first"));
    }

    #[test]
    fn resource_names_may_contain_characters_tool_names_reject() {
        let resources = wrap_resources(
            &config(),
            vec![
                Resource::new("file:///a.md", "docs/a.md"),
                Resource::new("", "empty-uri"),
                Resource::new("file:///a.md", "duplicate uri"),
            ],
        );
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].scope_name, "docs__wiki__docs/a.md");
    }

    #[test]
    fn read_prefers_exact_resource_then_longest_template_prefix() {
        let resources =
            wrap_resources(&config(), vec![Resource::new("file:///notes.txt", "notes")]);
        let templates = wrap_templates(
            &config(),
            vec![
                ResourceTemplate::new("file:///{path}", "any"),
                ResourceTemplate::new("file:///secret/{path}", "secret"),
                ResourceTemplate::new("{scheme}://x", "bare"),
            ],
        );
        let scope = |uri| scope_name_for_read(&resources, &templates, uri);
        assert_eq!(scope("file:///notes.txt"), Some("docs__wiki__notes"));
        assert_eq!(scope("file:///secret/a"), Some("docs__wiki__secret"));
        assert_eq!(scope("file:///other"), Some("docs__wiki__any"));
        // 以 `{` 开头的 template 字面前缀为空，匹配任意 URI
        assert_eq!(scope("memory://x"), Some("docs__wiki__bare"));
        assert_eq!(scope_name_for_read(&resources, &[], "memory://x"), None);
    }

    #[test]
    fn listed_items_are_rewritten_and_hide_scope_names() {
        let mut resource = Resource::new("file:///notes.txt", "notes");
        resource.size = Some(7);
        let snap = wrap_resources(&config(), vec![resource]);
        let json = serde_json::to_value(to_resource("docs", &snap[0])).unwrap();
        assert_eq!(json["uri"], "asterlane://docs/file:///notes.txt");
        assert_eq!(json["name"], "notes");
        assert_eq!(json["size"], 7);
        assert!(!json.to_string().contains("docs__wiki__notes"));

        let templates = wrap_templates(
            &config(),
            vec![ResourceTemplate::new("file:///{path}", "file")],
        );
        let json = serde_json::to_value(to_template("docs", &templates[0])).unwrap();
        assert_eq!(json["uriTemplate"], "asterlane://docs/file:///{path}");
    }

    #[test]
    fn content_uris_are_namespaced_once() {
        let mut result = ReadResourceResult::new(vec![
            ResourceContents::text("a", "file:///a"),
            ResourceContents::text("b", "asterlane://docs/file:///b"),
        ]);
        rewrite_read_uris("docs", &mut result);
        let uris: Vec<_> = result
            .contents
            .iter_mut()
            .filter_map(content_uri_mut)
            .map(|uri| uri.as_str())
            .collect();
        assert_eq!(
            uris,
            ["asterlane://docs/file:///a", "asterlane://docs/file:///b"]
        );
    }
}
