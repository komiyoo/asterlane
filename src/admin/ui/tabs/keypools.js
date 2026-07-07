import { api, esc, objTable } from "../core.js";

export async function loadKeyPools(view) {
  const pools = await api("/admin/key-pools");
  if (!pools.length) {
    view.innerHTML = '<p class="empty">未配置 key pool（资源级 key_pool 配置见 docs/config-schema.md）</p>';
    return;
  }
  view.innerHTML = pools.map(p =>
    '<h3>' + esc(p.resource_id) + ' <span class="hint" style="padding:0">· ' + esc(p.strategy) + "</span></h3>"
    + objTable(p.keys,
      ["key_id", "state", "leased_count", "cooling_remaining_ms", "weight", "ewma_latency_ms", "ref"])
  ).join("");
}
