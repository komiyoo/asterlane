//! SQLite 实现：`ResourceRepository`、`ProxyKeyRepository`、`UpstreamKeyRepository`。
//!
//! 从 `sqlite.rs` 拆出——实体 CRUD 语句独立于事件读写。

use crate::store::error::StoreError;
use crate::store::repository::{
    ProxyKeyRecord, ProxyKeyRepository, Resource, ResourceRepository, UpstreamKeyRecord,
    UpstreamKeyRepository,
};
use crate::store::sqlite::SqliteRequestEventRepository;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

// ── Resource ──

fn row_to_resource(row: SqliteRow) -> Result<Resource, StoreError> {
    Ok(Resource {
        id: row.try_get("id").map_err(StoreError::from)?,
        domain: row.try_get("domain").map_err(StoreError::from)?,
        provider: row.try_get("provider").map_err(StoreError::from)?,
        base_url: row.try_get("base_url").map_err(StoreError::from)?,
        description: row.try_get("description").map_err(StoreError::from)?,
        config_json: row.try_get("config_json").map_err(StoreError::from)?,
        created_at: row.try_get("created_at").map_err(StoreError::from)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::from)?,
    })
}

impl ResourceRepository for SqliteRequestEventRepository {
    async fn insert_resource(&self, resource: &Resource) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO resources (id, domain, provider, base_url, description, config_json)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&resource.id)
        .bind(&resource.domain)
        .bind(&resource.provider)
        .bind(&resource.base_url)
        .bind(&resource.description)
        .bind(&resource.config_json)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn get_resource(&self, id: &str) -> Result<Resource, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, domain, provider, base_url, description, config_json, created_at, updated_at
            FROM resources WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?
        .ok_or_else(|| StoreError::NotFound(format!("resource {id}")))?;
        row_to_resource(row)
    }

    async fn list_resources(&self) -> Result<Vec<Resource>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, domain, provider, base_url, description, config_json, created_at, updated_at
            FROM resources ORDER BY id
            "#,
        )
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.into_iter().map(row_to_resource).collect()
    }

    async fn update_resource(&self, resource: &Resource) -> Result<bool, StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE resources
            SET domain = ?, provider = ?, base_url = ?, description = ?, config_json = ?,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ?
            "#,
        )
        .bind(&resource.domain)
        .bind(&resource.provider)
        .bind(&resource.base_url)
        .bind(&resource.description)
        .bind(&resource.config_json)
        .bind(&resource.id)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_resource(&self, id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM resources WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }
}

// ── ProxyKey ──

fn row_to_proxy_key(row: SqliteRow) -> Result<ProxyKeyRecord, StoreError> {
    Ok(ProxyKeyRecord {
        id: row.try_get("id").map_err(StoreError::from)?,
        display_name: row.try_get("display_name").map_err(StoreError::from)?,
        default_tool_page_size: row
            .try_get("default_tool_page_size")
            .map_err(StoreError::from)?,
        scope_json: row.try_get("scope_json").map_err(StoreError::from)?,
        created_at: row.try_get("created_at").map_err(StoreError::from)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::from)?,
    })
}

impl ProxyKeyRepository for SqliteRequestEventRepository {
    async fn insert_proxy_key(&self, key: &ProxyKeyRecord) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO proxy_keys (id, display_name, default_tool_page_size, scope_json)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(&key.id)
        .bind(&key.display_name)
        .bind(key.default_tool_page_size)
        .bind(&key.scope_json)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn get_proxy_key(&self, id: &str) -> Result<ProxyKeyRecord, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, display_name, default_tool_page_size, scope_json, created_at, updated_at
            FROM proxy_keys WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?
        .ok_or_else(|| StoreError::NotFound(format!("proxy_key {id}")))?;
        row_to_proxy_key(row)
    }

    async fn list_proxy_keys(&self) -> Result<Vec<ProxyKeyRecord>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, display_name, default_tool_page_size, scope_json, created_at, updated_at
            FROM proxy_keys ORDER BY id
            "#,
        )
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.into_iter().map(row_to_proxy_key).collect()
    }

    async fn update_proxy_key(&self, key: &ProxyKeyRecord) -> Result<bool, StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE proxy_keys
            SET display_name = ?, default_tool_page_size = ?, scope_json = ?,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ?
            "#,
        )
        .bind(&key.display_name)
        .bind(key.default_tool_page_size)
        .bind(&key.scope_json)
        .bind(&key.id)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_proxy_key(&self, id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM proxy_keys WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }
}

// ── UpstreamKey ──

fn row_to_upstream_key(row: SqliteRow) -> Result<UpstreamKeyRecord, StoreError> {
    Ok(UpstreamKeyRecord {
        id: row.try_get("id").map_err(StoreError::from)?,
        resource_id: row.try_get("resource_id").map_err(StoreError::from)?,
        secret_ref: row.try_get("secret_ref").map_err(StoreError::from)?,
        weight: row.try_get("weight").map_err(StoreError::from)?,
        health_state: row.try_get("health_state").map_err(StoreError::from)?,
        cooldown_until: row.try_get("cooldown_until").map_err(StoreError::from)?,
        created_at: row.try_get("created_at").map_err(StoreError::from)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::from)?,
    })
}

impl UpstreamKeyRepository for SqliteRequestEventRepository {
    async fn insert_upstream_key(&self, key: &UpstreamKeyRecord) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO upstream_keys (id, resource_id, secret_ref, weight, health_state, cooldown_until)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&key.id)
        .bind(&key.resource_id)
        .bind(&key.secret_ref)
        .bind(key.weight)
        .bind(&key.health_state)
        .bind(&key.cooldown_until)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn get_upstream_key(&self, id: &str) -> Result<UpstreamKeyRecord, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, resource_id, secret_ref, weight, health_state, cooldown_until,
                   created_at, updated_at
            FROM upstream_keys WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?
        .ok_or_else(|| StoreError::NotFound(format!("upstream_key {id}")))?;
        row_to_upstream_key(row)
    }

    async fn list_upstream_keys_for_resource(
        &self,
        resource_id: &str,
    ) -> Result<Vec<UpstreamKeyRecord>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, resource_id, secret_ref, weight, health_state, cooldown_until,
                   created_at, updated_at
            FROM upstream_keys WHERE resource_id = ? ORDER BY id
            "#,
        )
        .bind(resource_id)
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.into_iter().map(row_to_upstream_key).collect()
    }

    async fn update_upstream_key_health(
        &self,
        id: &str,
        health_state: &str,
        cooldown_until: Option<&str>,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE upstream_keys
            SET health_state = ?, cooldown_until = ?,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ?
            "#,
        )
        .bind(health_state)
        .bind(cooldown_until)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_upstream_key(&self, id: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM upstream_keys WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(StoreError::from)?;
        Ok(result.rows_affected() > 0)
    }
}
