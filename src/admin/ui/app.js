// 控制台模块入口：装配 TABS 注册表、导航与连接引导。
// 由 console.html 以 <script type="module"> 加载（deferred，DOM 就绪后执行）。

import { $, api, esc, TOKEN_KEY } from "./core.js";
import { loadOverview } from "./tabs/overview.js";
import { loadUsage } from "./tabs/usage.js";
import { loadResources } from "./tabs/resources.js";
import { loadMcpServers } from "./tabs/mcp.js";
import { loadTools } from "./tabs/tools.js";
import { loadProxyKeys } from "./tabs/keys.js";
import { loadKeyPools } from "./tabs/keypools.js";
import { loadEvents } from "./tabs/events.js";
import { loadSecurity } from "./tabs/security.js";
import { loadAudit } from "./tabs/audit.js";
import { loadConfig } from "./tabs/config.js";

const TABS = {
  overview: { label: "总览", load: loadOverview },
  usage: { label: "用量", load: loadUsage },
  resources: { label: "资源", load: loadResources },
  mcp: { label: "MCP Servers", load: loadMcpServers },
  tools: { label: "工具", load: loadTools },
  keys: { label: "Proxy Keys", load: loadProxyKeys },
  keypools: { label: "Key Pools", load: loadKeyPools },
  events: { label: "事件", load: loadEvents },
  security: { label: "安全事件", load: loadSecurity },
  audit: { label: "审计", load: loadAudit },
  config: { label: "配置管理", load: loadConfig },
};
let current = "overview";

function buildNav() {
  const nav = $("#nav");
  nav.innerHTML = "";
  for (const [key, tab] of Object.entries(TABS)) {
    const b = document.createElement("button");
    b.textContent = tab.label;
    b.className = key === current ? "active" : "";
    b.addEventListener("click", () => { current = key; buildNav(); loadCurrent(); });
    nav.appendChild(b);
  }
}

async function loadCurrent() {
  const view = $("#view");
  view.innerHTML = '<p class="hint">加载中…</p>';
  try {
    await TABS[current].load(view);
  } catch (e) {
    view.innerHTML = '<p class="empty">' + esc(e.message) + "</p>";
  }
}

async function connect() {
  const conn = $("#conn");
  conn.textContent = "连接中…"; conn.className = "";
  try {
    const health = await api("/admin/health");
    conn.textContent = "已连接 · v" + health.version; conn.className = "ok";
    buildNav();
    await loadCurrent();
  } catch (e) {
    conn.textContent = e.message; conn.className = "err";
    $("#view").innerHTML = '<p class="hint">' + esc(e.message) + "</p>";
  }
}

$("#connect").addEventListener("click", () => {
  sessionStorage.setItem(TOKEN_KEY, $("#token").value.trim());
  connect();
});
$("#token").addEventListener("keydown", e => { if (e.key === "Enter") $("#connect").click(); });

if (sessionStorage.getItem(TOKEN_KEY)) {
  $("#token").value = sessionStorage.getItem(TOKEN_KEY);
  connect();
}
