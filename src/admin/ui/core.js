// 控制台跨切面 helper：被 ≥2 个 tab 复用的 DOM / 请求 / 渲染工具。
// 免构建 ES module，由各 tab 模块从 "../core.js" import；无第三方依赖。

export const $ = s => document.querySelector(s);
export const TOKEN_KEY = "asterlane-admin-token";

export function esc(v) {
  return String(v).replace(/[&<>"']/g, c => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
  })[c]);
}

export async function api(path) {
  const token = sessionStorage.getItem(TOKEN_KEY) || "";
  const res = await fetch(path, { headers: { Authorization: "Bearer " + token } });
  if (res.status === 401) throw new Error("认证失败：token 无效");
  if (!res.ok) throw new Error("请求失败：HTTP " + res.status);
  return res.json();
}

export async function apiWrite(method, path, body) {
  const token = sessionStorage.getItem(TOKEN_KEY) || "";
  const opts = { method, headers: { Authorization: "Bearer " + token, "Content-Type": "application/json" } };
  if (body) opts.body = JSON.stringify(body);
  const res = await fetch(path, opts);
  if (!res.ok) { const j = await res.json().catch(() => ({})); throw new Error(j.error?.message || "HTTP " + res.status); }
  if (res.status === 204) return {}; // DELETE /admin/mcp-servers/{id} 等无响应体
  return res.json().catch(() => ({}));
}

export function fmtCell(key, v) {
  if (v === null || v === undefined || v === "") return "";
  if (key === "status" && typeof v === "object" && v.kind)
    return v.kind + (v.code != null ? " " + v.code : "");
  if (key === "avg_latency_ms" && typeof v === "number") return v.toFixed(1);
  if (Array.isArray(v)) return v.map(x => typeof x === "object" ? JSON.stringify(x) : String(x)).join("\n");
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

export function cellClass(key, v) {
  if (key !== "status") return "";
  const kind = typeof v === "object" && v ? v.kind : String(v);
  return /success/i.test(kind) ? "ok" : "err";
}

export function objTable(rows, order, labels) {
  if (!rows || !rows.length) return '<p class="empty">无数据</p>';
  const present = new Set();
  rows.forEach(r => Object.keys(r).forEach(k => present.add(k)));
  const cols = (order || []).filter(k => present.has(k))
    .concat([...present].filter(k => !(order || []).includes(k)));
  const head = cols.map(c => "<th>" + esc(labels?.[c] ?? c) + "</th>").join("");
  const body = rows.map(r =>
    "<tr>" + cols.map(c =>
      '<td class="' + cellClass(c, r[c]) + '">' + esc(fmtCell(c, r[c])) + "</td>"
    ).join("") + "</tr>"
  ).join("");
  return '<div class="tablewrap"><table><thead><tr>' + head + "</tr></thead><tbody>" + body + "</tbody></table></div>";
}

// 健康状态灯：ok 绿 / unreachable 红 / unknown 灰 / disabled 暗
export function healthDot(h, id) {
  const s = ["ok", "unreachable", "unknown", "disabled"].includes(h?.status) ? h.status : "unknown";
  return '<span class="dot ' + s + '"' + (id ? ' id="' + id + '"' : "") + ' title="' + s + '"></span>';
}

// auth_mode 徽标：token 绿 / legacy 灰；字段缺失（后端未就绪）不渲染
export function authBadge(mode) {
  if (mode === "token") return '<span class="badge" style="color:var(--ok);border-color:var(--ok);margin-left:0">token</span>';
  if (mode === "legacy") return '<span class="badge" style="margin-left:0">legacy</span>';
  return "";
}

// per-key 配额进度条：总量 + 当日两条；≥80% 橙、100% 红；无上限只显示计数；usage 缺失整体不渲染
export function usageBars(u) {
  if (!u || typeof u !== "object") return "";
  const row = (lbl, count, max) => {
    const c = Number(count) || 0;
    if (max == null) return '<div class="q-row"><span>' + lbl + '</span><span></span><span class="q-num">' + c + '</span></div>';
    const pct = max > 0 ? Math.min(c / max * 100, 100) : 100;
    const color = pct >= 100 ? "var(--err)" : pct >= 80 ? "var(--warn)" : "var(--accent)";
    return '<div class="q-row"><span>' + lbl + '</span>'
      + '<span class="q-track" title="' + pct.toFixed(0) + '%"><span class="q-fill" style="width:' + pct.toFixed(0) + '%;background:' + color + '"></span></span>'
      + '<span class="q-num">' + c + "/" + max + '</span></div>';
  };
  return '<div class="q-wrap">' + row("总", u.calls_total, u.max_calls) + row("日", u.calls_today, u.max_calls_per_day) + '</div>';
}

// 工具调试面板：预填已存默认（GET defaults），调用（POST invoke）、存为默认（PUT defaults）
export async function toggleDebugPanel(name, rowEl) {
  if (rowEl.style.display !== "none") { rowEl.style.display = "none"; return; }
  rowEl.style.display = "";
  const cell = rowEl.querySelector("td");
  cell.innerHTML = '<div class="card" style="margin:4px 0;min-width:0">'
    + '<div class="hint" style="padding:0 0 6px;text-align:left">调试调用 ' + esc(name) + '（参数为 JSON object）</div>'
    + '<textarea class="dbg-args" rows="5" spellcheck="false" style="width:100%;font-family:ui-monospace,Menlo,Consolas,monospace;font-size:12.5px"></textarea>'
    + '<div class="toolbar" style="margin:8px 0 0"><button class="dbg-run">调用</button>'
    + '<button class="dbg-save">存为默认</button>'
    + '<span class="dbg-status hint" style="padding:6px;text-align:left"></span></div>'
    + '<pre class="dbg-result" style="white-space:pre-wrap;word-break:break-word;max-height:320px;overflow:auto;margin:8px 0 0"></pre></div>';
  const ta = cell.querySelector(".dbg-args"), st = cell.querySelector(".dbg-status"), out = cell.querySelector(".dbg-result");
  try {
    const d = await api("/admin/tools/" + encodeURIComponent(name) + "/defaults");
    ta.value = JSON.stringify(d.args, null, 2);
    st.textContent = "已加载存储默认（" + d.source + "）";
  } catch { ta.value = "{}"; }
  const parseArgs = () => {
    try {
      const v = JSON.parse(ta.value.trim() || "{}");
      if (typeof v !== "object" || v === null || Array.isArray(v)) { alert("参数必须是 JSON object"); return null; }
      return v;
    } catch { alert("参数不是合法 JSON"); return null; }
  };
  cell.querySelector(".dbg-run").addEventListener("click", async () => {
    const args = parseArgs(); if (args === null) return;
    st.textContent = "调用中…"; out.textContent = "";
    try {
      const r = await apiWrite("POST", "/admin/tools/" + encodeURIComponent(name) + "/invoke", args);
      st.textContent = "status " + r.status + " · " + r.latency_ms + " ms" + (r.request_id ? " · " + r.request_id : "");
      out.textContent = typeof r.result === "string" ? r.result : JSON.stringify(r.result, null, 2);
    } catch (e) { st.textContent = "调用失败"; out.textContent = e.message; }
  });
  cell.querySelector(".dbg-save").addEventListener("click", async () => {
    const args = parseArgs(); if (args === null) return;
    try {
      await apiWrite("PUT", "/admin/tools/" + encodeURIComponent(name) + "/defaults", args);
      st.textContent = "已存为默认参数";
    } catch (e) { alert(e.message); }
  });
}

// 工具介绍 override 面板：保存 = PUT metadata，清除 = DELETE（恢复上游原始描述）
export function toggleMetaPanel(tool, rowEl, onSaved) {
  if (rowEl.style.display !== "none") { rowEl.style.display = "none"; return; }
  rowEl.style.display = "";
  const cell = rowEl.querySelector("td");
  cell.innerHTML = '<div class="card" style="margin:4px 0;min-width:0">'
    + '<div class="hint" style="padding:0 0 6px;text-align:left">介绍覆盖（覆盖上游描述）· 原始: '
    + esc(tool.description || "（无）") + '</div>'
    + '<textarea class="mt-text" rows="3" spellcheck="false" style="width:100%;font-family:ui-monospace,Menlo,Consolas,monospace;font-size:12.5px">'
    + esc(tool.description_override || "") + '</textarea>'
    + '<div class="toolbar" style="margin:8px 0 0"><button class="mt-save">保存</button>'
    + '<button class="mt-clear">清除覆盖</button>'
    + '<span class="mt-status hint" style="padding:6px;text-align:left"></span></div></div>';
  const st = cell.querySelector(".mt-status");
  cell.querySelector(".mt-save").addEventListener("click", async () => {
    const text = cell.querySelector(".mt-text").value.trim();
    if (!text) { alert("介绍不能为空；恢复原始描述请用「清除 override」"); return; }
    try {
      await apiWrite("PUT", "/admin/tools/" + encodeURIComponent(tool.wire_name) + "/metadata", { description: text });
      st.textContent = "已保存";
      if (onSaved) await onSaved();
    } catch (e) { st.textContent = "保存失败：" + e.message; }
  });
  cell.querySelector(".mt-clear").addEventListener("click", async () => {
    try {
      await apiWrite("DELETE", "/admin/tools/" + encodeURIComponent(tool.wire_name) + "/metadata");
      st.textContent = "已清除";
      if (onSaved) await onSaved();
    } catch (e) { st.textContent = "清除失败：" + e.message; }
  });
}

// 事件行详情：负载捕获字段（request_args / response_preview / upstream_latency_ms）
// 与「存为默认参数」（前端 PUT 到该 tool 的 defaults）
export function toggleEventDetail(ev, rowEl) {
  if (rowEl.style.display !== "none") { rowEl.style.display = "none"; return; }
  rowEl.style.display = "";
  const cell = rowEl.querySelector("td");
  const pre = v => '<pre style="white-space:pre-wrap;word-break:break-word;max-height:240px;overflow:auto;margin:4px 0 8px">'
    + esc(v || "（未捕获）") + '</pre>';
  let h = '<div class="card" style="margin:4px 0;min-width:0">'
    + '<div><b>上游延迟(ms)</b>: ' + esc(ev.upstream_latency_ms ?? "—") + '</div>'
    + '<div style="margin-top:6px"><b>请求参数</b>:</div>' + pre(ev.request_args)
    + '<div><b>响应预览</b>:</div>' + pre(ev.response_preview);
  if (ev.request_args) h += '<button class="ev-save-default">存为默认参数</button>';
  h += '</div>';
  cell.innerHTML = h;
  const btn = cell.querySelector(".ev-save-default");
  if (btn) btn.addEventListener("click", async () => {
    let args;
    try { args = JSON.parse(ev.request_args); } catch { alert("request_args 不是合法 JSON（可能已被截断）"); return; }
    if (typeof args !== "object" || args === null || Array.isArray(args)) { alert("request_args 不是 JSON object"); return; }
    try {
      await apiWrite("PUT", "/admin/tools/" + encodeURIComponent(ev.tool_name) + "/defaults", args);
      alert("已存为 " + ev.tool_name + " 的默认参数");
    } catch (e) { alert(e.message); }
  });
}

// ── Proxy key token 签发/轮换弹窗 ──
// 安全红线：token 明文只存在于弹窗 DOM，不写 sessionStorage/localStorage/console；关闭即清空
export function openTokenDialog(key, onDone) {
  const dlg = $("#tok-dialog");
  const mode = key.auth_mode === "token" ? "轮换" : "签发";
  let issued = false;
  dlg.innerHTML = '<h3 style="margin:0 0 8px">' + mode + ' token · ' + esc(key.id) + '</h3>'
    + (key.auth_mode === "token"
      ? '<div class="hint" style="padding:0 0 8px;text-align:left">该 key 已有 token，' + mode + '后旧 token 立即失效</div>' : "")
    + '<label>过期时间（可选，留空永不过期）<br><input type="datetime-local" id="tok-exp"></label>'
    + '<div class="toolbar" style="margin:10px 0 0"><button id="tok-go">' + mode + '</button>'
    + '<button id="tok-cancel">取消</button>'
    + '<span id="tok-status" class="hint" style="padding:6px;text-align:left"></span></div>';
  dlg.addEventListener("close", () => { dlg.innerHTML = ""; if (issued && onDone) onDone(); }, { once: true });
  dlg.showModal();
  $("#tok-cancel").addEventListener("click", () => dlg.close());
  $("#tok-go").addEventListener("click", async () => {
    const exp = $("#tok-exp").value;
    $("#tok-status").textContent = mode + "中…";
    let r;
    try {
      r = await apiWrite("POST", "/admin/proxy-keys/" + encodeURIComponent(key.id) + "/token",
        exp ? { expires_at: new Date(exp).toISOString() } : undefined);
    } catch (e) { $("#tok-status").textContent = e.message + "（端点未就绪或无权限）"; return; }
    if (!r || !r.token) { $("#tok-status").textContent = "响应缺少 token 字段"; return; }
    issued = true;
    dlg.innerHTML = '<h3 style="margin:0 0 8px">token 已' + mode + ' · ' + esc(key.id) + '</h3>'
      + '<div style="color:var(--err);font-weight:600;margin:0 0 10px">仅此一次展示，关闭后无法再查看，请立即保存</div>'
      + '<div style="display:flex;gap:8px;align-items:center">'
      + '<input id="tok-value" readonly value="' + esc(r.token) + '" size="48" style="flex:1;min-width:0;font-family:ui-monospace,Menlo,Consolas,monospace;font-size:12.5px">'
      + '<button id="tok-copy">复制</button></div>'
      + '<div style="color:var(--muted);font-size:12px;margin-top:8px">过期时间：' + esc(r.expires_at || "永不过期") + '</div>'
      + '<div class="toolbar" style="margin:12px 0 0"><button id="tok-close">关闭</button></div>';
    $("#tok-copy").addEventListener("click", async () => {
      const inp = $("#tok-value");
      inp.select();
      try { await navigator.clipboard.writeText(inp.value); }
      catch { document.execCommand("copy"); } // 非安全上下文（http）时的兜底
      $("#tok-copy").textContent = "已复制";
    });
    $("#tok-close").addEventListener("click", () => dlg.close());
  });
}

export function enableColResize(table) {
  table.style.tableLayout = "fixed";
  const ths = table.querySelectorAll("th");
  ths.forEach(th => { th.style.width = th.offsetWidth + "px"; });
  ths.forEach(th => {
    th.style.position = "relative";
    const h = document.createElement("div");
    h.className = "col-handle";
    th.appendChild(h);
    h.addEventListener("mousedown", e => {
      e.preventDefault();
      const x0 = e.clientX, w0 = th.offsetWidth;
      h.classList.add("active");
      const move = ev => { th.style.width = Math.max(40, w0 + ev.clientX - x0) + "px"; };
      const up = () => { h.classList.remove("active"); document.removeEventListener("mousemove", move); document.removeEventListener("mouseup", up); };
      document.addEventListener("mousemove", move);
      document.addEventListener("mouseup", up);
    });
  });
}
