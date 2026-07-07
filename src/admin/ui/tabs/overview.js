import { api, esc } from "../core.js";

export async function loadOverview(view) {
  const [health, stats] = await Promise.all([api("/admin/health"), api("/admin/stats")]);
  const card = (num, lbl) =>
    '<div class="card"><div class="num">' + esc(num) + '</div><div class="lbl">' + esc(lbl) + "</div></div>";
  view.innerHTML = '<div class="cards">'
    + card(health.status + " · v" + health.version, "网关状态")
    + card(stats.total_requests, "请求总数")
    + card(stats.total_errors, "错误数")
    + card(stats.avg_latency_ms.toFixed(1) + " ms", "平均延迟")
    + card(stats.total_rate_limit_hits, "限流命中")
    + card(stats.unique_tools, "活跃工具")
    + card(stats.unique_proxy_keys, "活跃 proxy key")
    + card(stats.unique_resources, "活跃资源")
    + "</div>";
}
