import { $, api, esc, toggleDebugPanel } from "../core.js";

export async function loadTools(view) {
  const data = await api("/admin/tools");
  const render = filter => {
    const f = filter.trim().toLowerCase();
    const rows = data.tools.filter(t => !f
      || [t.name, t.resource_id, t.description, t.description_override]
        .some(v => (v || "").toLowerCase().includes(f)));
    let h = '<div class="tablewrap"><table><thead><tr><th>name</th><th>resource_id</th><th>description</th><th></th></tr></thead><tbody>';
    rows.forEach((t, i) => {
      // 有效描述 = override ?? 上游原始；override 徽标悬浮可见原始描述
      const badge = t.description_override
        ? '<span class="badge" title="原始描述: ' + esc(t.description || "（无）") + '">override</span>' : "";
      h += '<tr><td>' + esc(t.name) + '</td><td>' + esc(t.resource_id || "") + '</td>'
        + '<td>' + esc(t.description_override || t.description || "") + badge + '</td>'
        + '<td><button class="tool-debug" data-name="' + esc(t.name) + '" data-i="' + i + '">调试</button></td></tr>'
        + '<tr id="dbg-' + i + '" style="display:none"><td colspan="4"></td></tr>';
    });
    if (!rows.length) h += '<tr><td colspan="4" class="empty">无数据</td></tr>';
    h += '</tbody></table></div>';
    $("#tools-table").innerHTML = h;
    $("#tools-count").textContent = rows.length + " / " + data.total_count;
    $("#tools-table").querySelectorAll(".tool-debug").forEach(b =>
      b.addEventListener("click", () => toggleDebugPanel(b.dataset.name, $("#dbg-" + b.dataset.i))));
  };
  view.innerHTML = '<div class="toolbar"><input id="tools-filter" placeholder="按名称/资源/描述过滤" size="32">'
    + '<span id="tools-count" class="hint" style="padding:6px"></span></div><div id="tools-table"></div>';
  $("#tools-filter").addEventListener("input", e => render(e.target.value));
  render("");
}
