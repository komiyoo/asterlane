import { $, api, esc, fmtCell } from "../core.js";

// 审计 tab：security-events 按 kind 过滤；admin_audit 字段可能平铺在行上或嵌在 details 内，防御式取
export async function loadAudit(view) {
  view.innerHTML = '<div class="toolbar">'
    + '<select id="au-kind"><option value="admin_audit">admin_audit</option><option value="">全部 kind</option></select>'
    + '<input id="au-limit" value="100" size="5" title="limit">'
    + '<button id="au-go">查询</button></div><div id="au-table"></div>';
  const run = async () => {
    const p = new URLSearchParams({ limit: parseInt($("#au-limit").value, 10) || 100 });
    if ($("#au-kind").value) p.set("kind", $("#au-kind").value);
    try {
      const rows = await api("/admin/security-events?" + p);
      if (!rows.length) { $("#au-table").innerHTML = '<p class="empty">无数据</p>'; return; }
      const field = (r, det, names) => {
        for (const n of names) {
          if (r[n] != null && r[n] !== "") return r[n];
          if (det[n] != null && det[n] !== "") return det[n];
        }
        return "";
      };
      let h = '<div class="tablewrap"><table><thead><tr><th>时间</th><th>kind</th><th>admin</th><th>action</th><th>target</th><th>details</th></tr></thead><tbody>';
      rows.forEach(r => {
        let det = r.details;
        if (typeof det === "string") { try { det = JSON.parse(det); } catch { det = {}; } }
        if (!det || typeof det !== "object") det = {};
        h += '<tr><td>' + esc(r.timestamp || "") + '</td><td>' + esc(r.kind || "") + '</td>'
          + '<td>' + esc(field(r, det, ["admin_key_id", "admin_key", "admin_id", "admin"])) + '</td>'
          + '<td>' + esc(field(r, det, ["action"])) + '</td>'
          + '<td>' + esc(field(r, det, ["target", "target_id"])) + '</td>'
          + '<td>' + esc(fmtCell("details", r.details)) + '</td></tr>';
      });
      h += '</tbody></table></div>';
      $("#au-table").innerHTML = h;
    } catch (e) {
      $("#au-table").innerHTML = '<p class="empty">' + esc(e.message) + '（/admin/security-events 端点未就绪或无权限）</p>';
    }
  };
  $("#au-go").addEventListener("click", run);
  $("#au-kind").addEventListener("change", run);
  await run();
}
