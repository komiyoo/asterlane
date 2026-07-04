//! Key 标识与状态枚举（见 `docs/architecture.md` Key Pool And Load Balancing）。
//!
//! 借鉴 NyaProxy（`core/control.py`）但按 Asterlane 模型重新设计：
//! - `KeyState` 使用 `Instant` 替代 NyaProxy 的伪时间戳填充冷却。
//! - `KeyId` 以序号索引，`Display` 输出脱敏标识，**不持明文密钥**——
//!   明文由 `secrets` 模块解析、`proxy` 模块注入。

use std::fmt;
use std::fmt::{Display, Formatter};
use std::time::Instant;

/// 上游 key 的稳定标识（序号）。
///
/// 本模块只管理 `KeyId` 的状态与选取，**不持有明文密钥**。
/// 明文 secret 由 `secrets` 模块解析，`proxy` 执行层在写入上游
/// Authorization header 的瞬间 `expose_secret`，其余时刻为 `SecretString`。
///
/// `Display` 输出脱敏标识（如 `key#0001`），可安全出现在日志、错误消息与
/// `upstream_key_ref` tracing 字段中。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyId(u64);

impl KeyId {
    /// 从序号构造 `KeyId`。
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// 返回底层序号。
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Display for KeyId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // 脱敏:输出序号标识,不含明文
        write!(f, "key#{:04}", self.0)
    }
}

/// Key 的运行时状态。
///
/// 替代 NyaProxy 的伪时间戳冷却：使用 `Instant` 表达冷却到期时刻，
/// 由 `Instant` 的单调语义保证比较正确性。
///
/// - `Available`：空闲，无活跃租约。
/// - `CoolingUntil`：冷却中（429/5xx 触发），到期后恢复为 `Available`。
/// - `Leased`：有活跃租约，仍可承接更多并发租约（`count` 为当前活跃数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    /// 空闲可用，无活跃租约。
    Available,
    /// 冷却中，到期时刻由 `Instant` 指定。到期前跳过该 key。
    CoolingUntil(Instant),
    /// 已租出，`count` 为当前活跃租约数。`count > 0` 时仍可承接更多租约。
    Leased { count: u32 },
}

impl KeyState {
    /// 返回当前活跃租约数（`Available`/`CoolingUntil` 为 0）。
    pub const fn active_count(&self) -> u32 {
        match self {
            Self::Leased { count } => *count,
            _ => 0,
        }
    }

    /// 是否处于冷却中（不判断是否已到期）。
    pub const fn is_cooling(&self) -> bool {
        matches!(self, Self::CoolingUntil(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_id_display_is_redacted() {
        let id = KeyId::new(1);
        assert_eq!(id.to_string(), "key#0001");
        let id2 = KeyId::new(42);
        assert_eq!(id2.to_string(), "key#0042");
        assert!(!id.to_string().contains("sk-"));
    }

    #[test]
    fn key_id_ordering_preserves_numeric_order() {
        assert!(KeyId::new(1) < KeyId::new(2));
        assert_eq!(KeyId::new(3), KeyId::new(3));
    }

    #[test]
    fn available_active_count_is_zero() {
        assert_eq!(KeyState::Available.active_count(), 0);
    }

    #[test]
    fn leased_active_count_returns_count() {
        assert_eq!(KeyState::Leased { count: 3 }.active_count(), 3);
    }

    #[test]
    fn cooling_is_cooling_true() {
        let s = KeyState::CoolingUntil(Instant::now());
        assert!(s.is_cooling());
        assert!(!KeyState::Available.is_cooling());
        assert!(!KeyState::Leased { count: 1 }.is_cooling());
    }
}
