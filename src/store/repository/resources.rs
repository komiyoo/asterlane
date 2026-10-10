//! 资源、网关代理 key 与上游密钥的 DB 行映射和 repository。

use crate::store::error::StoreError;

/// 资源记录（DB 行映射）。
#[derive(Debug, Clone)]
pub struct Resource {
    pub id: String,
    pub domain: String,
    pub provider: String,
    pub base_url: String,
    pub description: Option<String>,
    pub config_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 网关代理 key 记录（DB 行映射）。
#[derive(Debug, Clone)]
pub struct ProxyKeyRecord {
    pub id: String,
    pub display_name: String,
    pub default_tool_page_size: i64,
    pub scope_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 上游密钥记录（DB 行映射）。
#[derive(Debug, Clone)]
pub struct UpstreamKeyRecord {
    pub id: String,
    pub resource_id: String,
    pub secret_ref: String,
    pub weight: i64,
    pub health_state: String,
    pub cooldown_until: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 资源 repository trait。
pub trait ResourceRepository: Send + Sync {
    /// 插入一条资源（created_at/updated_at 由 DB 默认值填充）。
    fn insert_resource(
        &self,
        resource: &Resource,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 按 ID 获取资源。
    fn get_resource(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<Resource, StoreError>> + Send;

    /// 列出所有资源。
    fn list_resources(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<Resource>, StoreError>> + Send;

    /// 更新资源，返回是否有行被更新。
    fn update_resource(
        &self,
        resource: &Resource,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    /// 删除资源，返回是否有行被删除。
    fn delete_resource(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;
}

impl ResourceRepository for () {
    async fn insert_resource(&self, _resource: &Resource) -> Result<(), StoreError> {
        Ok(())
    }
    async fn get_resource(&self, id: &str) -> Result<Resource, StoreError> {
        Err(StoreError::NotFound(format!("resource {id}")))
    }
    async fn list_resources(&self) -> Result<Vec<Resource>, StoreError> {
        Ok(Vec::new())
    }
    async fn update_resource(&self, _resource: &Resource) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn delete_resource(&self, _id: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
}

/// 网关代理 key repository trait。
pub trait ProxyKeyRepository: Send + Sync {
    fn insert_proxy_key(
        &self,
        key: &ProxyKeyRecord,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    fn get_proxy_key(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<ProxyKeyRecord, StoreError>> + Send;

    fn list_proxy_keys(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<ProxyKeyRecord>, StoreError>> + Send;

    fn update_proxy_key(
        &self,
        key: &ProxyKeyRecord,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    fn delete_proxy_key(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;
}

impl ProxyKeyRepository for () {
    async fn insert_proxy_key(&self, _key: &ProxyKeyRecord) -> Result<(), StoreError> {
        Ok(())
    }
    async fn get_proxy_key(&self, id: &str) -> Result<ProxyKeyRecord, StoreError> {
        Err(StoreError::NotFound(format!("proxy_key {id}")))
    }
    async fn list_proxy_keys(&self) -> Result<Vec<ProxyKeyRecord>, StoreError> {
        Ok(Vec::new())
    }
    async fn update_proxy_key(&self, _key: &ProxyKeyRecord) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn delete_proxy_key(&self, _id: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
}

/// 上游密钥 repository trait。
pub trait UpstreamKeyRepository: Send + Sync {
    fn insert_upstream_key(
        &self,
        key: &UpstreamKeyRecord,
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    fn get_upstream_key(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<UpstreamKeyRecord, StoreError>> + Send;

    /// 列出某个 resource 下的所有上游密钥。
    fn list_upstream_keys_for_resource(
        &self,
        resource_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<UpstreamKeyRecord>, StoreError>> + Send;

    /// 更新健康状态与冷却时间，返回是否有行被更新。
    fn update_upstream_key_health(
        &self,
        id: &str,
        health_state: &str,
        cooldown_until: Option<&str>,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    fn delete_upstream_key(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;

    /// 按 resource 整表替换：先删该 resource 的行，再插入 `keys`。
    fn replace_upstream_keys_for_resource(
        &self,
        resource_id: &str,
        keys: &[UpstreamKeyRecord],
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;

    /// 删除某个 resource 下的全部上游密钥，返回删除行数。
    fn delete_upstream_keys_for_resource(
        &self,
        resource_id: &str,
    ) -> impl std::future::Future<Output = Result<u64, StoreError>> + Send;
}

impl UpstreamKeyRepository for () {
    async fn insert_upstream_key(&self, _key: &UpstreamKeyRecord) -> Result<(), StoreError> {
        Ok(())
    }
    async fn get_upstream_key(&self, id: &str) -> Result<UpstreamKeyRecord, StoreError> {
        Err(StoreError::NotFound(format!("upstream_key {id}")))
    }
    async fn list_upstream_keys_for_resource(
        &self,
        _resource_id: &str,
    ) -> Result<Vec<UpstreamKeyRecord>, StoreError> {
        Ok(Vec::new())
    }
    async fn update_upstream_key_health(
        &self,
        _id: &str,
        _health_state: &str,
        _cooldown_until: Option<&str>,
    ) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn delete_upstream_key(&self, _id: &str) -> Result<bool, StoreError> {
        Ok(false)
    }
    async fn replace_upstream_keys_for_resource(
        &self,
        _resource_id: &str,
        _keys: &[UpstreamKeyRecord],
    ) -> Result<(), StoreError> {
        Ok(())
    }
    async fn delete_upstream_keys_for_resource(
        &self,
        _resource_id: &str,
    ) -> Result<u64, StoreError> {
        Ok(0)
    }
}
