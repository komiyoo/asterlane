//! 顶层 `admin` 节：admin key。

use serde::{Deserialize, Serialize};

/// Admin API 认证配置（见 docs/admin/admin-console.md）。
///
/// admin key 与 proxy key 物理分离：不同配置节、不同校验路径。
/// `keys` 为空时 admin API 与控制台整体不挂载。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminConfig {
    #[serde(default)]
    pub keys: Vec<AdminKey>,
}

/// 单个 admin key：token 只存 secret ref，不存明文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminKey {
    pub id: String,
    /// admin token 的 secret ref（如 `secret://env/ASTERLANE_ADMIN_TOKEN`）。
    pub token_ref: String,
}
