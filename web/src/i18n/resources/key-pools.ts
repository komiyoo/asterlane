export const zh = {
  empty: "未配置密钥池",
  emptyDescription: "资源级 key_pool 尚未配置。",
  columns: {
    id: "密钥 ID",
    state: "状态",
    leased: "租借数",
    cooling: "冷却剩余(ms)",
    weight: "权重",
    latency: "加权延迟(ms)",
    ref: "引用",
  },
};

export const en = {
  empty: "No key pools configured",
  emptyDescription: "No resource-level key_pool is configured.",
  columns: {
    id: "Key ID",
    state: "State",
    leased: "Leased",
    cooling: "Cooling left (ms)",
    weight: "Weight",
    latency: "Weighted latency (ms)",
    ref: "Reference",
  },
} satisfies typeof zh;
