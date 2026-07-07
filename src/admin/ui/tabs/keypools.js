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
      ["key_id", "state", "leased_count", "cooling_remaining_ms", "weight", "ewma_latency_ms", "ref"],
      {key_id: "密钥 ID", state: "状态", leased_count: "租借数", cooling_remaining_ms: "冷却剩余(ms)",
       weight: "权重", ewma_latency_ms: "加权延迟(ms)", ref: "引用"})
  ).join("");
}
