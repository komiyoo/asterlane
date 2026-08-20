import { $, api, apiWrite, esc } from "../core.js";

export async function loadResources(view) {
  const requireSecretRef = (v, label) => {
    if (!v.startsWith("secret://")) {
      alert(label + " 必须是 secret:// 引用，不接受明文");
      return false;
    }
    return true;
  };
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
      + '<label>认证<br><select id="nr-auth">'
      + ["none", "bearer", "header"].map(t => "<option>" + t + "</option>").join("")
      + '</select></label>'
      + '<label id="nr-l-token">Token ref<br><input id="nr-token" size="24" placeholder="secret://…"></label>'
      + '<label id="nr-l-hname">Header 名<br><input id="nr-hname" size="10" placeholder="x-api-key"></label>'
      + '<label id="nr-l-hval">Header ref<br><input id="nr-hval" size="24" placeholder="secret://…"></label>'
      + '<label>池策略<br><select id="nr-strategy">'
      + ["round_robin", "random", "least_requests", "fastest_response", "weighted"].map(t => "<option>" + t + "</option>").join("")
      + '</select></label>'
      + '<label>Key pool refs<br><textarea id="nr-refs" rows="3" cols="28" placeholder="每行一个 secret://"></textarea></label>'
      + '<button id="nr-save">创建</button><button id="nr-cancel">取消</button>'
      + '</div><div class="hint" style="padding:6px 0 0;text-align:left">凭据与池内 key 只接受 secret://，不接受明文</div></div></div>';
    h += '<div class="tablewrap"><table><thead><tr><th>ID</th><th>领域</th><th>提供商</th><th>基础 URL</th><th>认证</th><th>池大小</th><th>端点数</th><th></th></tr></thead><tbody>';
    resources.forEach(r => {
      h += '<tr><td>' + esc(r.id) + '</td><td>' + esc(r.domain) + '</td><td>' + esc(r.provider)
        + '</td><td>' + esc(r.base_url) + '</td><td>' + esc(r.auth_type || "none")
        + '</td><td>' + esc(r.key_pool_size ?? 0)
        + '</td><td>' + esc(r.endpoint_count)
        + '</td><td><button class="cfg-del-res" data-id="' + esc(r.id) + '">删除</button></td></tr>';
    });
    if (!resources.length) h += '<tr><td colspan="8" class="empty">无资源</td></tr>';
    h += '</tbody></table></div>';
    view.innerHTML = h;
    const syncAuth = () => {
      const t = $("#nr-auth").value;
      $("#nr-l-token").style.display = t === "bearer" ? "" : "none";
      $("#nr-l-hname").style.display = t === "header" ? "" : "none";
      $("#nr-l-hval").style.display = t === "header" ? "" : "none";
    };
    $("#nr-auth").addEventListener("change", syncAuth);
    syncAuth();
    $("#cfg-new-res").addEventListener("click", () => { $("#cfg-res-form").style.display = "block"; });
    $("#nr-cancel").addEventListener("click", () => { $("#cfg-res-form").style.display = "none"; });
    $("#nr-save").addEventListener("click", async () => {
      const body = {
        id: $("#nr-id").value, domain: $("#nr-domain").value, provider: $("#nr-provider").value,
        base_url: $("#nr-url").value, description: $("#nr-desc").value,
      };
      const at = $("#nr-auth").value;
      if (at === "bearer") {
        const v = $("#nr-token").value.trim();
        if (!v) { alert("Token ref 不能为空"); return; }
        if (!requireSecretRef(v, "Token ref")) return;
        body.auth = { type: "bearer", token_ref: v };
      } else if (at === "header") {
        const name = $("#nr-hname").value.trim(), v = $("#nr-hval").value.trim();
        if (!name) { alert("Header 名不能为空"); return; }
        if (!v) { alert("Header ref 不能为空"); return; }
        if (!requireSecretRef(v, "Header ref")) return;
        body.auth = { type: "header", name, value_ref: v };
      } else body.auth = { type: "none" };
      const refs = $("#nr-refs").value.split("\n").map(s => s.trim()).filter(Boolean);
      if (refs.length) {
        for (const r of refs) {
          if (!requireSecretRef(r, "Key pool ref")) return;
        }
        body.key_pool = { strategy: $("#nr-strategy").value, keys: refs.map(r => ({ ref: r, weight: 1 })) };
      }
      try {
        await apiWrite("POST", "/admin/resources", body);
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
