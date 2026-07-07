import { $, api, esc, objTable } from "../core.js";

export async function loadUsage(view) {
  view.innerHTML = '<div class="toolbar">'
    + '<select id="us-dim">'
    + '<option value="tool">按工具</option><option value="domain">按 domain</option>'
    + '<option value="resource">按资源</option><option value="proxy_key">按 proxy key</option>'
    + '<option value="status">按状态</option>'
    + '<option value="bucket">按小时（趋势）</option></select>'
    + '<input type="datetime-local" id="us-from"><input type="datetime-local" id="us-to">'
    + '<button id="us-go">查询</button></div>'
    + '<div id="us-chart"></div><div id="us-table"></div>';
  const run = async () => {
    const p = new URLSearchParams({ group_by: $("#us-dim").value });
    if ($("#us-from").value) p.set("from", new Date($("#us-from").value).toISOString());
    if ($("#us-to").value) p.set("to", new Date($("#us-to").value).toISOString());
    try {
      const data = await api("/admin/usage?" + p);
      const rows = data.rows;
      if (!rows.length) {
        $("#us-chart").innerHTML = "";
        $("#us-table").innerHTML = '<p class="empty">无数据</p>';
        return;
      }
      const isBucket = $("#us-dim").value === "bucket";
      const max = Math.max(...rows.map(r => r.request_count));
      $("#us-chart").innerHTML = '<div class="bars">' + rows.map(r => {
        const w = max ? (r.request_count / max * 100) : 0;
        const ew = r.request_count ? (r.error_count / r.request_count * 100) : 0;
        const lbl = isBucket ? r.dimension_value.slice(5, 16).replace("T", " ") : r.dimension_value;
        return '<div class="bar-row"><span class="bar-lbl" title="' + esc(r.dimension_value) + '">'
          + esc(lbl) + '</span><div class="bar-track">'
          + '<div class="bar-fill" style="width:' + w + '%">'
          + '<div class="bar-err" style="width:' + ew + '%"></div></div></div>'
          + '<span class="bar-num">' + esc(r.request_count)
          + (r.error_count ? " (" + esc(r.error_count) + " 错)" : "") + "</span></div>";
      }).join("") + "</div>";
      $("#us-table").innerHTML = objTable(rows,
        ["dimension_value", "request_count", "error_count", "total_units", "avg_latency_ms", "rate_limit_hits"]);
    } catch (e) {
      $("#us-chart").innerHTML = "";
      $("#us-table").innerHTML = '<p class="empty">' + esc(e.message) + "</p>";
    }
  };
  $("#us-go").addEventListener("click", run);
  $("#us-dim").addEventListener("change", run);
  await run();
}
