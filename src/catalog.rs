use crate::config::{ApiResource, GatewayConfig, ProxyKey, SpecSource};
use crate::naming::ToolName;
use crate::openapi;
use crate::policy::{PolicyError, key_can_use_tool};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedTool {
    pub name: ToolName,
    pub resource_id: String,
    pub description: String,
    pub upstream_path: String,
    /// HTTP method for upstream proxy call (from endpoint config, not wire name).
    #[serde(default = "default_http_method")]
    pub http_method: crate::config::HttpMethod,
    /// JSON Schema for MCP tool inputSchema. Default: `{"type": "object"}`.
    #[serde(default = "default_input_schema")]
    pub input_schema: serde_json::Value,
    /// Parameter location metadata for OpenAPI-discovered tools.
    /// None for hand-written endpoints (all args sent as JSON body).
    #[serde(default)]
    pub param_locations: Option<ParamLocations>,
    /// 最短无歧义暴露名（tools/list 展示用，见 [`ToolCatalog::resolve_for_key`]）。
    ///
    /// 仅在 `list_for_key` 返回页中填充；catalog 存储态恒为 `None`。
    /// 在 key scope 可见全集上计算（request filter 之前——过滤不改名）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposed_name: Option<String>,
}

fn default_http_method() -> crate::config::HttpMethod {
    crate::config::HttpMethod::Post
}

fn default_input_schema() -> serde_json::Value {
    serde_json::json!({"type": "object"})
}

/// Tracks where each input parameter should be placed when proxying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamLocations {
    pub path_params: Vec<String>,
    pub query_params: Vec<String>,
    /// (input_schema_field_name, actual_header_name)
    pub header_params: Vec<(String, String)>,
    pub has_body: bool,
}

/// [`ToolCatalog::resolve_for_key`] 的可选限定段。
///
/// 给出的字段在**所有**解析层都要求与候选工具的 domain/provider 相等，
/// 用于把歧义收窄为唯一命中。
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolQualifiers<'a> {
    pub domain: Option<&'a str>,
    pub provider: Option<&'a str>,
}

/// 歧义错误中最多列出的候选数。
const AMBIGUITY_CANDIDATE_LIMIT: usize = 8;

/// meta-tool 保留前缀：暴露名不得以此开头（会与网关自身工具混淆）。
const META_TOOL_PREFIX: &str = "asterlane__";

/// 工具目录。
///
/// `tools` 中的 `description` 为**有效描述**：管理员介绍 override 已应用
/// （见 docs/mcp-governance-and-key-limits.md §5）。所有读路径
/// （`/v1/tools`、MCP `tools/list`、meta-tool 搜索、语义索引）因此自动
/// 输出 `override ?? 上游原始`，无需各自处理。上游原始描述保存在
/// `original_descriptions`，仅供 admin 端点展示；integrity baseline 不经
/// catalog 取描述（用 registry descriptors），override 不参与 fingerprint。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCatalog {
    tools: Vec<WrappedTool>,
    /// wire name → 管理员介绍 override（来源 `tool_metadata` 表）。
    description_overrides: HashMap<String, String>,
    /// wire name → 上游原始描述（仅记录被 override 的工具）。
    /// 工具下线后的残留条目无害：读取只按 catalog 现存工具查询，
    /// 工具重新上线时 `overlay_tool` 会以新的原始描述覆盖。
    original_descriptions: HashMap<String, String>,
}

impl ToolCatalog {
    pub fn from_config(config: &GatewayConfig) -> Result<Self, CatalogError> {
        let mut tools = Vec::new();
        for resource in &config.api_resources {
            // Hand-written endpoints
            tools.extend(tools_for_resource(resource)?);
            // OpenAPI discovery
            if let Some(discovery) = &resource.discovery {
                tools.extend(tools_from_openapi(resource, discovery)?);
            }
        }
        tools.sort_by_key(|a| a.name.to_wire_name());
        Ok(Self {
            tools,
            description_overrides: HashMap::new(),
            original_descriptions: HashMap::new(),
        })
    }

    pub fn extend_with_mcp_tools(&mut self, tools: impl IntoIterator<Item = WrappedTool>) {
        for mut tool in tools {
            overlay_tool(
                &self.description_overrides,
                &mut self.original_descriptions,
                &mut tool,
            );
            self.tools.push(tool);
        }
        self.tools.sort_by_key(|a| a.name.to_wire_name());
    }

    /// 用新的 MCP 工具快照替换 catalog 中由 `mcp_resource_ids` 标记的远程 MCP 工具。
    ///
    /// refresh 后调用：先移除所有 `resource_id` 在给定集合中的旧工具，
    /// 再 extend 新工具并重排序。HTTP API 工具（非 MCP）不受影响。
    /// 保持 `list_for_key` 的过滤/scope 逻辑不变——仅替换数据源。
    /// 介绍 override 在 extend 时按 overlay map 重放，替换后不丢失。
    pub fn replace_mcp_tools(
        &mut self,
        new_tools: Vec<WrappedTool>,
        mcp_resource_ids: &std::collections::HashSet<String>,
    ) {
        self.tools
            .retain(|t| !mcp_resource_ids.contains(&t.resource_id));
        self.extend_with_mcp_tools(new_tools);
    }

    // ── 介绍 override（治理契约 §5）──

    /// 写入/更新单个工具的介绍 override，并即时应用到现有工具。
    /// 工具当前不在 catalog（如上游暂不可达）时仅登记，工具出现后重放。
    pub fn set_description_override(&mut self, wire_name: &str, description: &str) {
        self.description_overrides
            .insert(wire_name.to_string(), description.to_string());
        if let Some(tool) = self
            .tools
            .iter_mut()
            .find(|t| t.name.to_wire_name() == wire_name)
        {
            // 首次 override 时记录原始描述；再次覆盖时原始描述不变
            self.original_descriptions
                .entry(wire_name.to_string())
                .or_insert_with(|| tool.description.clone());
            tool.description = description.to_string();
        }
    }

    /// 移除介绍 override，恢复上游原始描述。
    pub fn remove_description_override(&mut self, wire_name: &str) {
        self.description_overrides.remove(wire_name);
        if let Some(original) = self.original_descriptions.remove(wire_name)
            && let Some(tool) = self
                .tools
                .iter_mut()
                .find(|t| t.name.to_wire_name() == wire_name)
        {
            tool.description = original;
        }
    }

    /// 整体加载 override 集合（启动时从 store 读取；配置热更新时从旧
    /// catalog 携带）。可重入：先还原已应用的 override 再全量重放。
    pub fn load_description_overrides(&mut self, overrides: HashMap<String, String>) {
        for tool in &mut self.tools {
            if let Some(original) = self.original_descriptions.remove(&tool.name.to_wire_name()) {
                tool.description = original;
            }
        }
        self.original_descriptions.clear();
        self.description_overrides = overrides;
        for tool in &mut self.tools {
            overlay_tool(
                &self.description_overrides,
                &mut self.original_descriptions,
                tool,
            );
        }
    }

    /// 当前 override 集合（wire name → 介绍）。
    pub fn description_overrides(&self) -> &HashMap<String, String> {
        &self.description_overrides
    }

    /// 某工具的介绍 override（若有）。
    pub fn description_override(&self, wire_name: &str) -> Option<&str> {
        self.description_overrides
            .get(wire_name)
            .map(String::as_str)
    }

    /// 某工具的上游原始描述。无 override 时工具自身 `description` 即原始，
    /// 返回 `None`——调用方以 `original_description(w).unwrap_or(&t.description)` 取原始。
    pub fn original_description(&self, wire_name: &str) -> Option<&str> {
        self.original_descriptions
            .get(wire_name)
            .map(String::as_str)
    }

    pub fn list_for_key(
        &self,
        key: &ProxyKey,
        query: &ToolListQuery,
    ) -> Result<ToolPage, CatalogError> {
        // 先编译所有正则（无效正则按 CatalogError 上报）
        let include = compile_optional_regex(&query.include_regex)?;
        let exclude = compile_optional_regex(&query.exclude_regex)?;
        let domain_re = compile_optional_regex(&query.domain_regex)?;
        let provider_re = compile_optional_regex(&query.provider_regex)?;
        let tool_re = compile_optional_regex(&query.tool_regex)?;
        let limit = query.limit.unwrap_or(key.default_tool_page_size).max(1);
        let cursor = query.cursor.unwrap_or(0);

        // 1. key scope 可见全集（收窄不扩张：request filter 只能在其内进一步收窄）。
        //    暴露名在**这一集合**上计算——请求级过滤只决定哪些条目出现，
        //    不改变条目的名字（过滤不改名）。
        let mut visible = Vec::new();
        for tool in &self.tools {
            if key_can_use_tool(key, &tool.name, &tool.resource_id)? {
                visible.push(tool);
            }
        }

        // 2. request filter：include/exclude 作用于 wire name，结构化过滤按段
        let filtered = visible.iter().copied().filter(|tool| {
            let full_name = tool.name.to_wire_name();
            include
                .as_ref()
                .is_none_or(|regex| regex.is_match(&full_name))
                && !exclude
                    .as_ref()
                    .is_some_and(|regex| regex.is_match(&full_name))
                && domain_re
                    .as_ref()
                    .is_none_or(|regex| regex.is_match(&tool.name.domain))
                && provider_re
                    .as_ref()
                    .is_none_or(|regex| regex.is_match(&tool.name.provider))
                && tool_re
                    .as_ref()
                    .is_none_or(|regex| regex.is_match(&tool.name.tool))
        });

        // 3. 分页，并为页内工具填充最短无歧义暴露名
        let page = filtered
            .skip(cursor)
            .take(limit)
            .map(|tool| {
                let mut entry = tool.clone();
                entry.exposed_name = Some(self.shortest_exposed_name(tool, &visible));
                entry
            })
            .collect::<Vec<_>>();
        let next_cursor = if page.len() == limit {
            Some(cursor + limit)
        } else {
            None
        };

        Ok(ToolPage {
            tools: page,
            next_cursor,
        })
    }

    /// 三级工具名解析（调用侧唯一入口）。
    ///
    /// 按优先级 canonical 全名 → `provider__tool` 两段 → 裸名 tool 逐层匹配，
    /// 命中即返回、不落下层。段内可能含 `__`（MCP 上游原名），因此一律
    /// 字符串查表，不做 `__` 切分 parse（见 `naming.rs` 文档注释）。
    ///
    /// - Tier0 canonical：范围为**全目录**、不经 key scope——scope 拒绝
    ///   留给 executor 的 policy 检查，保持既有错误语义。
    /// - Tier1 两段 / Tier2 裸名：范围为 key 可见工具，scope 外的工具
    ///   不参与匹配（裸名不泄漏 scope 外工具的存在性）。
    /// - 同层候选数 >1 → [`CatalogError::AmbiguousToolName`]；
    ///   全部无命中 → `Ok(None)`。
    pub fn resolve_for_key(
        &self,
        name: &str,
        qualifiers: ToolQualifiers<'_>,
        key: &ProxyKey,
    ) -> Result<Option<&WrappedTool>, CatalogError> {
        // Tier0 canonical
        if let Some(tool) =
            self.resolve_tier(name, qualifiers, None, |t| t.name.to_wire_name() == name)?
        {
            return Ok(Some(tool));
        }
        // Tier1 两段 provider__tool
        if let Some(tool) = self.resolve_tier(name, qualifiers, Some(key), |t| {
            two_segment_name_matches(&t.name, name)
        })? {
            return Ok(Some(tool));
        }
        // Tier2 裸名 tool
        self.resolve_tier(name, qualifiers, Some(key), |t| t.name.tool == name)
    }

    /// 单层解析：按 `matches` 收集候选（`key` 给定时限 key 可见工具），
    /// 恰好 1 个 → 命中；>1 → 歧义错误；0 → `None`（调用方落入下一层）。
    fn resolve_tier(
        &self,
        name: &str,
        qualifiers: ToolQualifiers<'_>,
        key: Option<&ProxyKey>,
        matches: impl Fn(&WrappedTool) -> bool,
    ) -> Result<Option<&WrappedTool>, CatalogError> {
        let mut candidates = Vec::new();
        for tool in &self.tools {
            if !matches(tool) || !qualifiers_match(qualifiers, &tool.name) {
                continue;
            }
            if let Some(key) = key
                && !key_can_use_tool(key, &tool.name, &tool.resource_id)?
            {
                continue;
            }
            candidates.push(tool);
        }
        match candidates.len() {
            0 => Ok(None),
            1 => Ok(Some(candidates[0])),
            _ => {
                let mut names: Vec<String> =
                    candidates.iter().map(|t| t.name.to_wire_name()).collect();
                names.sort();
                names.truncate(AMBIGUITY_CANDIDATE_LIMIT);
                Err(CatalogError::AmbiguousToolName {
                    name: name.to_string(),
                    candidates: names,
                })
            }
        }
    }

    /// key scope 可见全集内该工具的最短无歧义暴露名：
    /// 裸名 tool → 两段 `provider__tool` → canonical 兜底（总是有效）。
    ///
    /// 候选有效条件（全部满足）：
    /// - `visible` 内恰好 1 个工具以该字符串为裸名或两段名（跨形式遮蔽也算冲突）；
    /// - 不等于**任何**工具（含 key 不可见的）的 canonical wire name——
    ///   否则 Tier0 精确匹配会遮蔽它，列表展示的名字将解析到别的工具；
    /// - 不以 [`META_TOOL_PREFIX`] 开头。
    ///
    /// 保证经 [`Self::resolve_for_key`]（同 key、空 qualifiers）解析回同一工具。
    fn shortest_exposed_name(&self, tool: &WrappedTool, visible: &[&WrappedTool]) -> String {
        let two_segment = format!("{}__{}", tool.name.provider, tool.name.tool);
        for candidate in [tool.name.tool.as_str(), two_segment.as_str()] {
            if candidate.starts_with(META_TOOL_PREFIX) {
                continue;
            }
            if self
                .tools
                .iter()
                .any(|t| t.name.to_wire_name() == candidate)
            {
                continue;
            }
            let users = visible
                .iter()
                .filter(|t| {
                    t.name.tool == candidate || two_segment_name_matches(&t.name, candidate)
                })
                .count();
            // 唯一使用者必是 tool 自身（candidate 由 tool 派生，至少匹配自己）
            if users == 1 {
                return candidate.to_string();
            }
        }
        tool.name.to_wire_name()
    }

    /// 按 wire name 查找工具（不经 key scope，用于 proxy 执行层定位上游调用）。
    pub fn find_by_wire_name(&self, wire_name: &str) -> Option<&WrappedTool> {
        self.tools
            .iter()
            .find(|t| t.name.to_wire_name() == wire_name)
    }

    /// 返回 catalog 中的工具总数（不经 key scope）。
    pub fn total_tool_count(&self) -> usize {
        self.tools.len()
    }

    /// 返回所有工具的只读切片（不经 key scope，供 admin API 使用）。
    pub fn all_tools(&self) -> &[WrappedTool] {
        &self.tools
    }

    /// 统计某 key 可见的工具数。
    pub fn count_visible_for_key(&self, key: &ProxyKey) -> Result<usize, CatalogError> {
        let mut count = 0;
        for tool in &self.tools {
            if key_can_use_tool(key, &tool.name, &tool.resource_id)? {
                count += 1;
            }
        }
        Ok(count)
    }

    /// 按关键词搜索 key 可见的工具（substring match on wire_name and description）。
    ///
    /// 返回前 `limit` 条匹配结果。空 query 匹配所有可见工具。
    pub fn search_for_key(
        &self,
        query: &str,
        key: &ProxyKey,
        limit: usize,
    ) -> Result<Vec<&WrappedTool>, CatalogError> {
        if query.is_empty() {
            let mut results = Vec::new();
            for tool in &self.tools {
                if !key_can_use_tool(key, &tool.name, &tool.resource_id)? {
                    continue;
                }
                results.push(tool);
                if results.len() >= limit {
                    break;
                }
            }
            return Ok(results);
        }
        let query_lower = query.to_lowercase();
        let mut scored: Vec<(&WrappedTool, u8)> = Vec::new();
        for tool in &self.tools {
            if !key_can_use_tool(key, &tool.name, &tool.resource_id)? {
                continue;
            }
            let wire = tool.name.to_wire_name().to_lowercase();
            let score = if wire == query_lower {
                4 // exact match
            } else if wire.starts_with(&query_lower) {
                3 // prefix
            } else if wire.contains(&query_lower) {
                2 // name contains
            } else if tool.description.to_lowercase().contains(&query_lower) {
                1 // description contains
            } else {
                continue;
            };
            scored.push((tool, score));
        }
        scored.sort_by_key(|s| std::cmp::Reverse(s.1));
        scored.truncate(limit);
        Ok(scored.into_iter().map(|(t, _)| t).collect())
    }
}

/// 对一个**携带上游原始描述**的工具应用介绍 override：
/// 命中 overlay map 时把原始描述记入 `originals` 并替换为 override。
/// 输入约定为原始描述（catalog 的所有工具入口均满足），因此总是覆盖记录，
/// 上游原始描述变化后 admin 展示随之更新。
fn overlay_tool(
    overrides: &HashMap<String, String>,
    originals: &mut HashMap<String, String>,
    tool: &mut WrappedTool,
) {
    let wire_name = tool.name.to_wire_name();
    if let Some(override_desc) = overrides.get(&wire_name) {
        originals.insert(
            wire_name,
            std::mem::replace(&mut tool.description, override_desc.clone()),
        );
    }
}

/// `name` 是否等于该工具的两段形式 `provider__tool`。
///
/// 段内可含 `__`，等价于与 `format!("{provider}__{tool}")` 整串相等
/// （免分配写法）。歧义（多个 (provider, tool) 切分都成立）由调用方
/// 按候选计数处理。
fn two_segment_name_matches(tool_name: &ToolName, name: &str) -> bool {
    name.strip_prefix(&tool_name.provider)
        .and_then(|rest| rest.strip_prefix("__"))
        == Some(tool_name.tool.as_str())
}

fn qualifiers_match(qualifiers: ToolQualifiers<'_>, name: &ToolName) -> bool {
    qualifiers.domain.is_none_or(|d| d == name.domain)
        && qualifiers.provider.is_none_or(|p| p == name.provider)
}

fn compile_optional_regex(pattern: &Option<String>) -> Result<Option<Regex>, CatalogError> {
    match pattern {
        Some(p) => Ok(Some(Regex::new(p)?)),
        None => Ok(None),
    }
}

fn tools_for_resource(resource: &ApiResource) -> Result<Vec<WrappedTool>, CatalogError> {
    resource
        .endpoints
        .iter()
        .map(|endpoint| {
            let name = ToolName::new(&resource.domain, resource.provider_or_id(), &endpoint.tool)?;
            Ok(WrappedTool {
                name,
                resource_id: resource.id.clone(),
                description: endpoint.description.clone(),
                upstream_path: endpoint.path.clone(),
                http_method: endpoint.method,
                input_schema: default_input_schema(),
                param_locations: None,
                exposed_name: None,
            })
        })
        .collect()
}

fn tools_from_openapi(
    resource: &ApiResource,
    discovery: &crate::config::DiscoveryConfig,
) -> Result<Vec<WrappedTool>, CatalogError> {
    let spec_bytes = match discovery.openapi.source {
        SpecSource::File => {
            let path = discovery.openapi.path.as_deref().ok_or_else(|| {
                CatalogError::OpenApi(openapi::OpenApiError::ParseError(
                    "discovery.openapi.path required when source=file".to_string(),
                ))
            })?;
            std::fs::read(path).map_err(|e| {
                CatalogError::OpenApi(openapi::OpenApiError::ParseError(format!(
                    "cannot read spec file {path}: {e}"
                )))
            })?
        }
        // ponytail: URL source deferred — caller would fetch and pass bytes.
        // For now, error out; URL fetching belongs in an async startup path.
        SpecSource::Url => {
            return Err(CatalogError::OpenApi(openapi::OpenApiError::ParseError(
                "discovery.openapi.source=url not yet supported (use file)".to_string(),
            )));
        }
    };

    let config = openapi::OpenApiDiscoveryConfig {
        include_tags: discovery.openapi.include_tags.clone(),
        exclude_operations: discovery.openapi.exclude_operations.clone(),
        default_method_exposure: discovery.openapi.default_method_exposure.clone(),
        ..Default::default()
    };

    let endpoints = openapi::discover_endpoints(&spec_bytes, &config)?;

    endpoints
        .into_iter()
        .map(|ep| {
            let name = ToolName::new(
                &resource.domain,
                resource.provider_or_id(),
                &ep.tool_segment,
            )?;
            let http_method = match ep.method.as_str() {
                "get" => crate::config::HttpMethod::Get,
                "put" => crate::config::HttpMethod::Put,
                "patch" => crate::config::HttpMethod::Patch,
                "delete" => crate::config::HttpMethod::Delete,
                _ => crate::config::HttpMethod::Post,
            };
            Ok(WrappedTool {
                name,
                resource_id: resource.id.clone(),
                description: ep.description,
                upstream_path: ep.path,
                http_method,
                input_schema: ep.input_schema,
                param_locations: Some(ep.param_locations),
                exposed_name: None,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolListQuery {
    pub include_regex: Option<String>,
    pub exclude_regex: Option<String>,
    pub domain_regex: Option<String>,
    pub provider_regex: Option<String>,
    pub tool_regex: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPage {
    pub tools: Vec<WrappedTool>,
    pub next_cursor: Option<usize>,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error(transparent)]
    ToolName(#[from] crate::naming::ToolNameError),
    #[error(transparent)]
    Regex(#[from] regex::Error),
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    OpenApi(#[from] openapi::OpenApiError),
    /// 工具名在同一解析层匹配到多个候选（见 `ToolCatalog::resolve_for_key`）。
    /// `candidates` 为候选 canonical wire name，排序后截断
    /// `AMBIGUITY_CANDIDATE_LIMIT`（8）个，消息可直接给用户看。
    #[error("ambiguous tool name '{name}', use a longer form; candidates: {}", .candidates.join(", "))]
    AmbiguousToolName {
        name: String,
        candidates: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HttpMethod, SecurityConfig, ToolEndpoint, UpstreamAuth};

    fn config() -> GatewayConfig {
        GatewayConfig {
            defaults: Default::default(),
            admin: Default::default(),
            semantic_search: None,
            observability: Default::default(),
            builtin_mcp: Vec::new(),
            api_resources: vec![
                ApiResource {
                    id: "tavily".to_string(),
                    domain: "search".to_string(),
                    provider: "tavily".to_string(),
                    base_url: "https://api.tavily.com".to_string(),
                    description: "Tavily search".to_string(),
                    auth: UpstreamAuth::Bearer {
                        token_ref: "secret://tavily/default".to_string(),
                    },
                    endpoints: vec![ToolEndpoint {
                        tool: "web_search".to_string(),
                        method: HttpMethod::Post,
                        path: "/search".to_string(),
                        description: "Search web with Tavily".to_string(),
                    }],
                    key_pool: None,
                    discovery: None,
                    security: SecurityConfig::default(),
                    limits: None,
                },
                ApiResource {
                    id: "exa".to_string(),
                    domain: "search".to_string(),
                    provider: "exa".to_string(),
                    base_url: "https://api.exa.ai".to_string(),
                    description: "Exa search".to_string(),
                    auth: UpstreamAuth::Header {
                        name: "x-api-key".to_string(),
                        value_ref: "secret://exa/default".to_string(),
                    },
                    endpoints: vec![ToolEndpoint {
                        tool: "neural_search".to_string(),
                        method: HttpMethod::Post,
                        path: "/search".to_string(),
                        description: "Search web with Exa".to_string(),
                    }],
                    key_pool: None,
                    discovery: None,
                    security: SecurityConfig::default(),
                    limits: None,
                },
            ],
            mcp_servers: Vec::new(),
            proxy_keys: vec![ProxyKey {
                id: "agent-search".to_string(),
                display_name: "Search Agent".to_string(),
                allowed_tools: vec![r"^search:.*".to_string()],
                denied_tools: vec![r"^search:exa:.*".to_string()],
                default_tool_page_size: 1,
                discovery_mode: None,
                response_format: None,
                allowed_servers: Vec::new(),
                allowed_tool_names: Vec::new(),
                limits: None,
                token_ref: None,
                token_digest: None,
                expires_at: None,
            }],
        }
    }

    #[test]
    fn builds_wrapped_tools_from_api_resources() {
        let catalog = ToolCatalog::from_config(&config()).unwrap();
        assert_eq!(catalog.tools.len(), 2);
        assert_eq!(
            catalog.tools[0].name.to_wire_name(),
            "search__exa__neural_search"
        );
        assert_eq!(
            catalog.tools[1].name.to_wire_name(),
            "search__tavily__web_search"
        );
    }

    #[test]
    fn lists_tools_by_key_scope_and_denies_overrides() {
        let config = config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let page = catalog
            .list_for_key(key, &ToolListQuery::default())
            .unwrap();
        assert_eq!(page.tools.len(), 1);
        assert_eq!(
            page.tools[0].name.to_wire_name(),
            "search__tavily__web_search"
        );
        assert_eq!(page.next_cursor, Some(1));
    }

    #[test]
    fn filters_visible_tools_with_include_regex() {
        let mut config = config();
        config.proxy_keys[0].denied_tools.clear();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            include_regex: Some("tavily".to_string()),
            limit: Some(10),
            cursor: None,
            exclude_regex: None,
            domain_regex: None,
            provider_regex: None,
            tool_regex: None,
        };
        let page = catalog.list_for_key(key, &query).unwrap();
        assert_eq!(page.tools.len(), 1);
        assert_eq!(
            page.tools[0].name.to_wire_name(),
            "search__tavily__web_search"
        );
    }

    #[test]
    fn filters_by_provider_regex() {
        let mut config = config();
        config.proxy_keys[0].denied_tools.clear();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            provider_regex: Some("^exa$".to_string()),
            limit: Some(10),
            ..Default::default()
        };
        let page = catalog.list_for_key(key, &query).unwrap();
        assert_eq!(page.tools.len(), 1);
        assert_eq!(
            page.tools[0].name.to_wire_name(),
            "search__exa__neural_search"
        );
    }

    #[test]
    fn filters_by_domain_regex() {
        let mut config = config();
        config.proxy_keys[0].denied_tools.clear();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            domain_regex: Some("^search$".to_string()),
            limit: Some(10),
            ..Default::default()
        };
        let page = catalog.list_for_key(key, &query).unwrap();
        assert_eq!(page.tools.len(), 2);
    }

    #[test]
    fn invalid_regex_returns_error() {
        let config = config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = config.proxy_key("agent-search").unwrap();
        let query = ToolListQuery {
            domain_regex: Some("[invalid".to_string()),
            ..Default::default()
        };
        assert!(catalog.list_for_key(key, &query).is_err());
    }

    // ── replace_mcp_tools ──

    fn mcp_tool(wire: &str, resource_id: &str) -> WrappedTool {
        let name: ToolName = wire.parse().unwrap();
        WrappedTool {
            name,
            resource_id: resource_id.to_string(),
            description: "mcp tool".to_string(),
            upstream_path: "upstream".to_string(),
            http_method: HttpMethod::Post,
            input_schema: serde_json::json!({"type": "object"}),
            param_locations: None,
            exposed_name: None,
        }
    }

    #[test]
    fn replace_mcp_tools_swaps_only_mcp_entries() {
        let config = config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        // 初始有 2 个 HTTP API 工具
        assert_eq!(catalog.tools.len(), 2);

        // 添加 mcp tools
        catalog.extend_with_mcp_tools(vec![
            mcp_tool("travel__rollinggo__search", "rollinggo"),
            mcp_tool("travel__exa__fetch", "exa-mcp"),
        ]);
        assert_eq!(catalog.tools.len(), 4);

        // replace：rollinggo 工具变化，exa-mcp 下线
        let mut mcp_ids = std::collections::HashSet::new();
        mcp_ids.insert("rollinggo".to_string());
        mcp_ids.insert("exa-mcp".to_string());
        catalog.replace_mcp_tools(
            vec![mcp_tool("travel__rollinggo__searchv2", "rollinggo")],
            &mcp_ids,
        );

        // HTTP API 工具保留（2），旧 mcp 清除，新 mcp 加入（1）
        assert_eq!(catalog.tools.len(), 3);
        let wire_names: Vec<String> = catalog
            .tools
            .iter()
            .map(|t| t.name.to_wire_name())
            .collect();
        assert!(wire_names.contains(&"search__tavily__web_search".to_string()));
        assert!(wire_names.contains(&"search__exa__neural_search".to_string()));
        assert!(wire_names.contains(&"travel__rollinggo__searchv2".to_string()));
        // 旧 mcp 工具已移除
        assert!(!wire_names.contains(&"travel__rollinggo__search".to_string()));
        assert!(!wire_names.contains(&"travel__exa__fetch".to_string()));
    }

    #[test]
    fn search_ranks_exact_name_above_description_match() {
        let config = config();
        let catalog = ToolCatalog::from_config(&config).unwrap();
        let key = &config.proxy_keys[0];
        // "web_search" matches tavily wire name exactly; exa description also contains "search"
        let results = catalog.search_for_key("web_search", key, 10).unwrap();
        assert!(!results.is_empty());
        // tavily (name contains "web_search") should rank above exa (description contains "search")
        assert_eq!(results[0].name.to_wire_name(), "search__tavily__web_search");
    }

    // ── 介绍 override overlay（治理契约 §5）──

    #[test]
    fn override_applies_to_existing_tool_and_restores_on_remove() {
        let config = config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        let wire = "search__tavily__web_search";

        catalog.set_description_override(wire, "管理员介绍");
        let tool = catalog.find_by_wire_name(wire).unwrap();
        assert_eq!(tool.description, "管理员介绍");
        assert_eq!(
            catalog.original_description(wire),
            Some("Search web with Tavily")
        );
        assert_eq!(catalog.description_override(wire), Some("管理员介绍"));

        // 二次覆盖不破坏原始描述
        catalog.set_description_override(wire, "修订介绍");
        assert_eq!(
            catalog.original_description(wire),
            Some("Search web with Tavily")
        );

        catalog.remove_description_override(wire);
        let tool = catalog.find_by_wire_name(wire).unwrap();
        assert_eq!(tool.description, "Search web with Tavily");
        assert_eq!(catalog.original_description(wire), None);
        assert_eq!(catalog.description_override(wire), None);
    }

    #[test]
    fn override_survives_replace_mcp_tools() {
        let config = config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        catalog.extend_with_mcp_tools(vec![mcp_tool("travel__rollinggo__search", "rollinggo")]);
        catalog.set_description_override("travel__rollinggo__search", "override 介绍");

        // refresh 重建：registry 快照的描述总是上游原始
        let mut mcp_ids = std::collections::HashSet::new();
        mcp_ids.insert("rollinggo".to_string());
        catalog.replace_mcp_tools(
            vec![mcp_tool("travel__rollinggo__search", "rollinggo")],
            &mcp_ids,
        );

        let tool = catalog
            .find_by_wire_name("travel__rollinggo__search")
            .unwrap();
        assert_eq!(tool.description, "override 介绍");
        assert_eq!(
            catalog.original_description("travel__rollinggo__search"),
            Some("mcp tool")
        );
    }

    #[test]
    fn load_description_overrides_is_reentrant() {
        let config = config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        let wire = "search__exa__neural_search";

        let mut first = HashMap::new();
        first.insert(wire.to_string(), "第一版".to_string());
        catalog.load_description_overrides(first);
        assert_eq!(
            catalog.find_by_wire_name(wire).unwrap().description,
            "第一版"
        );

        // 重载为另一集合：旧 override 还原，新 override 应用
        let mut second = HashMap::new();
        second.insert(
            "search__tavily__web_search".to_string(),
            "tavily 介绍".to_string(),
        );
        catalog.load_description_overrides(second);
        assert_eq!(
            catalog.find_by_wire_name(wire).unwrap().description,
            "Search web with Exa"
        );
        assert_eq!(
            catalog
                .find_by_wire_name("search__tavily__web_search")
                .unwrap()
                .description,
            "tavily 介绍"
        );
        assert_eq!(catalog.original_description(wire), None);
    }

    #[test]
    fn replace_mcp_tools_empty_new_clears_all_mcp() {
        let config = config();
        let mut catalog = ToolCatalog::from_config(&config).unwrap();
        catalog.extend_with_mcp_tools(vec![mcp_tool("travel__rollinggo__search", "rollinggo")]);
        assert_eq!(catalog.tools.len(), 3);

        let mut mcp_ids = std::collections::HashSet::new();
        mcp_ids.insert("rollinggo".to_string());
        catalog.replace_mcp_tools(Vec::new(), &mcp_ids);

        // 只剩 HTTP API 工具
        assert_eq!(catalog.tools.len(), 2);
    }

    // ── resolve_for_key:三级解析 + 暴露名 ──

    /// 直接按三段构造（不经 `FromStr`，支持 tool 段含 `__`）。
    fn tool3(domain: &str, provider: &str, tool: &str, resource_id: &str) -> WrappedTool {
        WrappedTool {
            name: ToolName::new(domain, provider, tool).unwrap(),
            resource_id: resource_id.to_string(),
            description: "t".to_string(),
            upstream_path: "p".to_string(),
            http_method: HttpMethod::Post,
            input_schema: serde_json::json!({"type": "object"}),
            param_locations: None,
            exposed_name: None,
        }
    }

    fn catalog_of(tools: Vec<WrappedTool>) -> ToolCatalog {
        let mut catalog = ToolCatalog {
            tools: Vec::new(),
            description_overrides: HashMap::new(),
            original_descriptions: HashMap::new(),
        };
        catalog.extend_with_mcp_tools(tools);
        catalog
    }

    fn key_with_scope(allowed: &[&str]) -> ProxyKey {
        ProxyKey {
            id: "k".to_string(),
            display_name: "k".to_string(),
            allowed_tools: allowed.iter().map(|s| s.to_string()).collect(),
            denied_tools: Vec::new(),
            default_tool_page_size: 50,
            discovery_mode: None,
            response_format: None,
            allowed_servers: Vec::new(),
            allowed_tool_names: Vec::new(),
            limits: None,
            token_ref: None,
            token_digest: None,
            expires_at: None,
        }
    }

    /// 解析夹具：tavily/exa 裸名冲突、tavily 两段跨 domain 冲突、
    /// yt 在 key scope 外、gh 工具段含 `__`、neural 全局唯一裸名。
    fn resolve_fixture() -> (ToolCatalog, ProxyKey) {
        let catalog = catalog_of(vec![
            tool3("search", "tavily", "web_search", "tavily"),
            tool3("search", "exa", "web_search", "exa"),
            tool3("search", "exa", "neural", "exa"),
            tool3("news", "tavily", "web_search", "tavily-news"),
            tool3("video", "yt", "transcribe", "yt"),
            tool3("mcp", "gh", "issues__list", "gh"),
        ]);
        let key = key_with_scope(&["^search:", "^news:", "^mcp:"]);
        (catalog, key)
    }

    fn resolve<'c>(
        catalog: &'c ToolCatalog,
        key: &ProxyKey,
        name: &str,
        qualifiers: ToolQualifiers<'_>,
    ) -> Result<Option<&'c WrappedTool>, CatalogError> {
        catalog.resolve_for_key(name, qualifiers, key)
    }

    fn wire(tool: Option<&WrappedTool>) -> Option<String> {
        tool.map(|t| t.name.to_wire_name())
    }

    #[test]
    fn resolve_canonical_hits_and_skips_key_scope() {
        let (catalog, key) = resolve_fixture();
        let quals = ToolQualifiers::default();
        assert_eq!(
            wire(resolve(&catalog, &key, "search__tavily__web_search", quals).unwrap()),
            Some("search__tavily__web_search".to_string())
        );
        // Tier0 不经 key scope：scope 外工具的 canonical 也命中（拒绝留给 policy 检查）
        assert_eq!(
            wire(resolve(&catalog, &key, "video__yt__transcribe", quals).unwrap()),
            Some("video__yt__transcribe".to_string())
        );
    }

    #[test]
    fn resolve_two_segment_hits() {
        let (catalog, key) = resolve_fixture();
        assert_eq!(
            wire(resolve(&catalog, &key, "exa__web_search", ToolQualifiers::default()).unwrap()),
            Some("search__exa__web_search".to_string())
        );
    }

    #[test]
    fn resolve_bare_name_hits() {
        let (catalog, key) = resolve_fixture();
        assert_eq!(
            wire(resolve(&catalog, &key, "neural", ToolQualifiers::default()).unwrap()),
            Some("search__exa__neural".to_string())
        );
    }

    #[test]
    fn resolve_ambiguous_reports_sorted_candidates() {
        let (catalog, key) = resolve_fixture();
        let err = resolve(&catalog, &key, "web_search", ToolQualifiers::default()).unwrap_err();
        match err {
            CatalogError::AmbiguousToolName { name, candidates } => {
                assert_eq!(name, "web_search");
                assert_eq!(
                    candidates,
                    vec![
                        "news__tavily__web_search".to_string(),
                        "search__exa__web_search".to_string(),
                        "search__tavily__web_search".to_string(),
                    ]
                );
            }
            other => panic!("expected AmbiguousToolName, got {other:?}"),
        }
    }

    #[test]
    fn resolve_ambiguity_candidates_capped_at_8() {
        let tools = (0..10)
            .map(|i| tool3("d", &format!("p{i}"), "same", "r"))
            .collect();
        let catalog = catalog_of(tools);
        let key = key_with_scope(&["^d:"]);
        let err = resolve(&catalog, &key, "same", ToolQualifiers::default()).unwrap_err();
        match err {
            CatalogError::AmbiguousToolName { candidates, .. } => {
                assert_eq!(candidates.len(), 8);
                assert!(candidates.windows(2).all(|w| w[0] <= w[1]));
                assert_eq!(candidates[0], "d__p0__same");
            }
            other => panic!("expected AmbiguousToolName, got {other:?}"),
        }
    }

    #[test]
    fn resolve_qualifiers_narrow_ambiguity() {
        let (catalog, key) = resolve_fixture();
        // 裸名歧义 → provider 收窄为唯一
        let quals = ToolQualifiers {
            provider: Some("exa"),
            ..Default::default()
        };
        assert_eq!(
            wire(resolve(&catalog, &key, "web_search", quals).unwrap()),
            Some("search__exa__web_search".to_string())
        );
        // 两段歧义（tavily__web_search 跨 domain）→ domain 收窄
        let quals = ToolQualifiers {
            domain: Some("news"),
            ..Default::default()
        };
        assert_eq!(
            wire(resolve(&catalog, &key, "tavily__web_search", quals).unwrap()),
            Some("news__tavily__web_search".to_string())
        );
    }

    #[test]
    fn resolve_qualifier_mismatch_returns_none() {
        let (catalog, key) = resolve_fixture();
        let quals = ToolQualifiers {
            domain: Some("video"),
            ..Default::default()
        };
        assert!(resolve(&catalog, &key, "neural", quals).unwrap().is_none());
        // qualifiers 在 Tier0 同样生效
        let quals = ToolQualifiers {
            provider: Some("tavily"),
            ..Default::default()
        };
        assert!(
            resolve(&catalog, &key, "search__exa__web_search", quals)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn resolve_bare_name_out_of_scope_returns_none() {
        let (catalog, key) = resolve_fixture();
        // transcribe 只存在于 scope 外 → None，不泄漏存在性（也不报歧义）
        assert_eq!(
            wire(resolve(&catalog, &key, "transcribe", ToolQualifiers::default()).unwrap()),
            None
        );
    }

    #[test]
    fn resolve_tool_segment_with_double_underscore_all_forms() {
        let (catalog, key) = resolve_fixture();
        let quals = ToolQualifiers::default();
        for name in ["mcp__gh__issues__list", "gh__issues__list", "issues__list"] {
            assert_eq!(
                wire(resolve(&catalog, &key, name, quals).unwrap()),
                Some("mcp__gh__issues__list".to_string()),
                "form {name} should resolve"
            );
        }
    }

    // ── 暴露名（list_for_key.exposed_name）──

    fn exposed_map(
        catalog: &ToolCatalog,
        key: &ProxyKey,
        query: &ToolListQuery,
    ) -> HashMap<String, String> {
        let query = ToolListQuery {
            limit: Some(100),
            ..query.clone()
        };
        catalog
            .list_for_key(key, &query)
            .unwrap()
            .tools
            .into_iter()
            .map(|t| (t.name.to_wire_name(), t.exposed_name.unwrap()))
            .collect()
    }

    #[test]
    fn exposed_names_pick_shortest_unambiguous_form() {
        let (catalog, key) = resolve_fixture();
        let exposed = exposed_map(&catalog, &key, &ToolListQuery::default());
        // 全局唯一裸名 → tool
        assert_eq!(exposed["search__exa__neural"], "neural");
        assert_eq!(exposed["mcp__gh__issues__list"], "issues__list");
        // 跨 provider 裸名碰撞 → 两段
        assert_eq!(exposed["search__exa__web_search"], "exa__web_search");
        // 两段也碰撞（同 provider 名、不同 domain）→ canonical
        assert_eq!(
            exposed["search__tavily__web_search"],
            "search__tavily__web_search"
        );
        assert_eq!(
            exposed["news__tavily__web_search"],
            "news__tavily__web_search"
        );
        // catalog 存储态不携带暴露名
        assert!(catalog.all_tools().iter().all(|t| t.exposed_name.is_none()));
    }

    #[test]
    fn exposed_name_avoids_other_tools_canonical() {
        // A 的裸名与 B 的 canonical 相同；B 在 key scope 外仍参与遮蔽检查
        // （Tier0 精确匹配会把该名字解析到 B）
        let catalog = catalog_of(vec![
            tool3("mcp", "srv", "docs__search__find", "srv"),
            tool3("docs", "search", "find", "docs"),
        ]);
        let key = key_with_scope(&["^mcp:"]);
        let exposed = exposed_map(&catalog, &key, &ToolListQuery::default());
        assert_eq!(
            exposed["mcp__srv__docs__search__find"],
            "srv__docs__search__find"
        );
    }

    #[test]
    fn exposed_name_skips_meta_tool_prefix() {
        let catalog = catalog_of(vec![
            // 裸名以 asterlane__ 开头 → 跳过裸名，用两段
            tool3("mcp", "srv", "asterlane__ping", "srv"),
            // provider=asterlane 且裸名冲突 → 两段以 asterlane__ 开头 → canonical
            tool3("meta", "asterlane", "status", "meta"),
            tool3("ops", "mon", "status", "ops"),
        ]);
        let key = key_with_scope(&["^mcp:", "^meta:", "^ops:"]);
        let exposed = exposed_map(&catalog, &key, &ToolListQuery::default());
        assert_eq!(exposed["mcp__srv__asterlane__ping"], "srv__asterlane__ping");
        assert_eq!(
            exposed["meta__asterlane__status"],
            "meta__asterlane__status"
        );
        assert_eq!(exposed["ops__mon__status"], "mon__status");
    }

    #[test]
    fn request_filter_does_not_change_exposed_name() {
        let (catalog, key) = resolve_fixture();
        let unfiltered = exposed_map(&catalog, &key, &ToolListQuery::default());
        let by_include = exposed_map(
            &catalog,
            &key,
            &ToolListQuery {
                include_regex: Some("exa".to_string()),
                ..Default::default()
            },
        );
        let by_domain = exposed_map(
            &catalog,
            &key,
            &ToolListQuery {
                domain_regex: Some("^search$".to_string()),
                ..Default::default()
            },
        );
        for (wire_name, name) in by_include.iter().chain(by_domain.iter()) {
            assert_eq!(name, &unfiltered[wire_name], "filter 不得改名: {wire_name}");
        }
        // 过滤后 exa 只剩自己，若在过滤后的集合上计算会退化为裸名——必须仍是两段
        assert_eq!(by_include["search__exa__web_search"], "exa__web_search");
    }

    #[test]
    fn exposed_names_round_trip_through_resolve() {
        let (mut catalog, key) = resolve_fixture();
        catalog.extend_with_mcp_tools(vec![
            tool3("mcp", "srv", "asterlane__ping", "srv"),
            tool3("mcp", "srv", "docs__search__find", "srv"),
            tool3("docs", "search", "find", "docs"),
        ]);
        let page = catalog
            .list_for_key(
                &key,
                &ToolListQuery {
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!page.tools.is_empty());
        for tool in &page.tools {
            let exposed = tool.exposed_name.as_deref().unwrap();
            let resolved = catalog
                .resolve_for_key(exposed, ToolQualifiers::default(), &key)
                .unwrap_or_else(|e| panic!("exposed name {exposed} errored: {e}"))
                .unwrap_or_else(|| panic!("exposed name {exposed} resolved to nothing"));
            assert_eq!(resolved.name.to_wire_name(), tool.name.to_wire_name());
        }
    }
}
