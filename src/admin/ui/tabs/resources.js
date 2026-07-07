import { api, objTable } from "../core.js";

export async function loadResources(view) {
  const rows = await api("/admin/resources");
  view.innerHTML = objTable(rows, ["id", "domain", "provider", "base_url", "endpoint_count"]);
}
