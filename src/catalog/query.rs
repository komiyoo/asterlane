//! 按 key 列工具、解析名字、搜索。

use super::{
    AMBIGUITY_CANDIDATE_LIMIT, CatalogError, META_TOOL_PREFIX, ToolCatalog, ToolListQuery,
    ToolPage, ToolQualifiers, WrappedTool, compile_optional_regex, qualifiers_match,
    two_segment_name_matches,
};
use crate::config::ProxyKey;
use crate::policy::key_can_use_tool;

impl ToolCatalog {
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
        let mut remaining = filtered.skip(cursor);
        let page = remaining
            .by_ref()
            .take(limit)
            .map(|tool| {
                let mut entry = tool.clone();
                entry.exposed_name = Some(self.shortest_exposed_name(tool, &visible));
                entry
            })
            .collect::<Vec<_>>();
        let next_cursor = remaining.next().map(|_| cursor.saturating_add(page.len()));

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
            }
            results.sort_by_key(|tool| tool.name.to_wire_name());
            results.truncate(limit);
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
        scored.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.name.to_wire_name().cmp(&b.0.name.to_wire_name()))
        });
        scored.truncate(limit);
        Ok(scored.into_iter().map(|(t, _)| t).collect())
    }
}
