export const zh = {
  dimension: "维度",
  from: "起始时间",
  to: "结束时间",
  invalidTime: "时间格式无效",
  dimensions: {
    tool: "按工具",
    domain: "按领域",
    resource: "按资源",
    proxyKey: "按 proxy key",
    status: "按状态",
    bucket: "按小时（趋势）",
  },
  errorSuffix: " ({{count}} 错)",
  columns: {
    kind: "类型",
    dimension: "维度",
    requests: "请求数",
    errors: "错误数",
    units: "总单元",
    latency: "平均延迟(ms)",
    rateLimit: "限流命中",
  },
};

export const en = {
  dimension: "Dimension",
  from: "Start",
  to: "End",
  invalidTime: "Invalid time",
  dimensions: {
    tool: "By tool",
    domain: "By domain",
    resource: "By resource",
    proxyKey: "By proxy key",
    status: "By status",
    bucket: "By hour (trend)",
  },
  errorSuffix: " ({{count}} errors)",
  columns: {
    kind: "Kind",
    dimension: "Dimension",
    requests: "Requests",
    errors: "Errors",
    units: "Total units",
    latency: "Avg latency (ms)",
    rateLimit: "Rate-limit hits",
  },
} satisfies typeof zh;
