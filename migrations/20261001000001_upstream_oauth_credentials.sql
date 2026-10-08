-- 上游 MCP OAuth 凭据（见 docs/runtime/key-credentials-and-persistence.md）。
-- sealed_credentials 是 base64(nonce ‖ ChaCha20-Poly1305 密文)，明文是 rmcp
-- StoredCredentials 的 JSON（含 access/refresh token），AAD 为 server_id；
-- 数据库里不出现任何明文 token。client_credentials 的 token 只放内存，不进此表。
CREATE TABLE IF NOT EXISTS upstream_oauth_credentials (
    server_id           TEXT PRIMARY KEY,
    sealed_credentials  TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);
