import { $, api, esc, cellClass, fmtCell, toggleEventDetail } from "../core.js";

export async function loadEvents(view) {
  view.innerHTML = '<div class="toolbar">'
    + '<input id="ev-key" placeholder="proxy_key_id" size="14">'
    + '<input id="ev-res" placeholder="resource_id" size="14">'
    + '<input id="ev-tool" placeholder="tool_name" size="18">'
    + '<input type="datetime-local" id="ev-from" title="起始时间">'
    + '<input type="datetime-local" id="ev-to" title="结束时间">'
    + '<input id="ev-limit" value="50" size="5">'
    + '<button id="ev-go">查询</button>'
    + '<button id="ev-more" disabled>加载更多</button></div><div id="ev-table"></div>';
  const COLS = ["timestamp", "proxy_key_id", "resource_id", "tool_name", "status",
    "latency_ms", "retry_count", "rate_limited", "queued_ms", "upstream_key_ref", "request_id"];
  let acc = [];      // 累积行（加载更多在此追加）
  let cursor = null; // 时间游标 = 上一页末行 timestamp（events 按时间降序）
  const renderRows = () => {
    let h = '<div class="tablewrap"><table><thead><tr>'
      + COLS.map(c => '<th>' + esc(c) + '</th>').join("") + '<th></th></tr></thead><tbody>';
    acc.forEach((r, i) => {
      h += '<tr>' + COLS.map(c =>
        '<td class="' + cellClass(c, r[c]) + '">' + esc(fmtCell(c, r[c])) + '</td>').join("")
        + '<td><button class="ev-detail" data-i="' + i + '">详情</button></td></tr>'
        + '<tr id="evd-' + i + '" style="display:none"><td colspan="' + (COLS.length + 1) + '"></td></tr>';
    });
    if (!acc.length) h += '<tr><td colspan="' + (COLS.length + 1) + '" class="empty">无数据</td></tr>';
    h += '</tbody></table></div>';
    $("#ev-table").innerHTML = h;
    $("#ev-table").querySelectorAll(".ev-detail").forEach(b =>
      b.addEventListener("click", () => toggleEventDetail(acc[+b.dataset.i], $("#evd-" + b.dataset.i))));
  };
  const run = async append => {
    const limit = parseInt($("#ev-limit").value, 10) || 50;
    const p = new URLSearchParams({ limit });
    if ($("#ev-key").value) p.set("proxy_key_id", $("#ev-key").value);
    if ($("#ev-res").value) p.set("resource_id", $("#ev-res").value);
    if ($("#ev-tool").value) p.set("tool_name", $("#ev-tool").value);
    if ($("#ev-from").value) p.set("from", new Date($("#ev-from").value).toISOString());
    if (append && cursor) p.set("to", cursor);
    else if ($("#ev-to").value) p.set("to", new Date($("#ev-to").value).toISOString());
    try {
      const rows = await api("/admin/events?" + p);
      acc = append ? acc.concat(rows) : rows;
      cursor = rows.length ? rows[rows.length - 1].timestamp : cursor;
      $("#ev-more").disabled = rows.length < limit;
      renderRows();
    } catch (e) {
      $("#ev-table").innerHTML = '<p class="empty">' + esc(e.message) + "</p>";
    }
  };
  $("#ev-go").addEventListener("click", () => run(false));
  $("#ev-more").addEventListener("click", () => run(true));
  await run(false);
}
