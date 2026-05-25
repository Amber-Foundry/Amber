use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    ipc_types::Node, ipc_types::Vault, resolve_node_effective_privacy,
    resolve_vault_effective_privacy, DbState,
};

const DATA_SALT_KEY: &str = "auth_master_data_salt";
const REDACTED_LABEL: &str = "[REDACTED]";
const REDACTED_SUMMARY: &str = "[Metadata Locked]";

pub(crate) type SessionKey = [u8; 32];

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedEnvelope {
    v: u8,
    alg: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VaultSecretPayload {
    pub(crate) name: String,
    pub(crate) icon: Option<String>,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct NodeSecretPayload {
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) detail: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) source_type: Option<String>,
}

fn argon2id() -> Result<Argon2<'static>, String> {
    let params = Params::new(19_456, 2, 1, Some(32))
        .map_err(|err| format!("Invalid Argon2 params: {err}"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

fn generate_random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

pub(crate) fn set_session_key(state: &DbState, key: SessionKey) {
    if let Ok(mut guard) = state.redacted_session_key.lock() {
        *guard = Some(key);
    }
}

pub(crate) fn get_session_key(state: &DbState) -> Option<SessionKey> {
    state
        .redacted_session_key
        .lock()
        .ok()
        .and_then(|guard| *guard)
}

pub(crate) fn ensure_data_salt(conn: &Connection) -> Result<String, String> {
    let existing = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1 LIMIT 1;",
            [DATA_SALT_KEY],
            |row| row.get::<_, String>(0),
        )
        .ok();

    if let Some(value) = existing {
        let salt: String = serde_json::from_str(&value)
            .map_err(|err| format!("Failed parsing stored redacted data salt: {err}"))?;
        return Ok(salt);
    }

    let salt = STANDARD.encode(generate_random_bytes::<16>());
    let stored = serde_json::to_string(&salt)
        .map_err(|err| format!("Failed serializing data salt: {err}"))?;
    conn.execute(
        "INSERT INTO settings (key, value, scope, updated_at)
         VALUES (?1, ?2, 'global', datetime('now'))
         ON CONFLICT(key) DO UPDATE
         SET value = excluded.value,
             updated_at = datetime('now');",
        params![DATA_SALT_KEY, stored],
    )
    .map_err(|err| format!("Failed storing redacted data salt: {err}"))?;
    Ok(salt)
}

pub(crate) fn derive_session_key(passphrase: &str, salt_b64: &str) -> Result<SessionKey, String> {
    let salt = STANDARD
        .decode(salt_b64)
        .map_err(|err| format!("Failed decoding redacted data salt: {err}"))?;
    let mut output = [0_u8; 32];
    argon2id()?
        .hash_password_into(passphrase.as_bytes(), &salt, &mut output)
        .map_err(|err| format!("Failed deriving redacted session key: {err}"))?;
    Ok(output)
}

pub(crate) fn encrypt_json<T: Serialize>(payload: &T, key: &SessionKey) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|err| format!("Failed initializing AES-256-GCM cipher: {err}"))?;
    let nonce_bytes = generate_random_bytes::<12>();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext =
        serde_json::to_vec(payload).map_err(|err| format!("Failed serializing payload: {err}"))?;
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| "Failed encrypting redacted payload.".to_string())?;

    let envelope = EncryptedEnvelope {
        v: 1,
        alg: "aes-256-gcm".to_string(),
        nonce: STANDARD.encode(nonce_bytes),
        ciphertext: STANDARD.encode(ciphertext),
    };

    serde_json::to_string(&envelope)
        .map_err(|err| format!("Failed serializing encrypted payload envelope: {err}"))
}

pub(crate) fn decrypt_json<T: DeserializeOwned>(
    envelope_json: &str,
    key: &SessionKey,
) -> Result<T, String> {
    let envelope: EncryptedEnvelope = serde_json::from_str(envelope_json)
        .map_err(|err| format!("Failed parsing encrypted payload envelope: {err}"))?;
    if envelope.v != 1 || envelope.alg != "aes-256-gcm" {
        return Err("Unsupported encrypted payload format.".to_string());
    }

    let nonce_bytes = STANDARD
        .decode(envelope.nonce)
        .map_err(|err| format!("Failed decoding encrypted payload nonce: {err}"))?;
    if nonce_bytes.len() != 12 {
        return Err("Invalid nonce length for encrypted payload".to_string());
    }
    let ciphertext = STANDARD
        .decode(envelope.ciphertext)
        .map_err(|err| format!("Failed decoding encrypted payload ciphertext: {err}"))?;

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|err| format!("Failed initializing AES-256-GCM cipher: {err}"))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
        .map_err(|_| {
            "Failed decrypting redacted payload. The active master password may be wrong."
                .to_string()
        })?;

    serde_json::from_slice(&plaintext)
        .map_err(|err| format!("Failed decoding decrypted payload JSON: {err}"))
}

pub(crate) fn apply_locked_vault_placeholder(vault: &mut Vault) {
    vault.name = REDACTED_LABEL.to_string();
    vault.icon = None;
    vault.description = Some(REDACTED_SUMMARY.to_string());
}

pub(crate) fn apply_locked_node_placeholder(node: &mut Node) {
    node.title = REDACTED_LABEL.to_string();
    node.summary = REDACTED_SUMMARY.to_string();
    node.detail = None;
    node.source = None;
    node.source_type = None;
}

pub(crate) fn migrate_legacy_redacted_records(
    conn: &Connection,
    key: &SessionKey,
) -> Result<(), String> {
    migrate_legacy_redacted_vaults(conn, key, false)?;
    migrate_legacy_redacted_vaults(conn, key, true)?;
    migrate_legacy_redacted_nodes(conn, key)?;
    Ok(())
}

fn migrate_legacy_redacted_vaults(
    conn: &Connection,
    key: &SessionKey,
    is_subvault: bool,
) -> Result<(), String> {
    let sql = if is_subvault {
        "SELECT id, name, icon, description
         FROM sub_vaults
         WHERE deleted_at IS NULL
           AND (encrypted_payload IS NULL OR encrypted_payload = '');"
    } else {
        "SELECT id, name, icon, description
         FROM vaults
         WHERE deleted_at IS NULL
           AND (encrypted_payload IS NULL OR encrypted_payload = '');"
    };

    let mut statement = conn
        .prepare(sql)
        .map_err(|err| format!("Failed preparing redacted vault migration query: {err}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|err| format!("Failed reading legacy redacted vaults: {err}"))?;

    for row in rows {
        let (id, name, icon, description) =
            row.map_err(|err| format!("Failed decoding legacy redacted vault row: {err}"))?;
        if resolve_vault_effective_privacy(conn, &id)? != "redacted" {
            continue;
        }

        let payload = VaultSecretPayload {
            name,
            icon,
            description,
        };
        let encrypted = encrypt_json(&payload, key)?;
        let update_sql = if is_subvault {
            "UPDATE sub_vaults
             SET name = ?2,
                 icon = NULL,
                 description = ?3,
                 encrypted_payload = ?4,
                 updated_at = datetime('now')
             WHERE id = ?1;"
        } else {
            "UPDATE vaults
             SET name = ?2,
                 icon = NULL,
                 description = ?3,
                 encrypted_payload = ?4,
                 updated_at = datetime('now')
             WHERE id = ?1;"
        };

        conn.execute(
            update_sql,
            params![id, REDACTED_LABEL, REDACTED_SUMMARY, encrypted],
        )
        .map_err(|err| format!("Failed migrating legacy redacted vault {id}: {err}"))?;
    }

    Ok(())
}

fn migrate_legacy_redacted_nodes(conn: &Connection, key: &SessionKey) -> Result<(), String> {
    let mut statement = conn
        .prepare(
            "SELECT id, vault_id, sub_vault_id, title, summary, detail, source, source_type, privacy_tier
             FROM nodes
             WHERE deleted_at IS NULL
               AND (encrypted_payload IS NULL OR encrypted_payload = '');",
        )
        .map_err(|err| format!("Failed preparing redacted node migration query: {err}"))?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(|err| format!("Failed reading legacy redacted nodes: {err}"))?;

    for row in rows {
        let (id, vault_id, sub_vault_id, title, summary, detail, source, source_type, privacy_tier) =
            row.map_err(|err| format!("Failed decoding legacy redacted node row: {err}"))?;
        if resolve_node_effective_privacy(
            conn,
            &vault_id,
            sub_vault_id.as_deref(),
            privacy_tier.as_deref(),
        )? != "redacted"
        {
            continue;
        }

        let payload = NodeSecretPayload {
            title,
            summary,
            detail,
            source,
            source_type,
        };
        let encrypted = encrypt_json(&payload, key)?;
        conn.execute(
            "UPDATE nodes
             SET title = ?2,
                 summary = ?3,
                 detail = NULL,
                 source = NULL,
                 source_type = NULL,
                 encrypted_payload = ?4,
                 updated_at = datetime('now')
             WHERE id = ?1;",
            params![id, REDACTED_LABEL, REDACTED_SUMMARY, encrypted],
        )
        .map_err(|err| format!("Failed migrating legacy redacted node {id}: {err}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{decrypt_json, derive_session_key, encrypt_json, NodeSecretPayload};

    #[test]
    fn derived_session_key_is_stable_for_same_input() {
        let salt = "ZGV0ZXJtaW5pc3RpYy1zYWx0";
        let first = match derive_session_key("test-passphrase", salt) {
            Ok(value) => value,
            Err(err) => panic!("session key derivation should succeed: {err}"),
        };
        let second = match derive_session_key("test-passphrase", salt) {
            Ok(value) => value,
            Err(err) => panic!("session key derivation should succeed: {err}"),
        };
        assert_eq!(first, second);
    }

    #[test]
    fn encrypted_payload_round_trips() {
        let key = match derive_session_key("correct horse battery staple", "cmVkYWN0ZWQtc2FsdA==") {
            Ok(value) => value,
            Err(err) => panic!("session key derivation should succeed: {err}"),
        };
        let payload = NodeSecretPayload {
            title: "Top Secret".to_string(),
            summary: "Highly restricted".to_string(),
            detail: Some("Encrypted at rest".to_string()),
            source: Some("manual".to_string()),
            source_type: Some("manual".to_string()),
        };

        let ciphertext = match encrypt_json(&payload, &key) {
            Ok(value) => value,
            Err(err) => panic!("encryption should succeed: {err}"),
        };
        let decrypted: NodeSecretPayload = match decrypt_json(&ciphertext, &key) {
            Ok(value) => value,
            Err(err) => panic!("decryption should succeed: {err}"),
        };

        assert_eq!(decrypted.title, payload.title);
        assert_eq!(decrypted.summary, payload.summary);
        assert_eq!(decrypted.detail, payload.detail);
        assert_eq!(decrypted.source, payload.source);
        assert_eq!(decrypted.source_type, payload.source_type);
    }
}
