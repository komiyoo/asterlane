import { $, api, apiWrite, esc } from "../core.js";

export async function loadResources(view) {
  const render = async () => {
    const resources = await api("/admin/resources");
    let h = '<div class="toolbar"><button id="cfg-new-res">+ 新建资源</button></div>'
      + '<div id="cfg-res-form" style="display:none;margin:8px 0"><div class="card">'
      + '<div class="form-row">'
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
    view.innerHTML = h;
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
  };
  await render();
}
