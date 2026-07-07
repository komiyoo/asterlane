import { $, api, apiWrite, esc, authBadge, usageBars, openTokenDialog } from "../core.js";

export async function loadProxyKeys(view) {
  let editingKeyId = null;
  const render = async () => {
    const [keys, resources, mcps, toolsData] = await Promise.all([
      api("/admin/proxy-keys"),
      api("/admin/resources"),
      api("/admin/mcp-servers").catch(() => null),
      api("/admin/tools").catch(() => null),
    ]);
    const mcpList = Array.isArray(mcps) ? mcps : [];
    const allTools = (toolsData && toolsData.tools) || [];
    const serverOpts = resources.map(r => ({ id: r.id, kind: "api" }))
      .concat(mcpList.map(m => ({ id: m.id, kind: "mcp" })));
    const toolServers = [...new Set(allTools.map(t => t.resource_id).filter(Boolean))];
    const srcHint = [mcps === null ? "/admin/mcp-servers" : "", toolsData === null ? "/admin/tools" : ""]
      .filter(Boolean).join("、");
    let h = '<div class="toolbar"><button id="cfg-new-key">+ 新建 Proxy Key</button></div>'
      + '<div id="cfg-key-form" style="display:none;margin:8px 0"><div class="card">'
      + '<div class="form-row">'
      + '<label>ID<br><input id="nk-id" size="12"></label>'
      + '<label>显示名<br><input id="nk-name" size="12"></label>'
      + '<label>页大小<br><input id="nk-ps" size="4" value="20"></label>'
      + '<label>rps<br><input id="nk-rps" size="4"></label>'
      + '<label>rpm<br><input id="nk-rpm" size="4"></label>'
      + '<label>最大调用<br><input id="nk-calls" size="7"></label></div>'
      + '<div class="form-row" style="gap:16px;margin-top:8px">'
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
      + '<div class="form-row" style="margin-top:6px">'
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
    // key 表单：新建与编辑共用
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
