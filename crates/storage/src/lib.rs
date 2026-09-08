//! Database, hashing, and application-secret primitives.

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use anyhow::{Context, Result, bail};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;
use vpn_domain::SubscriptionProfile;

const NONCE_LENGTH: usize = 12;
const TOKEN_BYTES: usize = 32;

/// Opens a `PostgreSQL` connection pool.
///
/// # Errors
///
/// Returns the `SQLx` connection error when the URL is invalid or the database is unavailable.
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
}

pub async fn is_ready(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
        .is_ok()
}

/// Loads and decrypts one application secret without exposing ciphertext.
///
/// # Errors
///
/// Returns an error when the query fails or authenticated decryption fails.
pub async fn load_app_secret(
    pool: &PgPool,
    key: &[u8; 32],
    secret_name: &str,
) -> Result<Option<String>> {
    let encrypted =
        sqlx::query_scalar::<_, Vec<u8>>("SELECT value FROM app_secrets WHERE key = $1")
            .bind(secret_name)
            .fetch_optional(pool)
            .await?;
    encrypted
        .map(|value| decrypt_app_secret(key, &value))
        .transpose()
}

/// Stores a normalized provider snapshot encrypted at rest.
///
/// # Errors
///
/// Returns an error when serialization, encryption, or the database write fails.
pub async fn store_provider_snapshot(
    pool: &PgPool,
    key: &[u8; 32],
    subscription_id: Uuid,
    profile: &SubscriptionProfile,
    provider_revision: Option<&str>,
    fresh_until: DateTime<Utc>,
) -> Result<()> {
    let serialized = serde_json::to_string(profile)?;
    let encrypted = encrypt_app_secret(key, &serialized)?;
    sqlx::query(
        "INSERT INTO provider_snapshots
         (id, subscription_id, profile, provider_revision, fetched_at, fresh_until)
         VALUES ($1, $2, $3, $4, now(), $5)
         ON CONFLICT (subscription_id) DO UPDATE SET profile = EXCLUDED.profile,
           provider_revision = EXCLUDED.provider_revision, fetched_at = now(),
           fresh_until = EXCLUDED.fresh_until",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(encrypted)
    .bind(provider_revision)
    .bind(fresh_until)
    .execute(pool)
    .await?;
    Ok(())
}

/// Decodes the base64-encoded 32-byte application encryption key.
///
/// # Errors
///
/// Returns an error when the value is not base64 or is not exactly 32 bytes.
pub fn decode_encryption_key(encoded: &str) -> Result<[u8; 32]> {
    let bytes = STANDARD
        .decode(encoded)
        .context("APPLICATION_ENCRYPTION_KEY must be base64")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("APPLICATION_ENCRYPTION_KEY must decode to 32 bytes"))
}

/// Encrypts a credential with authenticated AES-256-GCM.
///
/// # Errors
///
/// Returns an error when authenticated encryption fails.
///
/// # Panics
///
/// This cannot panic for a 32-byte key; the key length is part of the type.
pub fn encrypt_app_secret(key: &[u8; 32], plaintext: &str) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("AES-256 key has a fixed length");
    let mut nonce = [0_u8; NONCE_LENGTH];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| anyhow::anyhow!("secret encryption failed"))?;
    let mut result = Vec::with_capacity(NONCE_LENGTH + ciphertext.len());
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypts and validates an application credential.
///
/// # Errors
///
/// Returns an error when the ciphertext is truncated, authentication fails, or the plaintext is not UTF-8.
///
/// # Panics
///
/// This cannot panic for a 32-byte key; the key length is part of the type.
pub fn decrypt_app_secret(key: &[u8; 32], encrypted: &[u8]) -> Result<String> {
    let (nonce, ciphertext) = encrypted
        .split_at_checked(NONCE_LENGTH)
        .context("encrypted secret is truncated")?;
    let cipher = Aes256Gcm::new_from_slice(key).expect("AES-256 key has a fixed length");
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow::anyhow!("secret decryption failed"))?;
    String::from_utf8(plaintext).context("decrypted secret is not UTF-8")
}

#[must_use]
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

#[must_use]
pub fn generate_token() -> String {
    let mut bytes = [0_u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Hashes a password with Argon2id.
///
/// # Errors
///
/// Returns an error when the password is shorter than the minimum policy or hashing fails.
pub fn hash_password(password: &str) -> Result<String> {
    if password.chars().count() < 12 {
        bail!("password must contain at least 12 characters");
    }
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("password hashing failed: {error}"))?;
    Ok(hash.to_string())
}

#[must_use]
pub fn verify_password(password: &str, encoded_hash: &str) -> bool {
    PasswordHash::new(encoded_hash).is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        decrypt_app_secret, encrypt_app_secret, generate_token, hash_password, hash_token,
        verify_password,
    };

    #[test]
    fn encrypted_secret_round_trips() {
        let key = [7_u8; 32];
        let encrypted = encrypt_app_secret(&key, "telegram-secret").unwrap();
        assert_ne!(encrypted, b"telegram-secret");
        assert_eq!(
            decrypt_app_secret(&key, &encrypted).unwrap(),
            "telegram-secret"
        );
    }

    #[test]
    fn access_tokens_are_opaque_and_hashed() {
        let token = generate_token();
        assert!(token.len() >= 40);
        assert_ne!(hash_token(&token), token.as_bytes());
    }

    #[test]
    fn password_hash_is_not_reversible() {
        let hash = hash_password("a sufficiently long password").unwrap();
        assert!(verify_password("a sufficiently long password", &hash));
        assert!(!verify_password("wrong password", &hash));
    }
}
