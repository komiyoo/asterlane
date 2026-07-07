import { api, objTable } from "../core.js";

export async function loadSecurity(view) {
  const rows = await api("/admin/security-events");
  view.innerHTML = objTable(rows,
    ["timestamp", "resource_id", "tool_name", "kind", "severity", "details"],
    {timestamp: "时间", resource_id: "资源", tool_name: "工具", kind: "类型", severity: "严重度", details: "详情"});
}
