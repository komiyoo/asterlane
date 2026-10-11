//! 出站地址校验开关。
//!
//! 见 docs/runtime/config-schema.md「Egress」与
//! docs/architecture/open-source-and-saas.md「出站请求」。

use serde::{Deserialize, Serialize};

/// 出站请求地址校验。缺省关闭，行为与未写这一节时相同。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressConfig {
    /// 为 true 时拒绝回环、私网、链路本地、CGNAT、未指定、组播、
    /// 保留地址与云元数据地址。开源版缺省 false。
    #[serde(default)]
    pub block_private_addresses: bool,
}

#[cfg(test)]
mod tests {
    use crate::config::GatewayConfig;

    #[test]
    fn missing_section_stays_off() {
        let config: GatewayConfig = serde_norway::from_str("api_resources: []").expect("yaml");
        assert!(!config.egress.block_private_addresses);
    }

    #[test]
    fn explicit_true_parses() {
        let config: GatewayConfig =
            serde_norway::from_str("egress:\n  block_private_addresses: true\napi_resources: []\n")
                .expect("yaml");
        assert!(config.egress.block_private_addresses);
    }
}
