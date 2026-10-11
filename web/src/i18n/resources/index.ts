import { en as apiEn, zh as apiZh } from "./api.ts";
import { en as auditEn, zh as auditZh } from "./audit.ts";
import { en as commonEn, zh as commonZh } from "./common.ts";
import { en as configEn, zh as configZh } from "./config.ts";
import { en as errorsEn, zh as errorsZh } from "./errors.ts";
import { en as eventsEn, zh as eventsZh } from "./events.ts";
import { en as formatEn, zh as formatZh } from "./format.ts";
import { en as keyPoolsEn, zh as keyPoolsZh } from "./key-pools.ts";
import { en as kumoEn, zh as kumoZh } from "./kumo.ts";
import { en as loginEn, zh as loginZh } from "./login.ts";
import { en as mcpEn, zh as mcpZh } from "./mcp.ts";
import { en as overviewEn, zh as overviewZh } from "./overview.ts";
import { en as proxyKeysEn, zh as proxyKeysZh } from "./proxy-keys.ts";
import { en as resourcesEn, zh as resourcesZh } from "./resources.ts";
import { en as securityEn, zh as securityZh } from "./security.ts";
import { en as shellEn, zh as shellZh } from "./shell.ts";
import { en as toolsEn, zh as toolsZh } from "./tools.ts";
import { en as usageEn, zh as usageZh } from "./usage.ts";

export const zh = {
  api: apiZh,
  audit: auditZh,
  common: commonZh,
  config: configZh,
  errors: errorsZh,
  events: eventsZh,
  format: formatZh,
  keyPools: keyPoolsZh,
  kumo: kumoZh,
  login: loginZh,
  mcp: mcpZh,
  overview: overviewZh,
  proxyKeys: proxyKeysZh,
  resources: resourcesZh,
  security: securityZh,
  shell: shellZh,
  tools: toolsZh,
  usage: usageZh,
};

export const en = {
  api: apiEn,
  audit: auditEn,
  common: commonEn,
  config: configEn,
  errors: errorsEn,
  events: eventsEn,
  format: formatEn,
  keyPools: keyPoolsEn,
  kumo: kumoEn,
  login: loginEn,
  mcp: mcpEn,
  overview: overviewEn,
  proxyKeys: proxyKeysEn,
  resources: resourcesEn,
  security: securityEn,
  shell: shellEn,
  tools: toolsEn,
  usage: usageEn,
} satisfies typeof zh;
