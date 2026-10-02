//! 测试用的可靠日志捕获：全进程只装一个全局 subscriber，各用例按线程取回自己的日志。
//!
//! 不能让每个用例各自用 `tracing::subscriber::set_default` 装线程级 subscriber：tracing
//! 在只有一个线程级 subscriber 存活时，首次触发某个回调点的线程只按自己线程的 subscriber
//! 计算并缓存「是否关注」。并行时没装 subscriber 的用例若先触发某条日志的回调点，会把它缓存
//! 为 never，之后装了捕获的用例就收不到这条日志：「日志里不含 token」之类的否定断言会在
//! 什么都没捕获的情况下空过，肯定断言则会偶发失败。
//!
//! 这里的做法：全局 subscriber 对所有线程都放行回调点（`Interest::sometimes()`，缓存与
//! 线程无关），再在 `enabled` 里按当前线程是否正在捕获、捕获的级别与是否叠加凭据日志上限
//! 决定放不放行；输出写进当前线程的缓冲区。`#[tokio::test]` 用单线程运行时，一个用例的
//! 日志都在该用例所在线程上产生。
//!
//! 用法：`let logs = capture_logs(Level::TRACE, true);` 持有到断言完成；否定断言用
//! [`LogCapture::text_containing`] 取日志，同时证明捕获确实生效。
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use asterlane::observability::log_filter::credential_log_cap;
use tracing::subscriber::Interest;
use tracing::{Level, Metadata, Subscriber};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

/// 一个用例捕获到的日志。
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

/// 当前线程的捕获设置。
#[derive(Clone)]
struct ThreadCapture {
    buffer: LogBuffer,
    max_level: Level,
    /// 是否叠加 `main.rs` 用的凭据日志上限层。
    credential_cap: bool,
}

thread_local! {
    static CURRENT: RefCell<Option<ThreadCapture>> = const { RefCell::new(None) };
}

/// 全局过滤：当前线程没有在捕获就不放行；否则按捕获级别与（可选的）凭据日志上限放行。
/// 回调点的缓存固定为 `sometimes`，每次事件都重新看当前线程的设置。
struct ThreadFilter<L>(L);

impl<S: Subscriber, L: Layer<S>> Layer<S> for ThreadFilter<L> {
    fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
        Interest::sometimes()
    }

    fn enabled(&self, metadata: &Metadata<'_>, ctx: Context<'_, S>) -> bool {
        CURRENT.with(|current| match current.borrow().as_ref() {
            None => false,
            Some(capture) => {
                *metadata.level() <= capture.max_level
                    && (!capture.credential_cap || self.0.enabled(metadata, ctx))
            }
        })
    }
}

/// 把 fmt 层的输出写进当前线程的缓冲区。
struct ThreadWriter;

struct CurrentBuffer(Option<LogBuffer>);

impl<'a> MakeWriter<'a> for ThreadWriter {
    type Writer = CurrentBuffer;

    fn make_writer(&'a self) -> CurrentBuffer {
        CurrentBuffer(CURRENT.with(|current| current.borrow().as_ref().map(|c| c.buffer.clone())))
    }
}

impl Write for CurrentBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(buffer) = &self.0 {
            buffer.0.lock().unwrap().extend_from_slice(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 安装全局 subscriber（只装一次）。装好后重建一次回调点缓存，覆盖安装期间恰好有别的线程
/// 触发的回调点。
fn install() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let subscriber = tracing_subscriber::registry()
            .with(ThreadFilter(credential_log_cap()))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(ThreadWriter)
                    .with_ansi(false),
            );
        tracing::subscriber::set_global_default(subscriber)
            .expect("本测试文件内不应有其他全局 subscriber");
        tracing::callsite::rebuild_interest_cache();
    });
}

/// 从现在起捕获当前线程的日志，直到返回值被丢弃。`max_level` 相当于 `RUST_LOG` 的级别；
/// `credential_cap` 为真时叠加 `main.rs` 用的凭据日志上限层。
pub fn capture_logs(max_level: Level, credential_cap: bool) -> LogCapture {
    install();
    let buffer = LogBuffer::default();
    let previous = CURRENT.with(|current| {
        current.replace(Some(ThreadCapture {
            buffer: buffer.clone(),
            max_level,
            credential_cap,
        }))
    });
    LogCapture { buffer, previous }
}

pub struct LogCapture {
    buffer: LogBuffer,
    previous: Option<ThreadCapture>,
}

impl Drop for LogCapture {
    fn drop(&mut self) {
        CURRENT.with(|current| *current.borrow_mut() = self.previous.take());
    }
}

impl LogCapture {
    /// 到目前为止捕获到的全部日志。
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.buffer.0.lock().unwrap()).into_owned()
    }

    /// 同 [`Self::text`]，并断言日志非空且含 `marker`（该用例路径上必然出现的一条日志）。
    /// 否定断言（日志里没有 token / code / secret）用它取日志，避免什么都没捕获时空过。
    /// 失败信息不打印日志内容，避免把凭据值带进测试输出。
    pub fn text_containing(&self, marker: &str) -> String {
        let text = self.text();
        assert!(
            !text.is_empty(),
            "没有捕获到任何日志，否定断言会空过（期望含 {marker:?}）"
        );
        assert!(
            text.contains(marker),
            "捕获到 {} 字节日志，但不含预期的 {marker:?}",
            text.len()
        );
        text
    }
}
