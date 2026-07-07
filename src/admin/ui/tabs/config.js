import { $, api, apiWrite, esc, authBadge, usageBars, openTokenDialog, TOKEN_KEY } from "../core.js";

export async function loadConfig(view) {
  let editingKeyId = null; // 非空 = 正在编辑该 proxy key（保存走 PUT）
  const render = async () => {
    // mcp-servers / tools 是 key 表单范围选择的数据源，端点未就绪时降级为空
    const [resources, keys, mcps, toolsData] = await Promise.all([
      api("/admin/resources"),
      api("/admin/proxy-keys"),
      api("/admin/mcp-servers").catch(() => null),
      api("/admin/tools").catch(() => null),
    ]);
    const mcpList = Array.isArray(mcps) ? mcps : [];
    const allTools = (toolsData && toolsData.tools) || [];
    // server 多选 = api resources ∪ mcp servers 的 id
    const serverOpts = resources.map(r => ({ id: r.id, kind: "api" }))
      .concat(mcpList.map(m => ({ id: m.id, kind: "mcp" })));
    const toolServers = [...new Set(allTools.map(t => t.resource_id).filter(Boolean))];
    const srcHint = [mcps === null ? "/admin/mcp-servers" : "", toolsData === null ? "/admin/tools" : ""]
      .filter(Boolean).join("、");
    let h = '<h3>配置校验与导出</h3><div class="toolbar"><button id="cfg-validate">校验当前配置</button>'
      + '<button id="cfg-export">导出 YAML</button>'
      + '<span id="cfg-export-msg" class="hint" style="padding:6px;text-align:left"></span></div><div id="cfg-vresult"></div>';
    // resources
    h += '<h3>资源管理</h3><button id="cfg-new-res">+ 新建资源</button>'
      + '<div id="cfg-res-form" style="display:none;margin:8px 0"><div class="card">'
      + '<div style="display:flex;flex-wrap:wrap;gap:6px;align-items:end">'
      + '<label>ID<br><input id="nr-id" size="12"></label>'
      + '<label>领域<br><input id="nr-domain" size="12"></label>'
      + '<label>提供商<br><input id="nr-provider" size="12"></label>'
      + '<label>基础 URL<br><input id="nr-url" size="24"></label>'
      + '<label>描述<br><input id="nr-desc" size="20"></label>'
      + '<button id="nr-save">创建</button><button id="nr-cancel">取消</button>'
      + '</div></div></div>';
    h += '<div class="tablewrap"><table><thead><tr><th>ID</th><th>领域</th><th>提供商</th><th>基础 URL</th><th>端点数</th><th></th></tr></thead><tbody>';
    resources.forEach(r => {
      h += '<tr><td>' + esc(r.id) + '</td><td>' + esc(r.domain) + '</td><td>' + esc(r.provider)
        + '</td><td>' + esc(r.base_url) + '</td><td>' + esc(r.endpoint_count)
        + '</td><td><button class="cfg-del-res" data-id="' + esc(r.id) + '">删除</button></td></tr>';
    });
    if (!resources.length) h += '<tr><td colspan="6" class="empty">无资源</td></tr>';
    h += '</tbody></table></div>';
    // proxy keys：结构化范围（多选）+ 限额；正则收进「高级」折叠区
    h += '<h3>Proxy Key 管理</h3><button id="cfg-new-key">+ 新建 Proxy Key</button>'
      + '<div id="cfg-key-form" style="display:none;margin:8px 0"><div class="card">'
      + '<div style="display:flex;flex-wrap:wrap;gap:6px;align-items:end">'
      + '<label>ID<br><input id="nk-id" size="12"></label>'
      + '<label>显示名<br><input id="nk-name" size="12"></label>'
      + '<label>页大小<br><input id="nk-ps" size="4" value="20"></label>'
      + '<label>rps<br><input id="nk-rps" size="4"></label>'
      + '<label>rpm<br><input id="nk-rpm" size="4"></label>'
      + '<label>最大调用<br><input id="nk-calls" size="7"></label></div>'
      + '<div style="display:flex;flex-wrap:wrap;gap:16px;margin-top:8px">'
      + '<label>允许的 MCP/资源（allowed_servers）<br><select id="nk-servers" multiple size="6" style="min-width:200px">'
      + serverOpts.map(o => '<option value="' + esc(o.id) + '">' + esc(o.id) + '（' + o.kind + '）</option>').join("")
      + '</select></label>'
      + '<div><label>允许的工具（allowed_tool_names）</label><br>'
      + '<input id="nk-toolfilter" placeholder="按名称过滤" size="14"> '
      + '<select id="nk-toolserver"><option value="">全部服务</option>'
      + toolServers.map(sid => '<option>' + esc(sid) + '</option>').join("") + '</select><br>'
      + '<select id="nk-tools" multiple size="6" style="min-width:280px;margin-top:4px">'
      + allTools.map(t => '<option value="' + esc(t.name) + '" data-res="' + esc(t.resource_id || "") + '">' + esc(t.name) + '</option>').join("")
      + '</select></div></div>'
      + (srcHint ? '<div class="hint" style="padding:4px 0 0;text-align:left">' + esc(srcHint) + ' 端点未就绪或无权限，范围选项可能不全</div>' : "")
      + '<details style="margin-top:8px"><summary style="cursor:pointer;color:var(--muted);font-size:12px">高级：正则范围（allowed_tools / denied_tools）</summary>'
      + '<div style="display:flex;flex-wrap:wrap;gap:6px;margin-top:6px">'
      + '<label>允许（正则，逗号分隔）<br><input id="nk-allow" size="26"></label>'
      + '<label>拒绝（正则，逗号分隔）<br><input id="nk-deny" size="26"></label></div></details>'
      + '<div class="toolbar" style="margin:10px 0 0"><button id="nk-save">创建</button><button id="nk-cancel">取消</button></div>'
      + '</div></div>';
    h += '<div class="tablewrap"><table><thead><tr><th>ID</th><th>显示名</th><th>认证</th><th>用量/配额</th><th>服务</th><th>工具名</th><th>限额</th><th>正则范围</th><th>页大小</th><th></th></tr></thead><tbody>';
    keys.forEach((k, i) => {
      const lim = k.limits || {};
      const limStr = ["rps", "rpm", "max_calls"].filter(x => lim[x] != null).map(x => x + " " + lim[x]).join(" · ");
      const tn = k.allowed_tool_names || [];
      const rx = [(k.allowed_tools || []).length ? "允许: " + k.allowed_tools.join(", ") : "",
        (k.denied_tools || []).length ? "拒绝: " + k.denied_tools.join(", ") : ""].filter(Boolean).join("\n");
      h += '<tr><td>' + esc(k.id) + '</td><td>' + esc(k.display_name || "") + '</td>'
        + '<td>' + authBadge(k.auth_mode)
        + (k.expires_at ? '<div style="color:var(--muted);font-size:11px">到期 ' + esc(k.expires_at) + '</div>' : "") + '</td>'
        + '<td>' + usageBars(k.usage) + '</td>'
        + '<td>' + esc((k.allowed_servers || []).join(", ")) + '</td>'
        + '<td' + (tn.length ? ' title="' + esc(tn.join("\n")) + '"' : "") + '>' + (tn.length ? tn.length + " 个" : "") + '</td>'
        + '<td>' + esc(limStr) + '</td><td>' + esc(rx) + '</td>'
        + '<td>' + esc(k.default_tool_page_size ?? "") + '</td>'
        + '<td><button class="cfg-edit-key" data-i="' + i + '">编辑</button> '
        + '<button class="cfg-tok-issue" data-i="' + i + '">' + (k.auth_mode === "token" ? "轮换 token" : "签发 token") + '</button> '
        + (k.auth_mode === "token" ? '<button class="cfg-tok-revoke" data-id="' + esc(k.id) + '">吊销</button> ' : "")
        + '<button class="cfg-del-key" data-id="' + esc(k.id) + '">删除</button></td></tr>';
    });
    if (!keys.length) h += '<tr><td colspan="10" class="empty">无 Proxy Key</td></tr>';
    h += '</tbody></table></div>';
    view.innerHTML = h;
    // events
    $("#cfg-validate").addEventListener("click", async () => {
      try {
        const v = await api("/admin/config/validate");
        let t = v.valid ? '<span style="color:var(--ok)">✓ 配置有效</span>' : '<span style="color:var(--err)">✗ 配置存在问题</span>';
        t += ' · 资源 ' + v.resource_count + ' · Proxy Key ' + v.proxy_key_count + ' · MCP ' + v.mcp_server_count;
        if (v.issues.length) t += '<br>' + v.issues.map(i => '<span style="color:var(--' + (i.level === 'error' ? 'err' : 'muted') + ')">[' + esc(i.level) + '] ' + esc(i.target) + ': ' + esc(i.message) + '</span>').join('<br>');
        $("#cfg-vresult").innerHTML = '<div class="card" style="margin-top:8px">' + t + '</div>';
      } catch (e) { $("#cfg-vresult").innerHTML = '<p class="empty">' + esc(e.message) + '</p>'; }
    });
    // 导出当前合并配置快照（api() 只吃 JSON，这里要拿 YAML 原文，走裸 fetch + Blob 下载）
    $("#cfg-export").addEventListener("click", async () => {
      const msg = $("#cfg-export-msg");
      msg.textContent = "导出中…";
      try {
        const res = await fetch("/admin/config/export", {
          headers: { Authorization: "Bearer " + (sessionStorage.getItem(TOKEN_KEY) || ""), Accept: "text/yaml" },
        });
        if (!res.ok) throw new Error("HTTP " + res.status);
        const url = URL.createObjectURL(await res.blob());
        const a = document.createElement("a");
        a.href = url; a.download = "gateway-export.yaml"; a.click();
        URL.revokeObjectURL(url);
        msg.textContent = "已导出";
      } catch (e) { msg.textContent = "导出失败：" + e.message + "（端点未就绪或无权限）"; }
    });
    $("#cfg-new-res").addEventListener("click", () => { $("#cfg-res-form").style.display = "block"; });
    $("#nr-cancel").addEventListener("click", () => { $("#cfg-res-form").style.display = "none"; });
    $("#nr-save").addEventListener("click", async () => {
      try {
        await apiWrite("POST", "/admin/resources", { id: $("#nr-id").value, domain: $("#nr-domain").value, provider: $("#nr-provider").value, base_url: $("#nr-url").value, description: $("#nr-desc").value });
        await render();
      } catch (e) { alert(e.message); }
    });
    view.querySelectorAll(".cfg-del-res").forEach(b => b.addEventListener("click", async () => {
      if (!confirm("删除资源 " + b.dataset.id + "？")) return;
      try { await apiWrite("DELETE", "/admin/resources/" + encodeURIComponent(b.dataset.id)); await render(); }
      catch (e) { alert(e.message); }
    }));
    // key 表单：新建与编辑共用；编辑时 ID 锁定、保存走 PUT
    const openKeyForm = k => {
      editingKeyId = k ? k.id : null;
      $("#cfg-key-form").style.display = "block";
      $("#nk-id").value = k ? k.id : ""; $("#nk-id").disabled = !!k;
      $("#nk-name").value = k?.display_name || "";
      $("#nk-ps").value = k?.default_tool_page_size ?? 20;
      const lim = k?.limits || {};
      $("#nk-rps").value = lim.rps ?? ""; $("#nk-rpm").value = lim.rpm ?? ""; $("#nk-calls").value = lim.max_calls ?? "";
      $("#nk-allow").value = (k?.allowed_tools || []).join(", ");
      $("#nk-deny").value = (k?.denied_tools || []).join(", ");
      const setSel = (el, vals) => [...el.options].forEach(o => { o.selected = vals.includes(o.value); });
      setSel($("#nk-servers"), k?.allowed_servers || []);
      setSel($("#nk-tools"), k?.allowed_tool_names || []);
      $("#nk-save").textContent = k ? "保存" : "创建";
    };
    $("#cfg-new-key").addEventListener("click", () => openKeyForm(null));
    view.querySelectorAll(".cfg-edit-key").forEach(b =>
      b.addEventListener("click", () => openKeyForm(keys[+b.dataset.i])));
    view.querySelectorAll(".cfg-tok-issue").forEach(b =>
      b.addEventListener("click", () => openTokenDialog(keys[+b.dataset.i], render)));
    view.querySelectorAll(".cfg-tok-revoke").forEach(b => b.addEventListener("click", async () => {
      if (!confirm("吊销 " + b.dataset.id + " 的 token？吊销后该 key 回到 legacy（id-only）模式")) return;
      try { await apiWrite("DELETE", "/admin/proxy-keys/" + encodeURIComponent(b.dataset.id) + "/token"); await render(); }
      catch (e) { alert(e.message + "（端点未就绪或无权限）"); }
    }));
    $("#nk-cancel").addEventListener("click", () => { $("#cfg-key-form").style.display = "none"; editingKeyId = null; });
    // 工具多选过滤：文本 + 按 server（隐藏不匹配项，已选中项保持选中）
    const filterTools = () => {
      const f = $("#nk-toolfilter").value.trim().toLowerCase();
      const srv = $("#nk-toolserver").value;
      [...$("#nk-tools").options].forEach(o => {
        o.hidden = Boolean((f && !o.value.toLowerCase().includes(f)) || (srv && o.dataset.res !== srv));
      });
    };
    $("#nk-toolfilter").addEventListener("input", filterTools);
    $("#nk-toolserver").addEventListener("change", filterTools);
    $("#nk-save").addEventListener("click", async () => {
      const split = s => s ? s.split(",").map(x => x.trim()).filter(Boolean) : [];
      const picked = el => [...el.selectedOptions].map(o => o.value);
      const body = {
        id: editingKeyId || $("#nk-id").value.trim(),
        display_name: $("#nk-name").value,
        allowed_tools: split($("#nk-allow").value),
        denied_tools: split($("#nk-deny").value),
        default_tool_page_size: parseInt($("#nk-ps").value, 10) || 20,
        allowed_servers: picked($("#nk-servers")),
        allowed_tool_names: picked($("#nk-tools")),
      };
      const lim = {};
      [["rps", "#nk-rps"], ["rpm", "#nk-rpm"], ["max_calls", "#nk-calls"]].forEach(([kk, sq]) => {
        const v = parseInt($(sq).value, 10);
        if (v > 0) lim[kk] = v;
      });
      if (Object.keys(lim).length) body.limits = lim;
      try {
        if (editingKeyId) await apiWrite("PUT", "/admin/proxy-keys/" + encodeURIComponent(editingKeyId), body);
        else await apiWrite("POST", "/admin/proxy-keys", body);
        editingKeyId = null;
        await render();
      } catch (e) { alert(e.message); }
    });
    view.querySelectorAll(".cfg-del-key").forEach(b => b.addEventListener("click", async () => {
      if (!confirm("删除 Proxy Key " + b.dataset.id + "？")) return;
      try { await apiWrite("DELETE", "/admin/proxy-keys/" + encodeURIComponent(b.dataset.id)); await render(); }
      catch (e) { alert(e.message); }
    }));
  };
  await render();
}
