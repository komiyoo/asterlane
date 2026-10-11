//! 出站地址校验：字面量 IP、DNS 解析结果与重定向逐跳。
//!
//! 开关关闭时调用方不进入本模块。开启后，HTTP 上游、MCP 上游连接与上游 OAuth
//! 共用这里的分类、[`check_url`] 与受控 reqwest client。用户可见错误固定为
//! [`SAFE_MESSAGE`]，不包含解析出的地址。

use std::error::Error as StdError;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::{self, Attempt};

/// 用户可见的固定文案。任何拒绝原因都用这一句，不回显地址。
pub(crate) const SAFE_MESSAGE: &str = "upstream address is not allowed";

/// 与 reqwest `Policy::limited(10)` 一致：`previous` 含最初的 URL，长度大于 10 即停止。
const MAX_REDIRECTS: usize = 10;

/// 只进 tracing 的拒绝原因，不进用户可见消息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockReason {
    Unspecified,
    Loopback,
    Private,
    LinkLocal,
    Cgnat,
    Multicast,
    Reserved,
    Metadata,
    UniqueLocal,
    Hostname,
    Scheme,
    Unpinned,
    InvalidUrl,
    LookupFailed,
    Empty,
}

/// 出站地址被拒绝。`Display` 不含原因与地址。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EgressBlocked {
    reason: BlockReason,
}

impl EgressBlocked {
    fn new(reason: BlockReason) -> Self {
        Self { reason }
    }

    pub(crate) fn reason_name(self) -> &'static str {
        match self.reason {
            BlockReason::Unspecified => "unspecified",
            BlockReason::Loopback => "loopback",
            BlockReason::Private => "private",
            BlockReason::LinkLocal => "link_local",
            BlockReason::Cgnat => "cgnat",
            BlockReason::Multicast => "multicast",
            BlockReason::Reserved => "reserved",
            BlockReason::Metadata => "metadata",
            BlockReason::UniqueLocal => "unique_local",
            BlockReason::Hostname => "hostname",
            BlockReason::Scheme => "scheme",
            BlockReason::Unpinned => "unpinned",
            BlockReason::InvalidUrl => "invalid_url",
            BlockReason::LookupFailed => "lookup_failed",
            BlockReason::Empty => "empty",
        }
    }
}

impl fmt::Display for EgressBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SAFE_MESSAGE)
    }
}

impl StdError for EgressBlocked {}

/// 沿 `source` 链查找地址拒绝。找不到时调用方按普通连接失败处理。
pub(crate) fn find_blocked(error: &(dyn StdError + 'static)) -> Option<EgressBlocked> {
    let mut current = Some(error);
    while let Some(err) = current {
        if let Some(blocked) = err.downcast_ref::<EgressBlocked>() {
            return Some(*blocked);
        }
        current = err.source();
    }
    None
}

/// 分类一个 IP。`None` 表示允许连接。IPv4 映射地址与其他内嵌 IPv4 按内嵌地址分类。
pub(crate) fn classify_ip(ip: IpAddr) -> Option<BlockReason> {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(ip: Ipv4Addr) -> Option<BlockReason> {
    // Azure 实例元数据不落在链路本地网段里。
    if ip == Ipv4Addr::new(168, 63, 129, 16) {
        return Some(BlockReason::Metadata);
    }
    let [a, b, _, _] = ip.octets();
    match a {
        0 => Some(BlockReason::Unspecified),
        10 => Some(BlockReason::Private),
        127 => Some(BlockReason::Loopback),
        // 100.64.0.0/10（含 100.100.100.200）。
        100 if (64..128).contains(&b) => Some(BlockReason::Cgnat),
        169 if b == 254 => Some(BlockReason::LinkLocal),
        172 if (16..32).contains(&b) => Some(BlockReason::Private),
        192 if b == 168 => Some(BlockReason::Private),
        224..=239 => Some(BlockReason::Multicast),
        240..=255 => Some(BlockReason::Reserved),
        _ => None,
    }
}

fn classify_v6(ip: Ipv6Addr) -> Option<BlockReason> {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return classify_v4(v4);
    }
    if ip.is_unspecified() {
        return Some(BlockReason::Unspecified);
    }
    if ip.is_loopback() {
        return Some(BlockReason::Loopback);
    }
    if ip.is_multicast() {
        return Some(BlockReason::Multicast);
    }
    let seg = ip.segments();
    // fe80::/10
    if seg[0] & 0xffc0 == 0xfe80 {
        return Some(BlockReason::LinkLocal);
    }
    // fc00::/7 唯一本地地址，以及已废弃的站点本地 fec0::/10（不在 fc00::/7 内）。
    if seg[0] & 0xfe00 == 0xfc00 || seg[0] & 0xffc0 == 0xfec0 {
        return Some(BlockReason::UniqueLocal);
    }
    embedded_block(seg)
}

/// 内嵌 IPv4 只在内嵌地址本身被拒绝时拒绝。公网内嵌地址仍允许。
fn embedded_block(seg: [u16; 8]) -> Option<BlockReason> {
    if is_nat64(seg) || is_ipv4_compatible(seg) {
        return classify_v4(last_ipv4(seg));
    }
    // 6to4 2002::/16，IPv4 在第 2、3 段。
    if seg[0] == 0x2002 {
        let v4 = Ipv4Addr::new(
            (seg[1] >> 8) as u8,
            (seg[1] & 0xff) as u8,
            (seg[2] >> 8) as u8,
            (seg[2] & 0xff) as u8,
        );
        return classify_v4(v4);
    }
    None
}

fn is_nat64(seg: [u16; 8]) -> bool {
    seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2] == 0 && seg[3] == 0 && seg[4] == 0 && seg[5] == 0
}

fn is_ipv4_compatible(seg: [u16; 8]) -> bool {
    seg[0] == 0 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0 && seg[4] == 0 && seg[5] == 0
}

fn last_ipv4(seg: [u16; 8]) -> Ipv4Addr {
    Ipv4Addr::new(
        (seg[6] >> 8) as u8,
        (seg[6] & 0xff) as u8,
        (seg[7] >> 8) as u8,
        (seg[7] & 0xff) as u8,
    )
}

fn blocked_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "metadata"
        || host == "metadata.google.internal"
        || host == "metadata.azure.internal"
}

/// 请求前检查 URL。
///
/// `dns_pinned` 为 true 时，普通主机名放行，交给 [`PinningResolver`] 在解析后过滤。
/// 为 false（受控 client 没建成）时拒绝包括字面量在内的全部 URL，避免用未校验的
/// client 发出去。字面量 IP 在这里分类；解析器不会被字面量调用。
pub(crate) fn check_url(url: &str, dns_pinned: bool) -> Result<(), EgressBlocked> {
    let parsed =
        reqwest::Url::parse(url).map_err(|_| EgressBlocked::new(BlockReason::InvalidUrl))?;
    check_parsed(&parsed, dns_pinned)
}

fn check_parsed(url: &reqwest::Url, dns_pinned: bool) -> Result<(), EgressBlocked> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(EgressBlocked::new(BlockReason::Scheme));
    }
    if !dns_pinned {
        return Err(EgressBlocked::new(BlockReason::Unpinned));
    }
    let Some(host) = url.host_str() else {
        return Err(EgressBlocked::new(BlockReason::InvalidUrl));
    };
    // URL 解析器已把 `127.1` 这类写法规范成点分 IPv4；IPv6 字面量带方括号，
    // 已被校验过，去掉括号后一定能解析。
    let unbracketed = host.strip_prefix('[').and_then(|h| h.strip_suffix(']'));
    let reason = match unbracketed.unwrap_or(host).parse::<IpAddr>() {
        Ok(ip) => classify_ip(ip),
        Err(_) if unbracketed.is_some() => Some(BlockReason::InvalidUrl),
        Err(_) => blocked_hostname(host).then_some(BlockReason::Hostname),
    };
    match reason {
        Some(reason) => Err(EgressBlocked::new(reason)),
        None => Ok(()),
    }
}

/// 丢掉被拒绝的地址，端口改为 0，让请求 URL 的端口生效。
/// 全部被拒绝或结果为空时失败关闭。
fn retain_allowed(addrs: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, EgressBlocked> {
    let mut allowed = Vec::new();
    let mut blocked = 0u32;
    let mut first_reason = None;
    for addr in addrs {
        match classify_ip(addr.ip()) {
            Some(reason) => {
                blocked += 1;
                if first_reason.is_none() {
                    first_reason = Some(reason);
                }
            }
            None => allowed.push(SocketAddr::new(addr.ip(), 0)),
        }
    }
    if allowed.is_empty() {
        let blocked_error = EgressBlocked::new(first_reason.unwrap_or(BlockReason::Empty));
        tracing::debug!(
            blocked,
            allowed = 0,
            reason = blocked_error.reason_name(),
            "egress dns result blocked"
        );
        return Err(blocked_error);
    }
    if blocked > 0 {
        tracing::debug!(
            blocked,
            allowed = allowed.len(),
            "egress dns result filtered"
        );
    }
    Ok(allowed)
}

#[derive(Debug, Default)]
struct PinningResolver;

impl Resolve for PinningResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let _ = self;
        let host = name.as_str().to_owned();
        Box::pin(async move { resolve_host(&host).await })
    }
}

async fn resolve_host(host: &str) -> Result<Addrs, Box<dyn StdError + Send + Sync>> {
    if blocked_hostname(host) {
        let blocked = EgressBlocked::new(BlockReason::Hostname);
        tracing::debug!(reason = blocked.reason_name(), "egress dns name blocked");
        return Err(Box::new(blocked));
    }
    let looked_up = match tokio::net::lookup_host((host, 0)).await {
        Ok(addrs) => addrs.collect::<Vec<_>>(),
        Err(_) => {
            let blocked = EgressBlocked::new(BlockReason::LookupFailed);
            tracing::debug!(reason = blocked.reason_name(), "egress dns lookup failed");
            return Err(Box::new(blocked));
        }
    };
    match retain_allowed(looked_up) {
        Ok(addrs) => Ok(Box::new(addrs.into_iter()) as Addrs),
        Err(blocked) => Err(Box::new(blocked)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RedirectChoice {
    Follow,
    Blocked(EgressBlocked),
    Stopped,
}

/// 重定向决策。跳数与环路用 [`RedirectStopped`]（不重试地址策略）；
/// 下一跳地址不合法用 [`EgressBlocked`]。
fn decide_redirect(next: &str, previous: &[&str]) -> RedirectChoice {
    if previous.len() > MAX_REDIRECTS || previous.contains(&next) {
        return RedirectChoice::Stopped;
    }
    match check_url(next, true) {
        Ok(()) => RedirectChoice::Follow,
        Err(blocked) => RedirectChoice::Blocked(blocked),
    }
}

#[derive(Debug)]
struct RedirectStopped;

impl fmt::Display for RedirectStopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("redirect was not followed")
    }
}

impl StdError for RedirectStopped {}

fn on_redirect(attempt: Attempt<'_>) -> redirect::Action {
    let previous: Vec<&str> = attempt
        .previous()
        .iter()
        .map(reqwest::Url::as_str)
        .collect();
    match decide_redirect(attempt.url().as_str(), &previous) {
        RedirectChoice::Follow => attempt.follow(),
        RedirectChoice::Blocked(blocked) => {
            tracing::debug!(reason = blocked.reason_name(), "egress redirect rejected");
            attempt.error(blocked)
        }
        RedirectChoice::Stopped => {
            tracing::debug!(reason = "redirect_stopped", "egress redirect rejected");
            attempt.error(RedirectStopped)
        }
    }
}

/// 受控 reqwest client 的重定向策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RedirectMode {
    /// 逐跳校验，最多 10 跳。
    Follow,
    /// 不跟随重定向（MCP 传输与 OAuth token 刷新）。
    None,
}

/// 带地址过滤解析器的 client。`pool_max_idle_per_host` / `timeout` 为 `None` 时沿用 reqwest 缺省。
pub(crate) fn guarded_client(
    redirect: RedirectMode,
    pool_max_idle_per_host: Option<usize>,
    timeout: Option<Duration>,
) -> Result<reqwest::Client, reqwest::Error> {
    let policy = match redirect {
        RedirectMode::Follow => redirect::Policy::custom(on_redirect),
        RedirectMode::None => redirect::Policy::none(),
    };
    let mut builder = reqwest::Client::builder()
        .redirect(policy)
        .dns_resolver(PinningResolver);
    if let Some(idle) = pool_max_idle_per_host {
        builder = builder.pool_max_idle_per_host(idle);
    }
    if let Some(limit) = timeout {
        builder = builder.timeout(limit);
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use std::error::Error as StdError;
    use std::fmt;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    use super::{
        BlockReason, EgressBlocked, RedirectChoice, SAFE_MESSAGE, check_url, classify_ip,
        decide_redirect, find_blocked, resolve_host, retain_allowed,
    };

    fn reason_of(url: &str) -> Result<(), BlockReason> {
        check_url(url, true).map_err(|blocked| blocked.reason)
    }

    fn ipv4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn ipv4_classes() {
        assert_eq!(
            classify_ip(ipv4(0, 0, 0, 0)),
            Some(BlockReason::Unspecified)
        );
        assert_eq!(
            classify_ip(ipv4(0, 1, 2, 3)),
            Some(BlockReason::Unspecified)
        );
        assert_eq!(classify_ip(ipv4(127, 0, 0, 1)), Some(BlockReason::Loopback));
        assert_eq!(
            classify_ip(ipv4(127, 255, 255, 255)),
            Some(BlockReason::Loopback)
        );
        assert_eq!(classify_ip(ipv4(10, 1, 2, 3)), Some(BlockReason::Private));
        assert_eq!(classify_ip(ipv4(172, 16, 0, 1)), Some(BlockReason::Private));
        assert_eq!(
            classify_ip(ipv4(172, 31, 255, 255)),
            Some(BlockReason::Private)
        );
        assert_eq!(
            classify_ip(ipv4(192, 168, 1, 1)),
            Some(BlockReason::Private)
        );
        assert_eq!(
            classify_ip(ipv4(169, 254, 169, 254)),
            Some(BlockReason::LinkLocal)
        );
        assert_eq!(
            classify_ip(ipv4(169, 254, 170, 2)),
            Some(BlockReason::LinkLocal)
        );
        assert_eq!(classify_ip(ipv4(100, 64, 0, 1)), Some(BlockReason::Cgnat));
        assert_eq!(
            classify_ip(ipv4(100, 100, 100, 200)),
            Some(BlockReason::Cgnat)
        );
        assert_eq!(
            classify_ip(ipv4(100, 127, 255, 255)),
            Some(BlockReason::Cgnat)
        );
        assert_eq!(
            classify_ip(ipv4(224, 0, 0, 1)),
            Some(BlockReason::Multicast)
        );
        assert_eq!(
            classify_ip(ipv4(239, 255, 255, 255)),
            Some(BlockReason::Multicast)
        );
        assert_eq!(classify_ip(ipv4(240, 0, 0, 1)), Some(BlockReason::Reserved));
        assert_eq!(
            classify_ip(ipv4(255, 255, 255, 255)),
            Some(BlockReason::Reserved)
        );
        assert_eq!(
            classify_ip(ipv4(168, 63, 129, 16)),
            Some(BlockReason::Metadata)
        );

        assert_eq!(classify_ip(ipv4(1, 1, 1, 1)), None);
        assert_eq!(classify_ip(ipv4(8, 8, 8, 8)), None);
        assert_eq!(classify_ip(ipv4(172, 15, 255, 255)), None);
        assert_eq!(classify_ip(ipv4(172, 32, 0, 1)), None);
        assert_eq!(classify_ip(ipv4(192, 169, 1, 1)), None);
        assert_eq!(classify_ip(ipv4(100, 63, 255, 255)), None);
        assert_eq!(classify_ip(ipv4(100, 128, 0, 1)), None);
        assert_eq!(classify_ip(ipv4(198, 18, 0, 1)), None);
    }

    #[test]
    fn ipv6_and_embedded_classes() {
        let parse = |text: &str| IpAddr::V6(text.parse().expect("ipv6"));
        assert_eq!(classify_ip(parse("::")), Some(BlockReason::Unspecified));
        assert_eq!(classify_ip(parse("::1")), Some(BlockReason::Loopback));
        assert_eq!(classify_ip(parse("fe80::1")), Some(BlockReason::LinkLocal));
        assert_eq!(
            classify_ip(parse("fc00::1")),
            Some(BlockReason::UniqueLocal)
        );
        assert_eq!(
            classify_ip(parse("fd00:ec2::254")),
            Some(BlockReason::UniqueLocal)
        );
        assert_eq!(
            classify_ip(parse("fec0::1")),
            Some(BlockReason::UniqueLocal)
        );
        assert_eq!(classify_ip(parse("ff02::1")), Some(BlockReason::Multicast));
        assert_eq!(
            classify_ip(parse("::ffff:10.0.0.1")),
            Some(BlockReason::Private)
        );
        assert_eq!(
            classify_ip(parse("::ffff:169.254.169.254")),
            Some(BlockReason::LinkLocal)
        );
        assert_eq!(
            classify_ip(parse("::ffff:127.0.0.1")),
            Some(BlockReason::Loopback)
        );
        assert_eq!(classify_ip(parse("::10.0.0.1")), Some(BlockReason::Private));
        assert_eq!(
            classify_ip(parse("64:ff9b::10.0.0.1")),
            Some(BlockReason::Private)
        );
        assert_eq!(
            classify_ip(parse("2002:a00:1::")),
            Some(BlockReason::Private)
        );

        assert_eq!(classify_ip(parse("::ffff:1.1.1.1")), None);
        assert_eq!(classify_ip(parse("::8.8.8.8")), None);
        assert_eq!(classify_ip(parse("64:ff9b::8.8.8.8")), None);
        assert_eq!(classify_ip(parse("2002:808:808::")), None);
        assert_eq!(classify_ip(parse("2001:db8::1")), None);
    }

    #[test]
    fn urls_cover_literals_names_and_schemes() {
        assert_eq!(reason_of("http://127.0.0.1/"), Err(BlockReason::Loopback));
        assert_eq!(reason_of("http://127.1/"), Err(BlockReason::Loopback));
        assert_eq!(reason_of("http://[::1]/"), Err(BlockReason::Loopback));
        assert_eq!(
            reason_of("http://[::ffff:10.0.0.1]/"),
            Err(BlockReason::Private)
        );
        assert_eq!(reason_of("http://10.0.0.1/"), Err(BlockReason::Private));
        assert_eq!(
            reason_of("http://169.254.169.254/"),
            Err(BlockReason::LinkLocal)
        );
        assert_eq!(
            reason_of("http://168.63.129.16/"),
            Err(BlockReason::Metadata)
        );
        assert_eq!(reason_of("http://localhost/"), Err(BlockReason::Hostname));
        assert_eq!(reason_of("http://localhost./"), Err(BlockReason::Hostname));
        assert_eq!(reason_of("http://LOCALHOST/"), Err(BlockReason::Hostname));
        assert_eq!(
            reason_of("http://foo.localhost/x"),
            Err(BlockReason::Hostname)
        );
        assert_eq!(reason_of("http://metadata/"), Err(BlockReason::Hostname));
        assert_eq!(
            reason_of("http://metadata.google.internal./"),
            Err(BlockReason::Hostname)
        );
        assert_eq!(
            reason_of("https://metadata.azure.internal/path"),
            Err(BlockReason::Hostname)
        );
        assert_eq!(reason_of("ftp://1.1.1.1/"), Err(BlockReason::Scheme));
        assert_eq!(reason_of("http://"), Err(BlockReason::InvalidUrl));

        assert_eq!(reason_of("https://1.1.1.1/"), Ok(()));
        assert_eq!(reason_of("https://example.com/v1"), Ok(()));
        assert_eq!(reason_of("https://metadata.example.com/"), Ok(()));
        assert!(check_url("https://1.1.1.1/", false).is_err());
        assert!(check_url("https://example.com/", false).is_err());
    }

    #[test]
    fn display_has_no_address() {
        let err = check_url("http://10.0.0.1/secret", true).expect_err("blocked");
        assert_eq!(err.to_string(), SAFE_MESSAGE);
        assert!(!err.to_string().contains("10.0.0.1"));
        assert!(!format!("{err:?}").contains("10.0.0.1"));
        assert_eq!(err.reason_name(), "private");
    }

    #[test]
    fn find_blocked_walks_the_source_chain() {
        #[derive(Debug)]
        struct Wrap(EgressBlocked);
        impl fmt::Display for Wrap {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("wrapped")
            }
        }
        impl StdError for Wrap {
            fn source(&self) -> Option<&(dyn StdError + 'static)> {
                Some(&self.0)
            }
        }
        let err = Wrap(EgressBlocked::new(BlockReason::LinkLocal));
        assert_eq!(
            find_blocked(&err).map(|blocked| blocked.reason),
            Some(BlockReason::LinkLocal)
        );
    }

    #[test]
    fn retain_allowed_keeps_only_public_and_resets_port() {
        let mixed = vec![
            SocketAddr::from(([10, 0, 0, 1], 80)),
            SocketAddr::from(([1, 1, 1, 1], 443)),
            SocketAddr::from(([169, 254, 169, 254], 80)),
        ];
        let kept = retain_allowed(mixed).expect("public remains");
        assert_eq!(kept, vec![SocketAddr::from(([1, 1, 1, 1], 0))]);

        let blocked = vec![
            SocketAddr::from(([127, 0, 0, 1], 9)),
            SocketAddr::from((Ipv6Addr::LOCALHOST, 9)),
        ];
        assert!(retain_allowed(blocked).is_err());
        assert!(retain_allowed(Vec::new()).is_err());
    }

    #[test]
    fn redirect_decision_checks_hops_loops_and_the_next_url() {
        assert_eq!(
            decide_redirect("https://example.com/next", &["https://example.com/"]),
            RedirectChoice::Follow
        );
        assert!(matches!(
            decide_redirect("http://10.0.0.1/a", &["https://example.com/"]),
            RedirectChoice::Blocked(_)
        ));
        assert!(matches!(
            decide_redirect("http://169.254.169.254/", &["https://example.com/"]),
            RedirectChoice::Blocked(_)
        ));
        assert_eq!(
            decide_redirect("https://example.com/", &["https://example.com/"]),
            RedirectChoice::Stopped
        );
        let previous = ["https://example.com/"; 11];
        assert_eq!(
            decide_redirect("https://other.example/", &previous),
            RedirectChoice::Stopped
        );
        let ten = ["https://example.com/"; 10];
        assert_eq!(
            decide_redirect("https://1.1.1.1/", &ten),
            RedirectChoice::Follow
        );
    }

    #[tokio::test]
    async fn resolve_host_rejects_private_literals_and_denied_names() {
        let Err(private) = resolve_host("10.0.0.1").await else {
            panic!("private literal should be blocked");
        };
        assert_eq!(private.to_string(), SAFE_MESSAGE);
        assert!(!private.to_string().contains("10.0.0.1"));
        assert!(find_blocked(private.as_ref()).is_some());

        let Err(name) = resolve_host("metadata.google.internal").await else {
            panic!("metadata name should be blocked");
        };
        assert_eq!(name.to_string(), SAFE_MESSAGE);
        assert!(!name.to_string().contains("metadata"));

        let public = resolve_host("1.1.1.1").await.expect("public literal");
        let got: Vec<_> = public.collect();
        assert!(!got.is_empty());
        for addr in &got {
            assert_eq!(addr.port(), 0);
            assert!(classify_ip(addr.ip()).is_none());
        }
    }

    #[tokio::test]
    async fn localhost_lookup_is_blocked_when_it_resolves() {
        let Ok(iter) = tokio::net::lookup_host(("localhost", 0)).await else {
            return;
        };
        let addrs: Vec<_> = iter.collect();
        if addrs.is_empty() {
            return;
        }
        for addr in &addrs {
            assert!(classify_ip(addr.ip()).is_some());
        }
        assert!(retain_allowed(addrs).is_err());
    }
}
