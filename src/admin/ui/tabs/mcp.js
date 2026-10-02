import { $, api, apiWrite, esc, healthDot, toggleMetaPanel, toggleDebugPanel } from "../core.js";

export async function loadMcpServers(view) {
  view.innerHTML = '<div id="ms-presets"></div>'
    + '<div class="toolbar"><button id="ms-new">+ 添加 MCP 服务</button>'
    + '<span class="hint" style="padding:6px;text-align:left">行点击展开详情</span></div>'
    + '<div id="ms-form"></div><div id="ms-notice"></div><div id="ms-list"></div>';
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
      + '<div id="ms-preset-status" class="hint" style="display:none;margin:0 0 6px;text-align:left;color:var(--err)"></div>'
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
    } catch (e) {
      // alert 会随点击消失；行内常驻提示让失败原因（如 id 与现有资源冲突）可停留查阅
      btn.disabled = false; btn.textContent = "启用";
      const st = $("#ms-preset-status");
      if (st) { st.style.display = "block"; st.textContent = "启用 " + p.id + " 失败：" + e.message; }
    }
  };

  const renderList = () => {
    let h = '<div class="tablewrap"><table><thead><tr><th>状态</th><th>ID</th><th>领域 / 提供商</th>'
      + '<th>URL</th><th>需要 key</th><th>工具数</th><th></th></tr></thead><tbody>';
    servers.forEach((s, i) => {
      h += '<tr class="ms-row" data-i="' + i + '" style="cursor:pointer">'
        + '<td>' + healthDot(s.health, "ms-dot-" + i) + '</td>'
        + '<td>' + esc(s.id) + (s.builtin ? '<span class="badge">内置</span>' : "") + '</td>'
        + '<td>' + esc(s.domain || "") + " / " + esc(s.provider || "") + '</td>'
        + '<td>' + esc(s.url || "") + '</td>'
        + '<td>' + (s.requires_key ? "是（" + esc(s.auth_type || "?") + "）" : "否") + '</td>'
        + '<td>' + esc(s.tool_count ?? "") + '</td>'
        + '<td><button class="ms-probe" data-i="' + i + '">探测</button> '
        + authButton(s, i)
        + '<button class="ms-edit" data-i="' + i + '">编辑</button> '
        + '<button class="ms-del" data-i="' + i + '">删除</button></td></tr>'
        + '<tr id="ms-detail-' + i + '" style="display:none"><td colspan="7"></td></tr>';
    });
    if (!servers.length) h += '<tr><td colspan="7" class="empty">无 MCP 服务</td></tr>';
    h += '</tbody></table></div>';
    $("#ms-list").innerHTML = h;
    $("#ms-list").querySelectorAll(".ms-row").forEach(tr =>
      tr.addEventListener("click", e => {
        if (e.target.closest("button")) return;
        toggleDetail(+tr.dataset.i);
      }));
    $("#ms-list").querySelectorAll(".ms-probe").forEach(b =>
      b.addEventListener("click", () => probe(+b.dataset.i, b)));
    $("#ms-list").querySelectorAll(".ms-auth").forEach(b =>
      b.addEventListener("click", () => authorize(servers[+b.dataset.i], b)));
    $("#ms-list").querySelectorAll(".ms-deauth").forEach(b =>
      b.addEventListener("click", () => deauthorize(servers[+b.dataset.i], b)));
    $("#ms-list").querySelectorAll(".ms-edit").forEach(b =>
      b.addEventListener("click", () => openForm(servers[+b.dataset.i])));
    $("#ms-list").querySelectorAll(".ms-del").forEach(b =>
      b.addEventListener("click", async () => {
        const id = servers[+b.dataset.i].id;
        if (!confirm("删除 MCP 服务 " + id + "？其工具将从目录移除")) return;
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

  // 授权码类 OAuth 上游的授权按钮：需要授权时「授权」，已授权时「撤销授权」；其他 server 不显示
  const authButton = (s, i) => {
    const oa = s.oauth;
    if (!oa || oa.grant !== "authorization_code") return "";
    return oa.status === "authorized"
      ? '<button class="ms-deauth" data-i="' + i + '" title="清除网关保存的凭据；不会通知授权服务器吊销 token">撤销授权</button> '
      : '<button class="ms-auth" data-i="' + i + '" title="在新标签页打开授权服务器的授权页面">授权</button> ';
  };

  // 详情里的 OAuth 一行：授权方式、授权状态与 access token 到期时间（到期前网关自动刷新）
  const oauthSummary = oa => {
    const label = { authorized: "已授权", authorization_required: "需要授权", automatic: "自动换取，无需授权" };
    return esc(oa.grant) + " · " + esc(label[oa.status] || oa.status)
      + (oa.expires_at ? " · access token 到期 " + esc(oa.expires_at) + "（到期前网关自动刷新）" : "");
  };

  const notice = html => { $("#ms-notice").innerHTML = html ? '<div class="hint" style="padding:6px 0;text-align:left">' + html + '</div>' : ""; };

  // 发起授权并在新标签页打开授权 URL。授权在授权服务器的页面完成，完成后浏览器被重定向回网关
  // （/oauth/callback），网关保存凭据并重连；回到本页后列表自动刷新。
  const authorize = async (s, btn) => {
    btn.disabled = true;
    // 在点击事件里同步打开空白标签页：await 之后再 window.open 会被浏览器当作弹窗拦截。
    // 断开 opener，授权服务器的页面不能反向操作本控制台标签页。
    const tab = window.open("about:blank", "_blank");
    if (tab) tab.opener = null;
    try {
      const r = await apiWrite("POST", "/admin/mcp-servers/" + encodeURIComponent(s.id) + "/oauth/authorize");
      // URL 来自授权服务器的元数据；网关只放行 https（本机联调为 http），这里再确认一次协议
      if (!/^https?:\/\//.test(r.authorization_url || "")) throw new Error("响应中的授权地址无效");
      const minutes = Math.max(1, Math.round((r.expires_in || 0) / 60));
      if (tab) {
        tab.location.href = r.authorization_url;
        notice("已在新标签页打开 " + esc(s.id) + " 的授权页面，请在 " + minutes + " 分钟内完成授权；完成后回到本页。");
      } else {
        notice('浏览器拦截了新标签页：<a href="' + esc(r.authorization_url) + '" target="_blank" rel="noopener">点此打开 '
          + esc(s.id) + ' 的授权页面</a>（' + minutes + ' 分钟内有效，只能使用一次）。');
      }
      window.addEventListener("focus", () => refresh(), { once: true });
    } catch (e) {
      if (tab) tab.close();
      alert("发起授权失败：" + e.message);
    }
    btn.disabled = false;
  };

  const deauthorize = async (s, btn) => {
    if (!confirm("撤销 " + s.id + " 的授权？网关将清除已保存的凭据，该服务的工具会从目录移除，重新授权后恢复")) return;
    btn.disabled = true;
    try {
      await apiWrite("DELETE", "/admin/mcp-servers/" + encodeURIComponent(s.id) + "/oauth");
      notice("");
      await refresh();
    } catch (e) { alert("撤销授权失败：" + e.message); btn.disabled = false; }
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
        + " · 末次检查 " + dash(hh.last_check_at)
        + " · 末次正常 " + dash(hh.last_ok_at)
        + " · 延迟(ms) " + dash(hh.latency_ms)
        + " · 连续失败 " + dash(hh.consecutive_failures))
      + (hh.last_error ? line("最近错误", '<span style="color:var(--err)">' + esc(hh.last_error) + "</span>") : "")
      + (d.oauth ? line("OAuth", oauthSummary(d.oauth)) : "")
      + line("测活", d.health_check_enabled === false ? "关闭" : "开启")
      + line("限额", "rps " + dash(lim.rps) + " · rpm " + dash(lim.rpm) + " · 最大并发 " + dash(lim.max_concurrent))
      + line("安全", '<span title="工具 schema 变化时：warn 仅记录 / quarantine 暂停 / block 拒绝">完整性策略</span> ' + dash(sec.integrity_policy)
        + ' · <span title="扫描返回内容，检测 prompt 注入等恶意负载">防御</span> ' + (sec.defense_enabled ? "开" : "关")
        + ' · <span title="单次调用返回结果字节上限，超出截断分页">结果预算</span> ' + dash(sec.result_budget_bytes));
    const tools = Array.isArray(d.tools) ? d.tools : [];
    h += '<h3 style="margin:12px 0 6px">工具（' + tools.length + '）</h3>';
    if (tools.length) {
      h += '<div class="tablewrap"><table><thead><tr><th>工具名</th><th>描述（有效）</th><th></th></tr></thead><tbody>';
      tools.forEach((t, j) => {
        h += '<tr><td>' + esc(t.wire_name)
          + (t.description_override ? '<span class="badge" title="原始描述: ' + esc(t.description || "（无）") + '">已覆盖</span>' : "")
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

  // 添加/编辑表单。bearer / header 的凭据编辑时不回显，需重新填写；OAuth 回显 grant、client id、
  // client secret 的引用（只是 secret:// 引用，不是 secret）与 scopes，保存时原样提交，认证配置保持不变。
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
    const oa = s.oauth || {};
    const f = $("#ms-form");
    const val = v => esc(v ?? "");
    const ipol = sec.integrity_policy || "warn";
    const applyRow = (!editing && preset && preset.apply_url)
      ? '<div class="hint" style="padding:0 0 8px;text-align:left">配置 ' + esc(preset.id)
        + ' 需要 API key：<a href="' + esc(preset.apply_url) + '" target="_blank" rel="noopener">申请 key</a>'
        + '，拿到后在下方直接填入即可</div>'
      : "";
    f.innerHTML = '<div class="card" style="margin:8px 0">'
      + applyRow
      + '<div class="form-row">'
      + '<label>ID<br><input id="ms-id" size="12" value="' + val(s.id) + '"' + (editing ? " disabled" : "") + '></label>'
      + '<label>领域<br><input id="ms-domain" size="10" value="' + val(s.domain) + '"></label>'
      + '<label>提供商<br><input id="ms-provider" size="10" value="' + val(s.provider) + '"></label>'
      + '<label>URL<br><input id="ms-url" size="28" value="' + val(s.url) + '"></label>'
      + '<label>描述<br><input id="ms-desc" size="18" value="' + val(s.description) + '"></label>'
      + '<label>认证<br><select id="ms-auth">'
      + ["none", "bearer", "header", "oauth"].map(t => '<option' + (s.auth_type === t ? " selected" : "") + '>' + t + '</option>').join("")
      + '</select></label>'
      + '<label id="ms-l-token">Token<br><input id="ms-token" size="24" placeholder="sk-…"></label>'
      + '<label id="ms-l-hname">Header 名<br><input id="ms-hname" size="10" placeholder="x-api-key"></label>'
      + '<label id="ms-l-hval">Header 值<br><input id="ms-hval" size="24" placeholder="sk-…"></label>'
      + '<label id="ms-l-ogrant" title="authorization_code：管理员在浏览器里授权一次，网关保存并刷新 token；client_credentials：网关自动换取">授权方式<br><select id="ms-ogrant">'
      + ["authorization_code", "client_credentials"].map(t => '<option' + (oa.grant === t ? " selected" : "") + '>' + t + '</option>').join("")
      + '</select></label>'
      + '<label id="ms-l-ocid" title="client_credentials 必填；authorization_code 留空则向授权服务器动态注册">Client ID<br><input id="ms-ocid" size="18" value="' + val(oa.client_id) + '"></label>'
      + '<label id="ms-l-osec" title="只接受 secret:// 引用，不要填明文">Client secret 引用<br><input id="ms-osec" size="26" placeholder="secret://env/NAME" value="' + val(oa.client_secret_ref) + '"></label>'
      + '<label id="ms-l-oscopes" title="空格或逗号分隔；留空由授权服务器元数据决定">Scopes<br><input id="ms-oscopes" size="16" value="' + val((oa.scopes || []).join(" ")) + '"></label>'
      + '<label>测活<br><input type="checkbox" id="ms-hc"' + (s.health_check_enabled === false ? "" : " checked") + '></label>'
      + '<label>rps<br><input id="ms-rps" size="4" value="' + val(lim.rps) + '"></label>'
      + '<label>rpm<br><input id="ms-rpm" size="4" value="' + val(lim.rpm) + '"></label>'
      + '<label>最大并发<br><input id="ms-conc" size="4" value="' + val(lim.max_concurrent) + '"></label>'
      + '<label title="工具 schema 发生变化时的处理策略：warn 仅记录，quarantine 暂停该工具，block 拒绝所有调用">完整性策略<br><select id="ms-ipol">'
      + ["warn", "quarantine", "block"].map(t => '<option' + (ipol === t ? " selected" : "") + '>' + t + '</option>').join("")
      + '</select></label>'
      + '<label title="启用后扫描工具返回内容，检测 prompt 注入等恶意负载">防御<br><input type="checkbox" id="ms-def"' + (sec.defense_enabled ? " checked" : "") + '></label>'
      + '<label title="单次工具调用返回结果的字节上限，超出则截断并分页；留空不限制">结果预算<br><input id="ms-rbb" size="8" value="' + val(sec.result_budget_bytes) + '"></label>'
      + '<button id="ms-save">' + (editing ? "保存" : "创建") + '</button><button id="ms-cancel">取消</button></div>'
      + '<div class="hint" style="padding:6px 0 0;text-align:left">直接填写 API key 即可'
      + (editing ? '；编辑不回显既有凭据，认证为 bearer/header 时需重新填写' : "")
      + '。OAuth：client secret 只接受 secret:// 引用；authorization_code 需在网关配置里设置 oauth.redirect_base_url 与 oauth.token_encryption_key_ref，'
      + '并在授权服务器登记回调地址 {redirect_base_url}/oauth/callback，保存后点列表里的「授权」</div></div>';
    const syncAuth = () => {
      const t = $("#ms-auth").value;
      $("#ms-l-token").style.display = t === "bearer" ? "" : "none";
      $("#ms-l-hname").style.display = t === "header" ? "" : "none";
      $("#ms-l-hval").style.display = t === "header" ? "" : "none";
      ["ogrant", "ocid", "osec", "oscopes"].forEach(k => {
        $("#ms-l-" + k).style.display = t === "oauth" ? "" : "none";
      });
    };
    $("#ms-auth").addEventListener("change", syncAuth);
    syncAuth();
    // 「配置 key 启用」：header preset 预填 header 名，聚焦凭据输入
    if (!editing && preset && preset.requires_key) {
      if (preset.auth?.type === "header") {
        if (preset.auth.name) $("#ms-hname").value = preset.auth.name;
        $("#ms-hval").focus();
      } else {
        $("#ms-token").focus();
      }
    }
    $("#ms-cancel").addEventListener("click", () => { f.innerHTML = ""; });
    $("#ms-save").addEventListener("click", async () => {
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
        const v = $("#ms-token").value.trim();
        if (!v) { alert("Token 不能为空"); return; }
        body.auth = { type: "bearer", token_ref: v };
      } else if (at === "header") {
        const name = $("#ms-hname").value.trim(), v = $("#ms-hval").value.trim();
        if (!name) { alert("Header 名不能为空"); return; }
        if (!v) { alert("Header 值不能为空"); return; }
        body.auth = { type: "header", name, value_ref: v };
      } else if (at === "oauth") {
        // 字段与回显一致，原样提交；client id / secret 引用留空即不提交（授权码类留空 = 动态注册）。
        // 校验（必填项、secret:// 引用、https、顶层 oauth 配置）由服务端完成，错误以 alert 展示
        const auth = { type: "oauth", grant: $("#ms-ogrant").value };
        const cid = $("#ms-ocid").value.trim(), sec = $("#ms-osec").value.trim();
        if (cid) auth.client_id = cid;
        if (sec) auth.client_secret_ref = sec;
        auth.scopes = $("#ms-oscopes").value.split(/[\s,]+/).filter(Boolean);
        body.auth = auth;
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
        else if (r.health?.status === "auth_required")
          alert("已保存，但该上游需要管理员授权后才能使用");
        f.innerHTML = "";
        await refresh();
      } catch (e) { alert(e.message); }
    });
  };

  $("#ms-new").addEventListener("click", () => openForm(null));
  await refresh();
}
