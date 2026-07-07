import { $, api, esc, TOKEN_KEY } from "../core.js";

export async function loadConfig(view) {
  view.innerHTML = '<h3>配置校验与导出</h3><div class="toolbar"><button id="cfg-validate">校验当前配置</button>'
    + '<button id="cfg-export">导出 YAML</button>'
    + '<span id="cfg-export-msg" class="hint" style="padding:6px;text-align:left"></span></div><div id="cfg-vresult"></div>';
  $("#cfg-validate").addEventListener("click", async () => {
    try {
      const v = await api("/admin/config/validate");
      let t = v.valid ? '<span style="color:var(--ok)">✓ 配置有效</span>' : '<span style="color:var(--err)">✗ 配置存在问题</span>';
      t += ' · 资源 ' + v.resource_count + ' · Proxy Key ' + v.proxy_key_count + ' · MCP ' + v.mcp_server_count;
      if (v.issues.length) t += '<br>' + v.issues.map(i => '<span style="color:var(--' + (i.level === 'error' ? 'err' : 'muted') + ')">[' + esc(i.level) + '] ' + esc(i.target) + ': ' + esc(i.message) + '</span>').join('<br>');
      $("#cfg-vresult").innerHTML = '<div class="card" style="margin-top:8px">' + t + '</div>';
    } catch (e) { $("#cfg-vresult").innerHTML = '<p class="empty">' + esc(e.message) + '</p>'; }
  });
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
}
