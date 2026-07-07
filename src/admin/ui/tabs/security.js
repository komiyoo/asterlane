import { api, objTable } from "../core.js";

export async function loadSecurity(view) {
  const rows = await api("/admin/security-events");
  view.innerHTML = objTable(rows,
    ["timestamp", "resource_id", "tool_name", "kind", "severity", "details"]);
}
