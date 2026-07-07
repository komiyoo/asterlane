import { api, objTable } from "../core.js";

export async function loadProxyKeys(view) {
  const rows = await api("/admin/proxy-keys");
  view.innerHTML = objTable(rows,
    ["id", "display_name", "auth_mode", "expires_at", "usage", "allowed_servers", "allowed_tool_names", "limits",
      "allowed_tools", "denied_tools", "default_tool_page_size"]);
}
