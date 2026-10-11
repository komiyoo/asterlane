export const zh = {
  hint: "校验和导出只读取当前合并快照，不会修改配置。",
  validate: "校验当前配置",
  export: "导出 YAML",
  exported: "已导出",
  exportFailed: "导出失败：{{detail}}",
  checking: "正在校验…",
  valid: "配置有效",
  invalid: "配置存在问题",
  counts: "资源 {{resources}} · Proxy Key {{keys}} · MCP {{mcp}}",
  errors: "错误",
  warnings: "警告",
  issueCount: "{{title}}（{{count}}）",
};

export const en = {
  hint: "Validate and export only read the current merged snapshot. They do not change configuration.",
  validate: "Validate current configuration",
  export: "Export YAML",
  exported: "Exported",
  exportFailed: "Export failed: {{detail}}",
  checking: "Validating…",
  valid: "Configuration is valid",
  invalid: "Configuration has problems",
  counts: "Resources {{resources}} · Proxy keys {{keys}} · MCP {{mcp}}",
  errors: "Errors",
  warnings: "Warnings",
  issueCount: "{{title}} ({{count}})",
} satisfies typeof zh;
