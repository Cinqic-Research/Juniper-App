use rusqlite::{Connection, OptionalExtension, Result, params, types::Type};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

pub const SCHEMA_VERSION: i64 = 4;

const INITIAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS assistants (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS provider_profiles (
    id TEXT PRIMARY KEY,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS model_profiles (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES provider_profiles(id),
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    assistant_id TEXT NOT NULL REFERENCES assistants(id),
    title TEXT NOT NULL,
    private_chat INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK(role IN ('system', 'user', 'assistant', 'tool')),
    created_at TEXT NOT NULL,
    model_id TEXT,
    provider_id TEXT
);
CREATE TABLE IF NOT EXISTS message_parts (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    part_type TEXT NOT NULL,
    text TEXT,
    payload TEXT
);
CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    assistant_id TEXT REFERENCES assistants(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    source TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    content_type TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    arguments TEXT NOT NULL,
    status TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tool_results (
    id TEXT PRIMARY KEY,
    tool_call_id TEXT NOT NULL REFERENCES tool_calls(id) ON DELETE CASCADE,
    result TEXT,
    error TEXT,
    created_at TEXT NOT NULL
);
INSERT OR IGNORE INTO schema_migrations(version, applied_at)
VALUES (1, datetime('now'));
"#;

const MIGRATION_V2: &str = r#"
CREATE TABLE IF NOT EXISTS app_settings (
    id TEXT PRIMARY KEY CHECK(id = 'singleton'),
    payload TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS app_state (
    id TEXT PRIMARY KEY CHECK(id = 'singleton'),
    payload TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

const MIGRATION_V3: &str = r#"
CREATE TABLE IF NOT EXISTS permissions (
    id TEXT PRIMARY KEY,
    tool_name TEXT NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('chat', 'assistant')),
    assistant_id TEXT NOT NULL REFERENCES assistants(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

const MIGRATION_V4: &str = r#"
CREATE TABLE IF NOT EXISTS device_link_identity (
    id TEXT PRIMARY KEY CHECK(id = 'singleton'),
    device_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS device_link_peers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    scopes_json TEXT NOT NULL,
    address TEXT,
    connected INTEGER NOT NULL DEFAULT 0,
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

fn migrate(connection: &Connection) -> Result<()> {
    debug_assert_eq!(SCHEMA_VERSION, 4);
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(INITIAL_SCHEMA)?;
    let version: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    if version > SCHEMA_VERSION {
        // Left untouched rather than migrated backwards; the message tells the
        // user why startup stopped.
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some(format!(
                "database schema v{version} was written by a newer Juniper; this build supports up to v{SCHEMA_VERSION} and left the file unchanged"
            )),
        ));
    }
    if version < 2 {
        let has_model_profile_id: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('conversations') WHERE name = 'model_profile_id')",
            [],
            |row| row.get(0),
        )?;
        if !has_model_profile_id {
            transaction.execute_batch(
                "ALTER TABLE conversations ADD COLUMN model_profile_id TEXT REFERENCES model_profiles(id);",
            )?;
        }
        transaction.execute_batch(MIGRATION_V2)?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(2, datetime('now'))",
            [],
        )?;
    }
    if version < 3 {
        transaction.execute_batch(MIGRATION_V3)?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(3, datetime('now'))",
            [],
        )?;
    }
    if version < 4 {
        transaction.execute_batch(MIGRATION_V4)?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(4, datetime('now'))",
            [],
        )?;
    }
    transaction.commit()
}

fn connection(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path)?;
    migrate(&connection)?;
    Ok(connection)
}

pub fn initialize(path: &Path) -> Result<()> {
    let _ = connection(path)?;
    Ok(())
}

pub fn load_app_data(path: &Path) -> Result<Option<Value>> {
    let connection = connection(path)?;
    let payload = connection
        .query_row(
            "SELECT payload FROM app_state WHERE id = 'singleton'",
            [],
            |row| {
                let payload: String = row.get(0)?;
                serde_json::from_str::<Value>(&payload).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
                })
            },
        )
        .optional()?;
    let Some(mut payload) = payload else {
        return Ok(None);
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "attachments".into(),
            Value::Array(load_attachment_metadata(&connection)?),
        );
        let peers = load_device_link_peers_from_connection(&connection)?;
        let device_link = object
            .entry("deviceLink")
            .or_insert_with(|| json!({"enabled": false, "hosting": false, "deviceId": ""}));
        if let Some(device_link) = device_link.as_object_mut() {
            device_link.insert(
                "peers".into(),
                Value::Array(
                    peers
                        .into_iter()
                        .map(|peer| serde_json::to_value(peer).unwrap_or(Value::Null))
                        .collect(),
                ),
            );
        }
    }
    Ok(Some(payload))
}

pub fn load_attachment_paths(path: &Path) -> Result<HashMap<String, PathBuf>> {
    let connection = connection(path)?;
    load_attachment_paths_from_connection(&connection)
}

fn load_attachment_paths_from_connection(
    connection: &Connection,
) -> Result<HashMap<String, PathBuf>> {
    let mut statement = connection.prepare("SELECT id, path FROM attachments WHERE path <> ''")?;
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
            ))
        })?
        .collect()
}

pub fn save_app_data_with_paths(
    path: &Path,
    data: &Value,
    attachment_paths: &HashMap<String, PathBuf>,
) -> Result<()> {
    let connection = connection(path)?;
    let mut known_attachment_paths = load_attachment_paths_from_connection(&connection)?;
    known_attachment_paths.extend(
        attachment_paths
            .iter()
            .map(|(id, path)| (id.clone(), path.clone())),
    );
    let mut persisted = data.clone();
    if let Some(object) = persisted.as_object_mut() {
        let private_conversation_ids = object
            .get("conversations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|conversation| conversation["privateChat"].as_bool().unwrap_or(false))
            .filter_map(|conversation| conversation["id"].as_str().map(str::to_owned))
            .collect::<std::collections::HashSet<_>>();
        if let Some(conversations) = object
            .get_mut("conversations")
            .and_then(Value::as_array_mut)
        {
            conversations
                .retain(|conversation| !conversation["privateChat"].as_bool().unwrap_or(false));
            // Raw model reasoning is never persisted, whatever the interface sends.
            for message in conversations
                .iter_mut()
                .filter_map(|conversation| conversation["messages"].as_array_mut())
                .flatten()
            {
                if let Some(parts) = message["parts"].as_array_mut() {
                    parts.retain(|part| part["type"] != "reasoning");
                }
            }
        }
        if let Some(attachments) = object.get_mut("attachments").and_then(Value::as_array_mut) {
            attachments.retain(|attachment| {
                !private_conversation_ids
                    .contains(attachment["conversationId"].as_str().unwrap_or_default())
            });
        }
        if let Some(permissions) = object.get_mut("permissions").and_then(Value::as_array_mut) {
            permissions.retain(|permission| {
                permission["scope"] != "chat"
                    || !private_conversation_ids
                        .contains(permission["conversationId"].as_str().unwrap_or_default())
            });
        }
    }
    let payload = serde_json::to_string(&persisted).unwrap_or_else(|_| "{}".into());
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO app_state(id, payload, updated_at) VALUES('singleton', ?1, datetime('now')) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload, updated_at=excluded.updated_at",
        params![payload],
    )?;
    let settings = persisted.get("settings").cloned().unwrap_or(Value::Null);
    transaction.execute(
        "INSERT INTO app_settings(id, payload, updated_at) VALUES('singleton', ?1, datetime('now')) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload, updated_at=excluded.updated_at",
        params![serde_json::to_string(&settings).unwrap_or_else(|_| "{}".into())],
    )?;
    for table in [
        "permissions",
        "tool_results",
        "tool_calls",
        "message_parts",
        "messages",
        "attachments",
        "conversations",
        "memories",
        "model_profiles",
        "provider_profiles",
        "assistants",
    ] {
        transaction.execute(&format!("DELETE FROM {table}"), [])?;
    }
    // `app_state` holds the authoritative snapshot and is what `load_app_data`
    // returns. The relational tables below are a projection of it, so a
    // reference the interface legitimately leaves dangling (a model whose
    // provider was removed, a chat pinned to a removed model) is dropped from
    // the projection instead of failing the foreign key and, with it, every
    // later save.
    let mut assistant_ids = HashSet::new();
    let mut provider_ids = HashSet::new();
    let mut model_ids = HashSet::new();
    let mut conversation_ids = HashSet::new();
    for item in persisted
        .get("assistants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = item["id"].as_str().unwrap_or_default();
        if id.is_empty() || !assistant_ids.insert(id.to_owned()) {
            continue;
        }
        transaction.execute("INSERT INTO assistants(id, schema_version, payload, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5)", params![id, item["schemaVersion"].as_i64().unwrap_or(2), item.to_string(), item["createdAt"].as_str().unwrap_or(""), item["updatedAt"].as_str().unwrap_or("")])?;
    }
    for item in persisted
        .get("providers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = item["id"].as_str().unwrap_or_default();
        if id.is_empty() || !provider_ids.insert(id.to_owned()) {
            continue;
        }
        transaction.execute("INSERT INTO provider_profiles(id, payload, created_at, updated_at) VALUES(?1, ?2, ?3, ?4)", params![id, item.to_string(), "", ""])?;
    }
    for item in persisted
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = item["id"].as_str().unwrap_or_default();
        let provider_id = item["providerId"].as_str().unwrap_or_default();
        if id.is_empty() || !provider_ids.contains(provider_id) || !model_ids.insert(id.to_owned())
        {
            continue;
        }
        transaction.execute("INSERT INTO model_profiles(id, provider_id, payload, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5)", params![id, provider_id, item.to_string(), "", ""])?;
    }
    for item in persisted
        .get("memories")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = item["id"].as_str().unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let assistant_id = item["assistantId"]
            .as_str()
            .filter(|assistant_id| assistant_ids.contains(*assistant_id));
        transaction.execute("INSERT OR IGNORE INTO memories(id, assistant_id, content, source, enabled, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![id, assistant_id, item["content"].as_str().unwrap_or(""), item["source"].as_str().unwrap_or("user"), item["enabled"].as_bool().unwrap_or(true), item["createdAt"].as_str().unwrap_or(""), item["updatedAt"].as_str().unwrap_or("")])?;
    }
    for conversation in persisted
        .get("conversations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = conversation["id"].as_str().unwrap_or_default();
        let assistant_id = conversation["assistantId"].as_str().unwrap_or_default();
        if id.is_empty()
            || !assistant_ids.contains(assistant_id)
            || !conversation_ids.insert(id.to_owned())
        {
            continue;
        }
        let model_profile_id = conversation["modelProfileId"]
            .as_str()
            .filter(|model_id| model_ids.contains(*model_id));
        transaction.execute("INSERT INTO conversations(id, assistant_id, title, private_chat, created_at, updated_at, model_profile_id) VALUES(?1, ?2, ?3, 0, ?4, ?5, ?6)", params![id, assistant_id, conversation["title"].as_str().unwrap_or("New conversation"), conversation["createdAt"].as_str().unwrap_or(""), conversation["updatedAt"].as_str().unwrap_or(""), model_profile_id])?;
        for message in conversation["messages"].as_array().into_iter().flatten() {
            let message_id = message["id"].as_str().unwrap_or_default();
            if message_id.is_empty() {
                continue;
            }
            transaction.execute("INSERT OR IGNORE INTO messages(id, conversation_id, role, created_at, model_id, provider_id) VALUES(?1, ?2, ?3, ?4, ?5, ?6)", params![message_id, id, message["role"].as_str().unwrap_or("user"), message["createdAt"].as_str().unwrap_or(""), message["modelId"].as_str(), message["providerId"].as_str()])?;
            for part in message["parts"].as_array().into_iter().flatten() {
                let part_id = part["id"].as_str().unwrap_or_default();
                if part_id.is_empty() {
                    continue;
                }
                transaction.execute("INSERT OR IGNORE INTO message_parts(id, message_id, part_type, text, payload) VALUES(?1, ?2, ?3, ?4, ?5)", params![part_id, message_id, part["type"].as_str().unwrap_or("text"), part["text"].as_str(), serde_json::to_string(&part["metadata"]).unwrap_or_else(|_| "null".into())])?;
            }
        }
    }
    for attachment in persisted
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = attachment["id"].as_str().unwrap_or_default();
        let conversation_id = attachment["conversationId"].as_str().unwrap_or_default();
        if id.is_empty()
            || conversation_id.is_empty()
            || !conversation_ids.contains(conversation_id)
        {
            continue;
        }
        let path = attachment_paths
            .get(id)
            .or_else(|| known_attachment_paths.get(id))
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        transaction.execute(
            "INSERT OR IGNORE INTO attachments(id, conversation_id, name, path, size_bytes, content_type, created_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
            params![
                id,
                conversation_id,
                attachment["name"].as_str().unwrap_or("attachment"),
                path,
                attachment["sizeBytes"].as_u64().unwrap_or(0) as i64,
                attachment["contentType"].as_str().unwrap_or("text/plain")
            ],
        )?;
    }
    for permission in persisted
        .get("permissions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = permission["id"].as_str().unwrap_or_default();
        let assistant_id = permission["assistantId"].as_str().unwrap_or_default();
        let scope = permission["scope"].as_str().unwrap_or_default();
        let conversation_id = permission["conversationId"].as_str();
        if id.is_empty()
            || !assistant_ids.contains(assistant_id)
            || !matches!(scope, "chat" | "assistant")
            || conversation_id
                .is_some_and(|conversation_id| !conversation_ids.contains(conversation_id))
        {
            continue;
        }
        transaction.execute(
            "INSERT OR IGNORE INTO permissions(id, tool_name, scope, assistant_id, conversation_id, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                permission["toolName"].as_str().unwrap_or_default(),
                scope,
                assistant_id,
                conversation_id,
                permission["createdAt"].as_str().unwrap_or_default(),
                permission["updatedAt"].as_str().unwrap_or_default()
            ],
        )?;
    }
    for peer in persisted
        .get("deviceLink")
        .and_then(|value| value.get("peers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = peer["id"].as_str() else {
            continue;
        };
        let Some(name) = peer["name"].as_str() else {
            continue;
        };
        let Some(fingerprint) = peer["fingerprint"].as_str() else {
            continue;
        };
        let scopes = peer["scopes"].clone();
        if id.is_empty() || name.is_empty() || fingerprint.is_empty() || !scopes.is_array() {
            continue;
        }
        transaction.execute(
            "INSERT INTO device_link_peers(id, name, fingerprint, scopes_json, address, connected, last_seen_at, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'), datetime('now')) ON CONFLICT(id) DO UPDATE SET name=excluded.name, fingerprint=excluded.fingerprint, scopes_json=excluded.scopes_json, address=excluded.address, connected=excluded.connected, last_seen_at=excluded.last_seen_at, updated_at=excluded.updated_at",
            params![
                id,
                name,
                fingerprint,
                scopes.to_string(),
                peer["address"].as_str(),
                peer["connected"].as_bool().unwrap_or(false),
                peer["lastSeenAt"].as_str(),
            ],
        )?;
    }
    transaction.commit()
}

fn load_attachment_metadata(connection: &Connection) -> Result<Vec<Value>> {
    let mut statement = connection.prepare(
        "SELECT id, conversation_id, name, size_bytes, content_type FROM attachments ORDER BY created_at, id",
    )?;
    statement
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "conversationId": row.get::<_, String>(1)?,
                "name": row.get::<_, String>(2)?,
                "sizeBytes": row.get::<_, i64>(3)?,
                "contentType": row.get::<_, String>(4)?
            }))
        })?
        .collect()
}

fn load_device_link_peers_from_connection(
    connection: &Connection,
) -> Result<Vec<crate::device_link::DeviceLinkPeer>> {
    let mut statement = connection.prepare(
        "SELECT id, name, fingerprint, scopes_json, address, connected, last_seen_at FROM device_link_peers ORDER BY created_at, id",
    )?;
    statement
        .query_map([], |row| {
            let scopes_json: String = row.get(3)?;
            let scopes = serde_json::from_str(&scopes_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
            })?;
            Ok(crate::device_link::DeviceLinkPeer {
                id: row.get(0)?,
                name: row.get(1)?,
                fingerprint: row.get(2)?,
                scopes,
                connected: row.get::<_, i64>(5)? != 0,
                last_seen_at: row.get(6)?,
                address: row.get(4)?,
            })
        })?
        .collect()
}

#[cfg(test)]
pub fn load_device_link_peers(path: &Path) -> Result<Vec<crate::device_link::DeviceLinkPeer>> {
    let connection = connection(path)?;
    load_device_link_peers_from_connection(&connection)
}

#[cfg(test)]
pub fn upsert_device_link_peer(
    path: &Path,
    peer: &crate::device_link::DeviceLinkPeer,
) -> Result<()> {
    let connection = connection(path)?;
    connection.execute(
        "INSERT INTO device_link_peers(id, name, fingerprint, scopes_json, address, connected, last_seen_at, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'), datetime('now')) ON CONFLICT(id) DO UPDATE SET name=excluded.name, fingerprint=excluded.fingerprint, scopes_json=excluded.scopes_json, address=excluded.address, connected=excluded.connected, last_seen_at=excluded.last_seen_at, updated_at=excluded.updated_at",
        params![
            peer.id,
            peer.name,
            peer.fingerprint,
            serde_json::to_string(&peer.scopes).unwrap_or_else(|_| "[]".into()),
            peer.address,
            peer.connected,
            peer.last_seen_at,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
pub fn load_device_link_identity(path: &Path) -> Result<Option<(String, String)>> {
    let connection = connection(path)?;
    connection
        .query_row(
            "SELECT device_id, fingerprint FROM device_link_identity WHERE id = 'singleton'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
}

#[cfg(test)]
pub fn save_device_link_identity(path: &Path, device_id: &str, fingerprint: &str) -> Result<()> {
    let connection = connection(path)?;
    connection.execute(
        "INSERT INTO device_link_identity(id, device_id, fingerprint, created_at, updated_at) VALUES('singleton', ?1, ?2, datetime('now'), datetime('now')) ON CONFLICT(id) DO UPDATE SET device_id=excluded.device_id, fingerprint=excluded.fingerprint, updated_at=excluded.updated_at",
        params![device_id, fingerprint],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_creates_schema_and_enables_foreign_keys() -> Result<()> {
        let connection = Connection::open_in_memory()?;
        migrate(&connection)?;
        let version: i64 =
            connection.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        let foreign_keys: i64 =
            connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(foreign_keys, 1);
        assert!(
            connection
                .query_row::<String, _, _>(
                    "SELECT sql FROM sqlite_master WHERE name = 'app_settings'",
                    [],
                    |row| row.get(0)
                )
                .is_ok()
        );
        assert!(
            connection
                .query_row::<String, _, _>(
                    "SELECT sql FROM sqlite_master WHERE name = 'permissions'",
                    [],
                    |row| row.get(0)
                )
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn migration_preserves_a_v1_database_and_adds_v2_v3_and_v4_state() -> Result<()> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            r#"
            CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
            INSERT INTO schema_migrations(version, applied_at) VALUES(1, '2026-01-01');
            CREATE TABLE assistants (
                id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE provider_profiles (
                id TEXT PRIMARY KEY,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE model_profiles (
                id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE conversations (
                id TEXT PRIMARY KEY,
                assistant_id TEXT NOT NULL,
                title TEXT NOT NULL,
                private_chat INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            INSERT INTO assistants(id, schema_version, payload, created_at, updated_at)
            VALUES('assistant-v1', 1, '{}', '2026-01-01', '2026-01-01');
            INSERT INTO conversations(id, assistant_id, title, created_at, updated_at)
            VALUES('chat-v1', 'assistant-v1', 'Preserved', '2026-01-01', '2026-01-01');
            "#,
        )?;

        migrate(&connection)?;
        let version: i64 =
            connection.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        let title: String = connection.query_row(
            "SELECT title FROM conversations WHERE id = 'chat-v1'",
            [],
            |row| row.get(0),
        )?;
        let model_override_column: String = connection.query_row(
            "SELECT name FROM pragma_table_info('conversations') WHERE name = 'model_profile_id'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(title, "Preserved");
        assert_eq!(model_override_column, "model_profile_id");
        assert!(
            connection
                .query_row::<String, _, _>(
                    "SELECT sql FROM sqlite_master WHERE name = 'app_state'",
                    [],
                    |row| row.get(0)
                )
                .is_ok()
        );
        assert!(
            connection
                .query_row::<String, _, _>(
                    "SELECT sql FROM sqlite_master WHERE name = 'permissions'",
                    [],
                    |row| row.get(0)
                )
                .is_ok()
        );
        assert!(
            connection
                .query_row::<String, _, _>(
                    "SELECT sql FROM sqlite_master WHERE name = 'device_link_identity'",
                    [],
                    |row| row.get(0)
                )
                .is_ok()
        );
        assert!(
            connection
                .query_row::<String, _, _>(
                    "SELECT sql FROM sqlite_master WHERE name = 'device_link_peers'",
                    [],
                    |row| row.get(0)
                )
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn device_link_identity_and_peer_metadata_round_trip_without_secrets() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "juniper-device-link-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        initialize(&path)?;
        save_device_link_identity(&path, "device-host", "SHA256:host")?;
        let peer = crate::device_link::DeviceLinkPeer {
            id: "device-peer".into(),
            name: "Phone".into(),
            fingerprint: "SHA256:peer".into(),
            scopes: vec![crate::device_link::Scope::Inference],
            connected: false,
            last_seen_at: None,
            address: Some("https://device.local:8443".into()),
        };
        upsert_device_link_peer(&path, &peer)?;
        assert_eq!(
            load_device_link_identity(&path)?,
            Some(("device-host".into(), "SHA256:host".into()))
        );
        assert_eq!(load_device_link_peers(&path)?, vec![peer]);
        let connection = connection(&path)?;
        let payload: String = connection.query_row(
            "SELECT scopes_json FROM device_link_peers WHERE id = 'device-peer'",
            [],
            |row| row.get(0),
        )?;
        assert!(!payload.contains("secret"));
        drop(connection);
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn migration_recovers_an_interrupted_v2_column_change() -> Result<()> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            r#"
            CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
            INSERT INTO schema_migrations(version, applied_at) VALUES(1, '2026-01-01');
            CREATE TABLE assistants (
                id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE provider_profiles (
                id TEXT PRIMARY KEY,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE model_profiles (
                id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE conversations (
                id TEXT PRIMARY KEY,
                assistant_id TEXT NOT NULL,
                title TEXT NOT NULL,
                private_chat INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                model_profile_id TEXT
            );
            "#,
        )?;

        migrate(&connection)?;
        let version: i64 =
            connection.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        assert_eq!(version, SCHEMA_VERSION);
        assert!(
            connection
                .query_row::<String, _, _>(
                    "SELECT sql FROM sqlite_master WHERE name = 'app_state'",
                    [],
                    |row| row.get(0)
                )
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn migration_refuses_a_database_from_a_newer_schema() -> Result<()> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL); INSERT INTO schema_migrations VALUES(99, 'future');",
        )?;
        let error = migrate(&connection).expect_err("a newer schema must be refused");
        assert!(
            error
                .to_string()
                .contains("schema v99 was written by a newer Juniper")
        );
        let version: i64 =
            connection.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        assert_eq!(version, 99);
        Ok(())
    }

    #[test]
    fn app_state_round_trips_and_private_chats_are_excluded() -> Result<()> {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("juniper-storage-test-{suffix}.db"));
        let data = serde_json::json!({
            "assistants": [{
                "id": "assistant-1",
                "schemaVersion": 2,
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }],
            "providers": [{ "id": "provider-1" }],
            "models": [{ "id": "model-1", "providerId": "provider-1" }],
            "memories": [],
            "attachments": [
                {
                    "id": "attachment-saved",
                    "conversationId": "chat-saved",
                    "name": "notes.txt",
                    "sizeBytes": 12,
                    "contentType": "text/plain"
                },
                {
                    "id": "attachment-private",
                    "conversationId": "chat-private",
                    "name": "private.txt",
                    "sizeBytes": 14,
                    "contentType": "text/plain"
                }
            ],
            "permissions": [{
                "id": "permission-1",
                "toolName": "memory.list",
                "scope": "assistant",
                "assistantId": "assistant-1",
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }],
            "conversations": [
                {
                    "id": "chat-saved",
                    "assistantId": "assistant-1",
                    "title": "Saved",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z",
                    "modelProfileId": "model-1",
                    "messages": []
                },
                {
                    "id": "chat-private",
                    "assistantId": "assistant-1",
                    "title": "Private",
                    "privateChat": true,
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z",
                    "messages": []
                }
            ],
            "settings": { "telemetry": "off" }
        });
        save_app_data_with_paths(&path, &data, &HashMap::new())?;
        let loaded = load_app_data(&path)?.expect("saved state should be present");
        let conversations = loaded["conversations"]
            .as_array()
            .expect("conversations should be an array");
        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0]["id"], "chat-saved");
        assert_eq!(conversations[0]["modelProfileId"], "model-1");
        let attachments = loaded["attachments"]
            .as_array()
            .expect("attachments should be an array");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0]["id"], "attachment-saved");

        let connection = Connection::open(&path)?;
        assert_eq!(
            connection
                .query_row::<i64, _, _>("SELECT COUNT(*) FROM assistants", [], |row| row.get(0))?,
            1
        );
        assert_eq!(
            connection
                .query_row::<i64, _, _>("SELECT COUNT(*) FROM permissions", [], |row| row.get(0))?,
            1
        );
        assert_eq!(
            connection
                .query_row::<i64, _, _>("SELECT COUNT(*) FROM conversations", [], |row| row
                    .get(0))?,
            1
        );
        assert_eq!(
            connection
                .query_row::<i64, _, _>("SELECT COUNT(*) FROM attachments", [], |row| row.get(0))?,
            1
        );
        std::fs::remove_file(path).expect("temporary database should be removable");
        Ok(())
    }

    /// Removing a provider keeps its model profiles (marked not-found), and
    /// removing a managed model leaves chats pinned to it. Both used to fail
    /// the foreign keys, so that save and every later one was rejected and all
    /// subsequent changes were lost on restart.
    #[test]
    fn raw_reasoning_parts_are_never_persisted() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("juniper-reasoning-{}.db", uuid::Uuid::new_v4()));
        save_app_data_with_paths(
            &path,
            &json!({
                "assistants": [{ "id": "assistant-1", "schemaVersion": 2 }],
                "conversations": [{
                    "id": "chat-1", "assistantId": "assistant-1", "title": "Chat",
                    "messages": [{ "id": "message-1", "role": "assistant", "parts": [
                        { "id": "part-1", "type": "reasoning", "text": "private analysis" },
                        { "id": "part-2", "type": "text", "text": "Answer." }
                    ], "provenance": { "backend": "generic" } }]
                }]
            }),
            &HashMap::new(),
        )?;
        let loaded = load_app_data(&path)?.expect("saved state");
        let message = &loaded["conversations"][0]["messages"][0];
        assert_eq!(message["parts"].as_array().map(Vec::len), Some(1));
        assert_eq!(message["parts"][0]["type"], "text");
        assert_eq!(message["provenance"]["backend"], "generic");
        assert!(
            !std::fs::read(&path)
                .map(|bytes| String::from_utf8_lossy(&bytes).contains("private analysis"))
                .unwrap_or(true)
        );
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn saves_survive_references_left_dangling_by_removals() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("juniper-dangling-test-{}.db", uuid::Uuid::new_v4()));
        let assistant = json!({ "id": "assistant-1", "schemaVersion": 2 });
        let removed_provider = json!({
            "assistants": [assistant],
            "providers": [{ "id": "provider-kept" }],
            "models": [
                { "id": "model-orphan", "providerId": "provider-removed", "status": "not-found" },
                { "id": "model-kept", "providerId": "provider-kept" }
            ],
            "memories": [{ "id": "memory-1", "assistantId": "assistant-removed", "content": "kept" }],
            "attachments": [{
                "id": "attachment-1",
                "conversationId": "chat-1",
                "name": "notes.txt",
                "sizeBytes": 5,
                "contentType": "text/plain"
            }],
            "permissions": [
                { "id": "grant-orphan", "toolName": "memory.list", "scope": "chat",
                  "assistantId": "assistant-1", "conversationId": "chat-removed" },
                { "id": "grant-kept", "toolName": "memory.list", "scope": "assistant",
                  "assistantId": "assistant-1" }
            ],
            "conversations": [{
                "id": "chat-1",
                "assistantId": "assistant-1",
                "title": "Pinned to a removed model",
                "modelProfileId": "model-removed",
                "messages": [{ "id": "message-1", "role": "user",
                               "parts": [{ "id": "part-1", "type": "text", "text": "hello" }] }]
            }],
            "settings": { "telemetry": "off" }
        });
        save_app_data_with_paths(&path, &removed_provider, &HashMap::new())?;

        // A later change must still be saved.
        let mut renamed = removed_provider.clone();
        renamed["conversations"][0]["title"] = json!("Renamed after removal");
        save_app_data_with_paths(&path, &renamed, &HashMap::new())?;

        let loaded = load_app_data(&path)?.expect("saved state should be present");
        assert_eq!(loaded["conversations"][0]["title"], "Renamed after removal");
        assert_eq!(
            loaded["conversations"][0]["modelProfileId"],
            "model-removed"
        );
        assert_eq!(loaded["models"].as_array().map(Vec::len), Some(2));
        assert_eq!(loaded["attachments"][0]["id"], "attachment-1");

        let connection = Connection::open(&path)?;
        let count = |sql: &str| connection.query_row::<i64, _, _>(sql, [], |row| row.get(0));
        assert_eq!(count("SELECT COUNT(*) FROM model_profiles")?, 1);
        assert_eq!(
            count("SELECT COUNT(*) FROM conversations WHERE model_profile_id IS NULL")?,
            1
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM memories WHERE assistant_id IS NULL")?,
            1
        );
        assert_eq!(count("SELECT COUNT(*) FROM permissions")?, 1);
        assert_eq!(count("SELECT COUNT(*) FROM message_parts")?, 1);
        assert_eq!(count("SELECT COUNT(*) FROM pragma_foreign_key_check")?, 0);
        drop(connection);
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn attachment_paths_survive_restart_and_subsequent_saves()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos();
        let database = std::env::temp_dir().join(format!("juniper-attachment-test-{suffix}.db"));
        let attachment = std::env::temp_dir().join(format!("juniper-attachment-test-{suffix}.txt"));
        std::fs::write(&attachment, "persisted attachment")?;
        let data = serde_json::json!({
            "assistants": [{
                "id": "assistant-1",
                "schemaVersion": 2,
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z"
            }],
            "providers": [],
            "models": [],
            "memories": [],
            "permissions": [],
            "attachments": [{
                "id": "attachment-1",
                "conversationId": "chat-1",
                "name": "notes.txt",
                "sizeBytes": 20,
                "contentType": "text/plain"
            }],
            "conversations": [{
                "id": "chat-1",
                "assistantId": "assistant-1",
                "title": "Saved",
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z",
                "messages": []
            }],
            "settings": { "telemetry": "off" }
        });
        let mut paths = HashMap::from([(String::from("attachment-1"), attachment.clone())]);
        save_app_data_with_paths(&database, &data, &paths)?;

        let loaded = load_app_data(&database)?.expect("saved state should be present");
        let restored_paths = load_attachment_paths(&database)?;
        assert_eq!(restored_paths.get("attachment-1"), Some(&attachment));
        paths.clear();
        save_app_data_with_paths(&database, &loaded, &paths)?;

        let connection = Connection::open(&database)?;
        let stored_path: String = connection.query_row(
            "SELECT path FROM attachments WHERE id = 'attachment-1'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(PathBuf::from(stored_path), attachment);
        std::fs::remove_file(database)?;
        std::fs::remove_file(attachment)?;
        Ok(())
    }
}
