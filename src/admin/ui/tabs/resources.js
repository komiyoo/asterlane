import { api, objTable } from "../core.js";

export async function loadResources(view) {
  const rows = await api("/admin/resources");
  view.innerHTML = objTable(rows, ["id", "domain", "provider", "base_url", "endpoint_count"],
    {id: "ID", domain: "领域", provider: "提供商", base_url: "基础 URL", endpoint_count: "端点数"});
}
