export const zh = {
  connection: "连接失败",
  connectionStatus: "连接失败（HTTP {{status}}）",
  auth: "认证失败",
  authDetail: "认证失败（{{code}}）：{{message}}（{{requestId}}）",
  cancelled: "请求已取消",
};

export const en = {
  connection: "Connection failed",
  connectionStatus: "Connection failed (HTTP {{status}})",
  auth: "Authentication failed",
  authDetail: "Authentication failed ({{code}}): {{message}} ({{requestId}})",
  cancelled: "Request cancelled",
} satisfies typeof zh;
