import { $, api, apiWrite, esc, healthDot, toggleMetaPanel, toggleDebugPanel } from "../core.js";

export async function loadMcpServers(view) {
  view.innerHTML = '<div id="ms-presets"></div>'
    + '<div class="toolbar"><button id="ms-new">+ 添加 MCP Server</button>'
    + '<span class="hint" style="padding:6px;text-align:left">行点击展开详情</span></div>'
    + '<div id="ms-form"></div><div id="ms-list"></div>';
  let servers = [];

  const refresh = async () => {
    await loadPresets();
    try {
      const rows = await api("/admin/mcp-servers");
      servers = Array.isArray(rows) ? rows : [];
      renderList();
    } catch (e) {
      $("#ms-list").innerHTML = '<p class="empty">' + esc(e.message) + '（/admin/mcp-servers 端点未就绪或无权限）</p>';
    }
  };

  // 内置集成：始终可见的 preset 目录。免费的一键「启用」，需 key 的走「配置 key 启用」
  const loadPresets = async () => {
    let presets = [];
    try { const r = await api("/admin/mcp-presets"); presets = Array.isArray(r) ? r : []; }
    catch { $("#ms-presets").innerHTML = ""; return; } // 目录不可用则静默隐藏，手动添加仍可用
    let h = '<div class="card" style="margin:8px 0"><h3 style="margin:0 0 8px">内置集成</h3>'
      + '<div class="tablewrap"><table><thead><tr><th>集成</th><th>描述</th><th>状态</th><th>凭据</th><th></th></tr></thead><tbody>';
    presets.forEach((p, i) => {
      const status = p.enabled
        ? '<span class="badge" style="color:var(--ok);border-color:var(--ok)">已启用</span>'
        : '<span class="hint" style="padding:0">未启用</span>';
      const cred = p.requires_key
        ? '<span class="badge">需 key</span>'
          + (p.apply_url ? ' <a href="' + esc(p.apply_url) + '" target="_blank" rel="noopener">申请 key</a>' : "")
        : '<span class="badge">免费</span>';
      const action = p.enabled
        ? '<button disabled>已启用</button>'
        : (p.requires_key
          ? '<button class="mp-key" data-i="' + i + '">配置 key 启用</button>'
          : '<button class="mp-enable" data-i="' + i + '">启用</button>');
      h += '<tr><td>' + esc(p.id)
        + '<div class="hint" style="padding:2px 0 0;text-align:left">' + esc(p.domain) + ' / ' + esc(p.provider) + '</div></td>'
        + '<td>' + esc(p.description || "") + '</td>'
        + '<td>' + status + '</td>'
        + '<td>' + cred + '</td>'
        + '<td>' + action + '</td></tr>';
    });
    if (!presets.length) h += '<tr><td colspan="5" class="empty">无内置集成</td></tr>';
    h += '</tbody></table></div></div>';
    $("#ms-presets").innerHTML = h;
    $("#ms-presets").querySelectorAll(".mp-enable").forEach(b =>
      b.addEventListener("click", () => enablePreset(presets[+b.dataset.i], b)));
    $("#ms-presets").querySelectorAll(".mp-key").forEach(b =>
      b.addEventListener("click", () => openForm(null, presets[+b.dataset.i])));
  };

  // 免费 preset 一键启用：以 auth:none 创建 mcp server（字段对齐 McpServerInput）
  const enablePreset = async (p, btn) => {
    btn.disabled = true; btn.textContent = "启用中…";
    try {
      await apiWrite("POST", "/admin/mcp-servers", {
        id: p.id, domain: p.domain, provider: p.provider, url: p.url,
        description: p.description || "", auth: { type: "none" },
      });
      await refresh(); // 同步刷新列表与内置集成区（状态翻绿）
    } catch (e) { alert(e.message); btn.disabled = false; btn.textContent = "启用"; }
  };

  const renderList = () => {
    let h = '<div class="tablewrap"><table><thead><tr><th>状态</th><th>id</th><th>domain / provider</th>'
      + '<th>url</th><th>需要 key</th><th>工具数</th><th></th></tr></thead><tbody>';
    servers.forEach((s, i) => {
      h += '<tr class="ms-row" data-i="' + i + '" style="cursor:pointer">'
        + '<td>' + healthDot(s.health, "ms-dot-" + i) + '</td>'
        + '<td>' + esc(s.id) + (s.builtin ? '<span class="badge">builtin</span>' : "") + '</td>'
        + '<td>' + esc(s.domain || "") + " / " + esc(s.provider || "") + '</td>'
        + '<td>' + esc(s.url || "") + '</td>'
        + '<td>' + (s.requires_key ? "是（" + esc(s.auth_type || "?") + "）" : "否") + '</td>'
        + '<td>' + esc(s.tool_count ?? "") + '</td>'
        + '<td><button class="ms-probe" data-i="' + i + '">探测</button> '
        + '<button class="ms-edit" data-i="' + i + '">编辑</button> '
        + '<button class="ms-del" data-i="' + i + '">删除</button></td></tr>'
        + '<tr id="ms-detail-' + i + '" style="display:none"><td colspan="7"></td></tr>';
    });
    if (!servers.length) h += '<tr><td colspan="7" class="empty">无 MCP server</td></tr>';
    h += '</tbody></table></div>';
    $("#ms-list").innerHTML = h;
    $("#ms-list").querySelectorAll(".ms-row").forEach(tr =>
      tr.addEventListener("click", e => {
        if (e.target.closest("button")) return;
        toggleDetail(+tr.dataset.i);
      }));
    $("#ms-list").querySelectorAll(".ms-probe").forEach(b =>
      b.addEventListener("click", () => probe(+b.dataset.i, b)));
    $("#ms-list").querySelectorAll(".ms-edit").forEach(b =>
      b.addEventListener("click", () => openForm(servers[+b.dataset.i])));
    $("#ms-list").querySelectorAll(".ms-del").forEach(b =>
      b.addEventListener("click", async () => {
        const id = servers[+b.dataset.i].id;
        if (!confirm("删除 MCP server " + id + "？其工具将从目录移除")) return;
        try { await apiWrite("DELETE", "/admin/mcp-servers/" + encodeURIComponent(id)); await refresh(); }
        catch (e) { alert(e.message); }
      }));
  };

  const probe = async (i, btn) => {
    btn.disabled = true; btn.textContent = "探测中…";
    try {
      const r = await apiWrite("POST", "/admin/mcp-servers/" + encodeURIComponent(servers[i].id) + "/probe");
      servers[i].health = r.health || r; // 契约返回 health 对象本体，防御兼容包裹形
      const dot = $("#ms-dot-" + i);
      if (dot) dot.outerHTML = healthDot(servers[i].health, "ms-dot-" + i);
      const det = $("#ms-detail-" + i);
      if (det && det.style.display !== "none") await renderDetail(i);
    } catch (e) { alert("探测失败：" + e.message); }
    btn.disabled = false; btn.textContent = "探测";
  };

  const toggleDetail = async i => {
    const rowEl = $("#ms-detail-" + i);
    if (rowEl.style.display !== "none") { rowEl.style.display = "none"; return; }
    rowEl.style.display = "";
    await renderDetail(i);
  };

  const renderDetail = async i => {
    const cell = $("#ms-detail-" + i).querySelector("td");
    cell.innerHTML = '<p class="hint">加载中…</p>';
    let d;
    try { d = await api("/admin/mcp-servers/" + encodeURIComponent(servers[i].id)); }
    catch (e) { cell.innerHTML = '<p class="empty">' + esc(e.message) + '（详情端点未就绪或无权限）</p>'; return; }
    const hh = d.health || {}, lim = d.limits || {}, sec = d.security || {};
    const line = (k, v) => '<div><b>' + esc(k) + '</b>: ' + v + '</div>';
    const dash = v => v === null || v === undefined || v === "" ? "—" : esc(v);
    let h = '<div class="card" style="margin:4px 0;min-width:0">'
      + line("描述", dash(d.description))
      + line("健康", healthDot(hh) + " " + dash(hh.status)
        + " · last_check_at " + dash(hh.last_check_at)
        + " · last_ok_at " + dash(hh.last_ok_at)
        + " · latency_ms " + dash(hh.latency_ms)
        + " · consecutive_failures " + dash(hh.consecutive_failures))
      + (hh.last_error ? line("last_error", '<span style="color:var(--err)">' + esc(hh.last_error) + "</span>") : "")
      + line("测活", d.health_check_enabled === false ? "关闭" : "开启")
      + line("限额", "rps " + dash(lim.rps) + " · rpm " + dash(lim.rpm) + " · max_concurrent " + dash(lim.max_concurrent))
      + line("安全", "integrity_policy " + dash(sec.integrity_policy)
        + " · defense " + (sec.defense_enabled ? "开" : "关")
        + " · result_budget_bytes " + dash(sec.result_budget_bytes));
    const tools = Array.isArray(d.tools) ? d.tools : [];
    h += '<h3 style="margin:12px 0 6px">工具（' + tools.length + '）</h3>';
    if (tools.length) {
      h += '<div class="tablewrap"><table><thead><tr><th>wire_name</th><th>描述（有效）</th><th></th></tr></thead><tbody>';
      tools.forEach((t, j) => {
        h += '<tr><td>' + esc(t.wire_name)
          + (t.description_override ? '<span class="badge" title="原始描述: ' + esc(t.description || "（无）") + '">override</span>' : "")
          + '</td><td>' + esc(t.description_override || t.description || "") + '</td>'
          + '<td><button class="mt-meta" data-j="' + j + '">介绍</button> '
          + '<button class="mt-dbg" data-j="' + j + '">调试</button></td></tr>'
          + '<tr id="mt-meta-' + i + '-' + j + '" style="display:none"><td colspan="3"></td></tr>'
          + '<tr id="mt-dbg-' + i + '-' + j + '" style="display:none"><td colspan="3"></td></tr>';
      });
      h += '</tbody></table></div>';
    } else h += '<p class="empty">无工具</p>';
    h += '</div>';
    cell.innerHTML = h;
    cell.querySelectorAll(".mt-meta").forEach(b =>
      b.addEventListener("click", () =>
        toggleMetaPanel(tools[+b.dataset.j], $("#mt-meta-" + i + "-" + b.dataset.j), () => renderDetail(i))));
    cell.querySelectorAll(".mt-dbg").forEach(b =>
      b.addEventListener("click", () =>
        toggleDebugPanel(tools[+b.dataset.j].wire_name, $("#mt-dbg-" + i + "-" + b.dataset.j))));
  };

  // 添加/编辑表单；凭据一律 secret ref，编辑不回显既有 ref。
  // preset 非空时从内置集成「配置 key 启用」进入：预填字段并把 auth 设为 preset 形态
  const openForm = async (s, preset) => {
    const editing = !!s;
    if (!editing && preset) {
      s = {
        id: preset.id, domain: preset.domain, provider: preset.provider,
        url: preset.url, description: preset.description,
        auth_type: preset.auth?.type || "none",
      };
    }
    s = s || {};
    const lim = s.limits || {};
    const sec = s.security || {};
    const f = $("#ms-form");
    const val = v => esc(v ?? "");
    const ipol = sec.integrity_policy || "warn";
    const applyRow = (!editing && preset && preset.apply_url)
      ? '<div class="hint" style="padding:0 0 8px;text-align:left">配置 ' + esc(preset.id)
        + ' 需要 API key：<a href="' + esc(preset.apply_url) + '" target="_blank" rel="noopener">申请 key</a>'
        + '，拿到后存入你的 secret 后端，再在下方填引用（不收明文）</div>'
      : "";
    f.innerHTML = '<div class="card" style="margin:8px 0">'
      + applyRow
      + '<div style="display:flex;flex-wrap:wrap;gap:6px;align-items:end">'
      + '<label>ID<br><input id="ms-id" size="12" value="' + val(s.id) + '"' + (editing ? " disabled" : "") + '></label>'
      + '<label>Domain<br><input id="ms-domain" size="10" value="' + val(s.domain) + '"></label>'
      + '<label>Provider<br><input id="ms-provider" size="10" value="' + val(s.provider) + '"></label>'
      + '<label>URL<br><input id="ms-url" size="28" value="' + val(s.url) + '"></label>'
      + '<label>描述<br><input id="ms-desc" size="18" value="' + val(s.description) + '"></label>'
      + '<label>Auth<br><select id="ms-auth">'
      + ["none", "bearer", "header"].map(t => '<option' + (s.auth_type === t ? " selected" : "") + '>' + t + '</option>').join("")
      + '</select></label>'
      + '<label id="ms-l-token">token_ref<br><input id="ms-token" size="24" placeholder="secret://env/NAME"></label>'
      + '<label id="ms-l-hname">Header 名<br><input id="ms-hname" size="10" placeholder="x-api-key"></label>'
      + '<label id="ms-l-hval">value_ref<br><input id="ms-hval" size="24" placeholder="secret://env/NAME"></label>'
      + '<label>测活<br><input type="checkbox" id="ms-hc"' + (s.health_check_enabled === false ? "" : " checked") + '></label>'
      + '<label>rps<br><input id="ms-rps" size="4" value="' + val(lim.rps) + '"></label>'
      + '<label>rpm<br><input id="ms-rpm" size="4" value="' + val(lim.rpm) + '"></label>'
      + '<label>max_concurrent<br><input id="ms-conc" size="4" value="' + val(lim.max_concurrent) + '"></label>'
      + '<label>integrity_policy<br><select id="ms-ipol">'
      + ["warn", "quarantine", "block"].map(t => '<option' + (ipol === t ? " selected" : "") + '>' + t + '</option>').join("")
      + '</select></label>'
      + '<label>defense<br><input type="checkbox" id="ms-def"' + (sec.defense_enabled ? " checked" : "") + '></label>'
      + '<label>result_budget_bytes<br><input id="ms-rbb" size="8" value="' + val(sec.result_budget_bytes) + '"></label>'
      + '<button id="ms-save">' + (editing ? "保存" : "创建") + '</button><button id="ms-cancel">取消</button></div>'
      + '<div class="hint" style="padding:6px 0 0;text-align:left">凭据只收 secret ref（如 secret://env/NAME），不收明文密钥'
      + (editing ? '；编辑不回显既有 ref，auth 为 bearer/header 时需重新填写' : "") + '</div></div>';
    const syncAuth = () => {
      const t = $("#ms-auth").value;
      $("#ms-l-token").style.display = t === "bearer" ? "" : "none";
      $("#ms-l-hname").style.display = t === "header" ? "" : "none";
      $("#ms-l-hval").style.display = t === "header" ? "" : "none";
    };
    $("#ms-auth").addEventListener("change", syncAuth);
    syncAuth();
    // 「配置 key 启用」：header preset 预填 header 名，聚焦凭据输入并给 provider 相关 placeholder
    if (!editing && preset && preset.requires_key) {
      const envHint = "secret://env/" + (preset.provider || "KEY").toUpperCase().replace(/[^A-Z0-9]/g, "_") + "_API_KEY";
      if (preset.auth?.type === "header") {
        if (preset.auth.name) $("#ms-hname").value = preset.auth.name;
        const hv = $("#ms-hval"); hv.placeholder = envHint; hv.focus();
      } else {
        const tk = $("#ms-token"); tk.placeholder = envHint; tk.focus();
      }
    }
    $("#ms-cancel").addEventListener("click", () => { f.innerHTML = ""; });
    $("#ms-save").addEventListener("click", async () => {
      const refOk = r => r.startsWith("secret://");
      const body = {
        domain: $("#ms-domain").value.trim(),
        provider: $("#ms-provider").value.trim(),
        url: $("#ms-url").value.trim(),
        health_check: { enabled: $("#ms-hc").checked },
      };
      const id = editing ? s.id : $("#ms-id").value.trim();
      if (!editing) body.id = id;
      const desc = $("#ms-desc").value.trim();
      if (desc) body.description = desc;
      const at = $("#ms-auth").value;
      if (at === "bearer") {
        const ref = $("#ms-token").value.trim();
        if (!refOk(ref)) { alert("token_ref 必须是 secret ref（secret://…），不收明文密钥"); return; }
        body.auth = { type: "bearer", token_ref: ref };
      } else if (at === "header") {
        const name = $("#ms-hname").value.trim(), ref = $("#ms-hval").value.trim();
        if (!name) { alert("Header 名不能为空"); return; }
        if (!refOk(ref)) { alert("value_ref 必须是 secret ref（secret://…），不收明文密钥"); return; }
        body.auth = { type: "header", name, value_ref: ref };
      } else body.auth = { type: "none" };
      const lim2 = {};
      [["rps", "#ms-rps"], ["rpm", "#ms-rpm"], ["max_concurrent", "#ms-conc"]].forEach(([k, sel]) => {
        const v = parseInt($(sel).value, 10);
        if (v > 0) lim2[k] = v;
      });
      if (Object.keys(lim2).length) body.limits = lim2;
      // security 形态同配置 schema：defense 嵌套 enabled（对齐 health_check 写法）
      const sec2 = {
        integrity_policy: $("#ms-ipol").value,
        defense: { enabled: $("#ms-def").checked },
      };
      const rbb = parseInt($("#ms-rbb").value, 10);
      if (rbb > 0) sec2.result_budget_bytes = rbb;
      body.security = sec2;
      try {
        const r = editing
          ? await apiWrite("PUT", "/admin/mcp-servers/" + encodeURIComponent(id), body)
          : await apiWrite("POST", "/admin/mcp-servers", body);
        if (r.health?.status === "unreachable")
          alert("已保存但连接失败" + (r.health.last_error ? "：" + r.health.last_error : "，可稍后「探测」重试"));
        f.innerHTML = "";
        await refresh();
      } catch (e) { alert(e.message); }
    });
  };

  $("#ms-new").addEventListener("click", () => openForm(null));
  await refresh();
}
