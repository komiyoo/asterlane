//! limits 模块：限流、配额、队列准入
//! （见 `docs/architecture.md` Rate Limit And Queue）。
//!
//! 纠正 NyaProxy `{api}_key_{sk-xxx}` 明文拼接反模式，使用类型化
//! `LimiterKey` 枚举替代字符串拼接。算法使用 governor GCRA（O(1) 内存）。

mod error;
mod key;
mod limiter;
mod queue;

pub use error::LimitError;
pub use key::{ApiId, LimiterKey, PrincipalId};
pub use limiter::RateLimits;
pub use queue::{Priority, QueuePermit, RequestQueue};
