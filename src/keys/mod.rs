//! keys 模块:上游 key 池、冷却、负载均衡与 RAII guard。
//!
//! 设计依据见 `docs/architecture.md` Key Pool And Load Balancing。
//! 借鉴 NyaProxy（`core/control.py`、`services/lb.py`）并按 Asterlane 模型
//! 重新设计:
//!
//! - **`KeyId`**（[`state`]）:序号 newtype,`Display` 输出脱敏标识,不持明文。
//! - **`KeyState`**（[`state`]）:`Available` / `CoolingUntil(Instant)` / `Leased{count}`,
//!   用 `Instant` 替代 NyaProxy 的伪时间戳。
//! - **`KeyPool`**（[`pool`]）:管理 `HashMap<KeyId, KeyEntry>`,用 `std::sync::Mutex`
//!   保护(临界区无 await,RAII `Drop` 可同步释放)。
//! - **`KeyGuard`**（[`pool`]）:RAII,`Drop` 自动 `release`,避免 NyaProxy 手工
//!   `release_*` 四连调漏调。guard 只持 `KeyId`,不持明文。
//! - **`LoadBalanceStrategy`**（[`strategy`]）:`RoundRobin` / `Random` /
//!   `LeastRequests` / `FastestResponse`(EWMA) / `Weighted`。
//! - **`KeyPoolError`**（[`error`]）:`NoAvailableKey` 等,通过 `From` 接入
//!   `AsterlaneError::Internal { code: ProxyRetryExhausted, .. }`。
//!
//! 本模块只管 `KeyId` 状态;明文密钥由 `secrets` 模块解析、`proxy` 模块注入。

pub mod error;
pub mod pool;
pub mod state;
pub mod strategy;

pub use error::KeyPoolError;
pub use pool::{KeyGuard, KeyPool, KeyPoolBuilder};
pub use state::{KeyId, KeyState};
pub use strategy::{KeyCandidate, LoadBalanceStrategy};
