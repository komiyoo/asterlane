//! 凭据加密：落库的上游 OAuth token 用 ChaCha20-Poly1305 加密。
//!
//! 存储格式为 `base64(nonce ‖ 密文 ‖ tag)`：每次加密用 `ring::rand::SystemRandom`
//! 生成新的 12 字节 nonce；AAD 绑定调用方给的标识（上游 server id），
//! 因此一行密文不能被搬到另一个 server 名下解密。密钥来自配置
//! `oauth.token_encryption_key_ref`，必须是 base64 编码的 32 字节。

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use thiserror::Error;
use zeroize::Zeroizing;

/// ChaCha20-Poly1305 密钥长度。
const KEY_LEN: usize = 32;
/// Poly1305 认证标签长度。
const TAG_LEN: usize = 16;

/// 加解密错误。`Display` 不含密钥、明文或密文内容。
#[derive(Debug, Error)]
pub enum SealError {
    /// 密钥配置无效（不是 base64 或解码后不是 32 字节）。
    #[error("token encryption key must be base64 encoded 32 bytes")]
    InvalidKey,
    /// 加密失败（系统随机数不可用等）。
    #[error("failed to encrypt credentials")]
    Encrypt,
    /// 解密失败：密文损坏、AAD 不匹配或密钥不对。不区分原因，避免成为 oracle。
    #[error("failed to decrypt credentials")]
    Decrypt,
}

/// token 加密密钥。手写 `Debug`，不输出任何密钥内容。
pub struct TokenEncryptionKey {
    key: LessSafeKey,
    rng: SystemRandom,
}

impl std::fmt::Debug for TokenEncryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenEncryptionKey(<redacted>)")
    }
}

impl TokenEncryptionKey {
    /// 从 base64 文本构造；首尾空白（例如文件末尾换行）会被忽略。
    pub fn from_base64(encoded: &str) -> Result<Self, SealError> {
        let bytes = Zeroizing::new(
            STANDARD
                .decode(encoded.trim())
                .map_err(|_| SealError::InvalidKey)?,
        );
        if bytes.len() != KEY_LEN {
            return Err(SealError::InvalidKey);
        }
        let unbound =
            UnboundKey::new(&CHACHA20_POLY1305, &bytes).map_err(|_| SealError::InvalidKey)?;
        Ok(Self {
            key: LessSafeKey::new(unbound),
            rng: SystemRandom::new(),
        })
    }

    /// 加密 `plaintext`，`aad` 绑定到密文（解密时必须给相同值）。
    pub fn encrypt(&self, aad: &str, plaintext: &[u8]) -> Result<String, SealError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        self.rng
            .fill(&mut nonce_bytes)
            .map_err(|_| SealError::Encrypt)?;
        let mut in_out = Zeroizing::new(plaintext.to_vec());
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(aad.as_bytes()),
                &mut *in_out,
            )
            .map_err(|_| SealError::Encrypt)?;
        let mut sealed = Vec::with_capacity(NONCE_LEN + in_out.len());
        sealed.extend_from_slice(&nonce_bytes);
        sealed.extend_from_slice(&in_out);
        Ok(STANDARD.encode(sealed))
    }

    /// 解密 [`Self::encrypt`] 的输出；明文用 `Zeroizing` 包裹，释放时清零。
    pub fn decrypt(&self, aad: &str, sealed: &str) -> Result<Zeroizing<Vec<u8>>, SealError> {
        let raw = STANDARD
            .decode(sealed.trim())
            .map_err(|_| SealError::Decrypt)?;
        if raw.len() < NONCE_LEN + TAG_LEN {
            return Err(SealError::Decrypt);
        }
        let (nonce_bytes, ciphertext) = raw.split_at(NONCE_LEN);
        let nonce =
            Nonce::try_assume_unique_for_key(nonce_bytes).map_err(|_| SealError::Decrypt)?;
        let mut in_out = Zeroizing::new(ciphertext.to_vec());
        let plain_len = self
            .key
            .open_in_place(nonce, Aad::from(aad.as_bytes()), &mut in_out)
            .map_err(|_| SealError::Decrypt)?
            .len();
        in_out.truncate(plain_len);
        Ok(in_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_b64(byte: u8) -> String {
        STANDARD.encode([byte; KEY_LEN])
    }

    fn key(byte: u8) -> TokenEncryptionKey {
        TokenEncryptionKey::from_base64(&key_b64(byte)).unwrap()
    }

    #[test]
    fn round_trip_restores_plaintext() {
        let key = key(7);
        let sealed = key
            .encrypt("linear", b"{\"refresh_token\":\"r-1\"}")
            .unwrap();
        let plain = key.decrypt("linear", &sealed).unwrap();
        assert_eq!(&*plain, b"{\"refresh_token\":\"r-1\"}");
    }

    #[test]
    fn ciphertext_hides_plaintext_and_uses_a_fresh_nonce_each_time() {
        let key = key(7);
        let a = key.encrypt("linear", b"super-secret-token").unwrap();
        let b = key.encrypt("linear", b"super-secret-token").unwrap();
        assert_ne!(a, b, "same plaintext must not produce the same ciphertext");
        assert!(!a.contains("super-secret-token"));
        let raw = STANDARD.decode(&a).unwrap();
        assert_eq!(raw.len(), NONCE_LEN + "super-secret-token".len() + TAG_LEN);
        assert!(
            !raw.windows(18).any(|w| w == b"super-secret-token"),
            "plaintext must not appear in the stored bytes"
        );
    }

    #[test]
    fn different_aad_fails_to_decrypt() {
        let key = key(7);
        let sealed = key.encrypt("linear", b"token").unwrap();
        assert!(matches!(
            key.decrypt("notion", &sealed),
            Err(SealError::Decrypt)
        ));
    }

    #[test]
    fn wrong_key_tampering_and_truncation_fail_to_decrypt() {
        let sealed = key(7).encrypt("linear", b"token").unwrap();
        assert!(key(8).decrypt("linear", &sealed).is_err());

        let mut raw = STANDARD.decode(&sealed).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        assert!(key(7).decrypt("linear", &STANDARD.encode(&raw)).is_err());

        assert!(key(7).decrypt("linear", "AAAA").is_err());
        assert!(key(7).decrypt("linear", "not base64!").is_err());
        assert!(key(7).decrypt("linear", "").is_err());
    }

    #[test]
    fn key_must_be_base64_of_32_bytes() {
        assert!(TokenEncryptionKey::from_base64(&key_b64(1)).is_ok());
        // 文件 backend 常带结尾换行
        assert!(TokenEncryptionKey::from_base64(&format!("{}\n", key_b64(1))).is_ok());
        for bad in [
            STANDARD.encode([1u8; 31]),
            STANDARD.encode([1u8; 33]),
            "not base64!".to_string(),
            String::new(),
        ] {
            assert!(
                matches!(
                    TokenEncryptionKey::from_base64(&bad),
                    Err(SealError::InvalidKey)
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn debug_and_errors_never_show_key_material() {
        let encoded = key_b64(9);
        let key = TokenEncryptionKey::from_base64(&encoded).unwrap();
        assert!(!format!("{key:?}").contains(&encoded));
        let err = TokenEncryptionKey::from_base64("c2VjcmV0LWtleS1tYXRlcmlhbA==").unwrap_err();
        assert!(!err.to_string().contains("c2VjcmV0"));
    }
}
