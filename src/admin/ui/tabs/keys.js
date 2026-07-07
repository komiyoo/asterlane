import { api, objTable } from "../core.js";

export async function loadProxyKeys(view) {
  const rows = await api("/admin/proxy-keys");
  view.innerHTML = objTable(rows,
    ["id", "display_name", "auth_mode", "expires_at", "usage", "allowed_servers", "allowed_tool_names", "limits",
      "allowed_tools", "denied_tools", "default_tool_page_size"],
    {id: "ID", display_name: "显示名", auth_mode: "认证模式", expires_at: "到期时间",
     usage: "用量", allowed_servers: "允许的服务", allowed_tool_names: "允许的工具",
     limits: "限额", allowed_tools: "允许（正则）", denied_tools: "拒绝（正则）",
     default_tool_page_size: "页大小"});
}
