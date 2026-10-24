use rusqlite::{Connection, OptionalExtension, Result, params, types::Type};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

pub const SCHEMA_VERSION: i64 = 6;

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

/// Agent, project, schedule, skill, and plugin state. The native host is the
/// only writer of these tables: `save_app_data` deletes and rebuilds the
/// snapshot projections above, so nothing here references them by foreign
/// key, and a stale interface snapshot can never delete or rewrite task
/// activity or approvals.
const MIGRATION_V5: &str = r#"
CREATE TABLE IF NOT EXISTS agent_profiles (
    assistant_id TEXT PRIMARY KEY,
    payload TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_settings (
    id TEXT PRIMARY KEY CHECK(id = 'singleton'),
    payload TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    instructions TEXT NOT NULL DEFAULT '',
    archived INTEGER NOT NULL DEFAULT 0,
    payload TEXT NOT NULL DEFAULT '{}',
    revision INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS project_roots (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    label TEXT NOT NULL,
    access TEXT NOT NULL CHECK(access IN ('read', 'read-write')),
    created_at TEXT NOT NULL,
    revoked_at TEXT
);
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    root_id TEXT NOT NULL,
    depth INTEGER NOT NULL DEFAULT 0,
    role TEXT,
    assistant_id TEXT NOT NULL,
    project_id TEXT,
    schedule_run_id TEXT,
    title TEXT NOT NULL,
    goal TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'waiting-input', 'waiting-approval', 'paused', 'interrupted', 'completed', 'failed', 'cancelled')),
    status_reason TEXT,
    current_action TEXT,
    plan TEXT NOT NULL DEFAULT '[]',
    budget TEXT NOT NULL,
    usage TEXT NOT NULL,
    provenance TEXT NOT NULL,
    transcript TEXT NOT NULL DEFAULT '[]',
    pending TEXT,
    result TEXT,
    revision INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finished_at TEXT
);
CREATE INDEX IF NOT EXISTS tasks_status ON tasks(status, created_at);
CREATE INDEX IF NOT EXISTS tasks_parent ON tasks(parent_id);
CREATE TABLE IF NOT EXISTS task_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    summary TEXT NOT NULL,
    detail TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS task_events_task ON task_events(task_id, id);
CREATE TABLE IF NOT EXISTS task_actions (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    step INTEGER NOT NULL,
    call_id TEXT NOT NULL,
    tool TEXT NOT NULL,
    arguments TEXT NOT NULL,
    binding TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('proposed', 'awaiting-approval', 'approved', 'denied', 'started', 'succeeded', 'failed', 'uncertain', 'cancelled')),
    result TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(task_id, call_id)
);
CREATE TABLE IF NOT EXISTS approvals (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    action_id TEXT NOT NULL REFERENCES task_actions(id) ON DELETE CASCADE,
    tool TEXT NOT NULL,
    capability TEXT NOT NULL,
    preview TEXT NOT NULL,
    binding TEXT NOT NULL,
    standing_allowed INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK(status IN ('pending', 'approved', 'denied', 'expired', 'stale')),
    decision TEXT,
    created_at TEXT NOT NULL,
    decided_at TEXT,
    expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS grants (
    id TEXT PRIMARY KEY,
    assistant_id TEXT NOT NULL,
    project_id TEXT,
    capability TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    expires_at TEXT,
    revoked_at TEXT
);
CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    project_id TEXT,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    file_name TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    source_ids TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY,
    task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    url TEXT,
    final_url TEXT,
    location TEXT,
    retrieved_at TEXT NOT NULL,
    content_sha256 TEXT,
    excerpt TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS schedules (
    id TEXT PRIMARY KEY,
    assistant_id TEXT NOT NULL,
    project_id TEXT,
    title TEXT NOT NULL,
    payload TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    next_run_at TEXT,
    revision INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS schedule_runs (
    id TEXT PRIMARY KEY,
    schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    scheduled_for TEXT NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL,
    task_id TEXT,
    lease_until TEXT,
    detail TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(schedule_id, scheduled_for, attempt)
);
CREATE TABLE IF NOT EXISTS skills (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    payload TEXT NOT NULL,
    digest TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0,
    bundled INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS plugins (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('mcp-stdio', 'mcp-http')),
    name TEXT NOT NULL,
    payload TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS inbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    read INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS memory_outbox (
    id TEXT PRIMARY KEY,
    assistant_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK(operation IN ('save', 'delete')),
    payload TEXT NOT NULL,
    task_id TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS doc_files (
    root_id TEXT NOT NULL REFERENCES project_roots(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    version TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_nanos INTEGER NOT NULL,
    indexed_at TEXT NOT NULL,
    PRIMARY KEY(root_id, path)
);
CREATE VIRTUAL TABLE IF NOT EXISTS doc_chunks USING fts5(
    text,
    project_id UNINDEXED,
    root_id UNINDEXED,
    path UNINDEXED,
    version UNINDEXED,
    start_offset UNINDEXED,
    end_offset UNINDEXED,
    tokenize = 'unicode61'
);
"#;

/// Retry-safe schedule settlement and idempotent task dispatch. This is a
/// separate migration so databases already at v5 receive the new constraints.
const MIGRATION_V6: &str = r#"
ALTER TABLE schedule_runs ADD COLUMN finalized_at TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS tasks_schedule_run_id_unique
    ON tasks(schedule_run_id) WHERE schedule_run_id IS NOT NULL;
"#;

fn migrate(connection: &Connection) -> Result<()> {
    debug_assert_eq!(SCHEMA_VERSION, 6);
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
    if version < 5 {
        transaction.execute_batch(MIGRATION_V5)?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(5, datetime('now'))",
            [],
        )?;
    }
    if version < 6 {
        transaction.execute_batch(MIGRATION_V6)?;
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(6, datetime('now'))",
            [],
        )?;
    }
    transaction.commit()
}

fn connection(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path)?;
    // The agent runtime and interface saves write concurrently.
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    migrate(&connection)?;
    Ok(connection)
}

/// Opens (and migrates) the database once at startup. An existing database
/// on an older schema is first copied with `VACUUM INTO` beside the original,
/// so a migration can be rolled back by restoring that file; the migration
/// itself runs in one transaction and leaves the old schema intact on failure.
pub fn initialize(path: &Path) -> Result<()> {
    if let Some(version) = existing_version(path)?
        && version < SCHEMA_VERSION
    {
        backup_before_migration(path, version)?;
    }
    let _ = connection(path)?;
    Ok(())
}

fn existing_version(path: &Path) -> Result<Option<i64>> {
    if !path.is_file() {
        return Ok(None);
    }
    let connection = Connection::open(path)?;
    let has_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations')",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(None);
    }
    connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .map(Some)
}

/// The backup path for a database migrating away from `version`.
pub fn migration_backup_path(path: &Path, version: i64) -> PathBuf {
    path.with_file_name(format!(
        "{}.schema-v{version}.backup",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("juniper.db")
    ))
}

fn backup_before_migration(path: &Path, version: i64) -> Result<()> {
    let backup = migration_backup_path(path, version);
    // An earlier interrupted attempt may have left a backup already; the
    // first one taken from the unmigrated file is the one worth keeping.
    if backup.exists() {
        return Ok(());
    }
    let staging = backup.with_extension("backup.partial");
    let _ = std::fs::remove_file(&staging);
    let connection = Connection::open(path)?;
    connection.execute(
        "VACUUM INTO ?1",
        params![staging.to_string_lossy().into_owned()],
    )?;
    drop(connection);
    std::fs::rename(&staging, &backup).map_err(|error| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
            Some(format!("could not keep the pre-migration backup: {error}")),
        )
    })
}

/// Opens the database for native agent state. Exposed so the agent store
/// shares the one migration path.
pub fn open(path: &Path) -> Result<Connection> {
    connection(path)
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
    let transaction = connection.unchecked_transaction()?;
    let mut memory_change_ids = Vec::new();
    {
        let mut statement = transaction.prepare(
            "SELECT id, assistant_id, operation, payload, created_at FROM memory_outbox ORDER BY created_at, id",
        )?;
        let changes = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    serde_json::from_str::<Value>(&row.get::<_, String>(3)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
                    })?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if !changes.is_empty() {
            let memories = persisted
                .as_object_mut()
                .ok_or_else(|| rusqlite::Error::InvalidQuery)?
                .entry("memories")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or_else(|| rusqlite::Error::InvalidQuery)?;
            for (id, assistant_id, operation, payload, created_at) in changes {
                let memory_id = format!("memory-agent-{id}");
                // Keep a change visible to the renderer until its snapshot
                // includes it. An unrelated stale save can race the event
                // listener; acknowledging here would lose the memory on the
                // next stale snapshot before the renderer ever observed it.
                // The new change id proves observation; content may already
                // contain a subsequent user edit, which remains renderer-owned.
                let observed = match operation.as_str() {
                    "save" => memories.iter().any(|memory| memory["id"] == memory_id),
                    "delete" => payload["id"].as_str().is_some_and(|target| {
                        !memories.iter().any(|memory| memory["id"] == target)
                    }),
                    _ => false,
                };
                match operation.as_str() {
                    "save" => {
                        if let Some(content) = payload["content"].as_str().filter(|s| !s.is_empty())
                            && !memories.iter().any(|memory| memory["id"] == memory_id)
                        {
                            memories.push(json!({
                                "id": memory_id,
                                "assistantId": assistant_id,
                                "content": content,
                                "source": "agent-task",
                                "enabled": true,
                                "createdAt": created_at,
                                "updatedAt": created_at,
                            }));
                        }
                    }
                    "delete" => {
                        if let Some(target_id) = payload["id"].as_str() {
                            memories.retain(|memory| memory["id"] != target_id);
                        }
                    }
                    _ => return Err(rusqlite::Error::InvalidQuery),
                }
                if observed {
                    memory_change_ids.push(id);
                }
            }
        }
    }
    let payload = serde_json::to_string(&persisted).unwrap_or_else(|_| "{}".into());
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
    for id in memory_change_ids {
        transaction.execute("DELETE FROM memory_outbox WHERE id = ?1", [id])?;
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
    fn approved_memory_outbox_is_applied_and_acknowledged_with_the_snapshot() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "juniper-memory-outbox-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let snapshot = json!({
            "assistants": [{
                "id": "assistant-1",
                "schemaVersion": 2,
                "createdAt": "2026-10-15T00:00:00Z",
                "updatedAt": "2026-10-15T00:00:00Z"
            }],
            "memories": [],
            "settings": {}
        });
        save_app_data_with_paths(&path, &snapshot, &HashMap::new())?;
        let connection = super::connection(&path)?;
        connection.execute(
            "INSERT INTO memory_outbox(id, assistant_id, operation, payload, created_at) VALUES('change-1', 'assistant-1', 'save', ?1, '2026-10-15T12:00:00Z')",
            [json!({ "content": "Remember the project codename." }).to_string()],
        )?;
        drop(connection);

        // The renderer has not yet seen the approved change; it saves its
        // existing snapshot. Merge the change, but retain it for the renderer
        // until a subsequent snapshot includes the approved memory.
        save_app_data_with_paths(&path, &snapshot, &HashMap::new())?;
        save_app_data_with_paths(&path, &snapshot, &HashMap::new())?;
        let saved = load_app_data(&path)?.expect("snapshot exists");
        assert_eq!(
            saved["memories"][0],
            json!({
                "id": "memory-agent-change-1",
                "assistantId": "assistant-1",
                "content": "Remember the project codename.",
                "source": "agent-task",
                "enabled": true,
                "createdAt": "2026-10-15T12:00:00Z",
                "updatedAt": "2026-10-15T12:00:00Z"
            })
        );
        let connection = super::connection(&path)?;
        let outbox_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM memory_outbox", [], |row| row.get(0))?;
        assert_eq!(
            outbox_count, 1,
            "stale snapshots cannot acknowledge unseen changes"
        );
        let projected: i64 = connection.query_row(
            "SELECT COUNT(*) FROM memories WHERE id = 'memory-agent-change-1'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(projected, 1);
        drop(connection);
        let mut edited = saved.clone();
        edited["memories"][0]["content"] = json!("The user's revised project note.");
        edited["memories"][0]["enabled"] = json!(false);
        save_app_data_with_paths(&path, &edited, &HashMap::new())?;
        assert_eq!(
            load_app_data(&path)?.unwrap()["memories"],
            edited["memories"]
        );
        let connection = super::connection(&path)?;
        let acknowledged: i64 =
            connection.query_row("SELECT COUNT(*) FROM memory_outbox", [], |row| row.get(0))?;
        assert_eq!(
            acknowledged, 0,
            "the renderer observed and persisted the memory"
        );

        connection.execute(
            "INSERT INTO memory_outbox(id, assistant_id, operation, payload, created_at) VALUES('change-2', 'assistant-1', 'save', ?1, '2026-10-15T12:01:00Z')",
            [json!({ "content": "Second approved memory." }).to_string()],
        )?;
        connection.execute_batch(
            "CREATE TRIGGER fail_memory_outbox_ack BEFORE DELETE ON memory_outbox BEGIN SELECT RAISE(ABORT, 'simulated acknowledgement failure'); END;",
        )?;
        drop(connection);
        let mut acknowledging = edited.clone();
        acknowledging["memories"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "memory-agent-change-2",
                "assistantId": "assistant-1",
                "content": "Second approved memory.",
                "source": "agent-task",
                "enabled": true,
                "createdAt": "2026-10-15T12:01:00Z",
                "updatedAt": "2026-10-15T12:01:00Z"
            }));
        assert!(save_app_data_with_paths(&path, &acknowledging, &HashMap::new()).is_err());
        let loaded_after_failure = load_app_data(&path)?.expect("prior snapshot remains");
        assert_eq!(
            loaded_after_failure["memories"].as_array().unwrap().len(),
            1
        );
        let connection = super::connection(&path)?;
        let remaining: i64 =
            connection.query_row("SELECT COUNT(*) FROM memory_outbox", [], |row| row.get(0))?;
        assert_eq!(remaining, 1);
        drop(connection);
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn approved_memory_deletions_wait_for_a_snapshot_that_observed_them() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("juniper-memory-delete-{}.db", uuid::Uuid::new_v4()));
        let snapshot = json!({"memories": [{"id": "old", "content": "Remove me"}], "settings": {}});
        save_app_data_with_paths(&path, &snapshot, &HashMap::new())?;
        let connection = super::connection(&path)?;
        connection.execute(
            "INSERT INTO memory_outbox(id, assistant_id, operation, payload, created_at) VALUES('delete-1', 'assistant-1', 'delete', ?1, '2026-10-15T12:00:00Z')",
            [json!({"id": "old"}).to_string()],
        )?;
        drop(connection);
        save_app_data_with_paths(&path, &snapshot, &HashMap::new())?;
        save_app_data_with_paths(&path, &snapshot, &HashMap::new())?;
        let saved = load_app_data(&path)?.unwrap();
        assert!(saved["memories"].as_array().unwrap().is_empty());
        let connection = super::connection(&path)?;
        let pending: i64 =
            connection.query_row("SELECT COUNT(*) FROM memory_outbox", [], |row| row.get(0))?;
        assert_eq!(pending, 1);
        drop(connection);
        save_app_data_with_paths(&path, &saved, &HashMap::new())?;
        let connection = super::connection(&path)?;
        let pending: i64 =
            connection.query_row("SELECT COUNT(*) FROM memory_outbox", [], |row| row.get(0))?;
        assert_eq!(pending, 0);
        drop(connection);
        std::fs::remove_file(path).ok();
        Ok(())
    }

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
    fn migration_preserves_a_v1_database_and_adds_v2_through_v6_state() -> Result<()> {
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
        // Windows refuses to delete a file that is still open.
        drop(connection);
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
        // Windows refuses to delete a file that is still open.
        drop(connection);
        std::fs::remove_file(database)?;
        std::fs::remove_file(attachment)?;
        Ok(())
    }
    fn v4_database(path: &Path) -> Result<()> {
        let connection = Connection::open(path)?;
        connection.execute_batch(INITIAL_SCHEMA)?;
        connection.execute_batch(
            "ALTER TABLE conversations ADD COLUMN model_profile_id TEXT REFERENCES model_profiles(id);",
        )?;
        connection.execute_batch(MIGRATION_V2)?;
        connection.execute_batch(MIGRATION_V3)?;
        connection.execute_batch(MIGRATION_V4)?;
        connection.execute_batch(
            "INSERT INTO schema_migrations VALUES(2, 'x'), (3, 'x'), (4, 'x');
             INSERT INTO app_state(id, payload, updated_at) VALUES('singleton', '{\"assistants\":[],\"marker\":\"v4-user-data\"}', 'x');",
        )?;
        Ok(())
    }

    fn v5_database(path: &Path) -> Result<()> {
        v4_database(path)?;
        let connection = Connection::open(path)?;
        connection.execute_batch(MIGRATION_V5)?;
        connection.execute(
            "INSERT INTO schema_migrations(version, applied_at) VALUES(5, '2026-01-01')",
            [],
        )?;
        Ok(())
    }

    #[test]
    fn upgrading_an_existing_v5_database_adds_schedule_recovery_schema_and_backup() -> Result<()> {
        let root = std::env::temp_dir().join(format!("juniper-v5-to-v6-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("directory");
        let path = root.join("juniper.db");
        v5_database(&path)?;

        initialize(&path)?;

        let backup = migration_backup_path(&path, 5);
        assert!(
            backup.is_file(),
            "the v5 database is backed up before upgrade"
        );
        let backed_up_version: i64 = Connection::open(&backup)?.query_row(
            "SELECT MAX(version) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(backed_up_version, 5);

        let migrated = open(&path)?;
        let version: i64 =
            migrated.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        assert_eq!(version, 6);
        assert!(
            migrated
                .query_row::<String, _, _>(
                    "SELECT sql FROM sqlite_master WHERE type='index' AND name='tasks_schedule_run_id_unique'",
                    [],
                    |row| row.get(0),
                )
                .is_ok()
        );
        assert!(
            migrated
                .query_row::<String, _, _>(
                    "SELECT name FROM pragma_table_info('schedule_runs') WHERE name='finalized_at'",
                    [],
                    |row| row.get(0),
                )
                .is_ok()
        );
        let payload: String = migrated.query_row(
            "SELECT payload FROM app_state WHERE id = 'singleton'",
            [],
            |row| row.get(0),
        )?;
        assert!(payload.contains("v4-user-data"));
        drop(migrated);
        std::fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn v6_unique_index_failure_keeps_duplicate_v5_tasks_and_schema_unchanged() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("juniper-v6-conflict-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("directory");
        let path = root.join("juniper.db");
        v5_database(&path)?;
        Connection::open(&path)?.execute_batch(
            "INSERT INTO tasks(id, root_id, assistant_id, schedule_run_id, title, goal, status, budget, usage, provenance, created_at, updated_at)
             VALUES('task-1', 'task-1', 'assistant-1', 'run-1', 'One', 'Goal', 'queued', '{}', '{}', '{}', 'x', 'x'),
                   ('task-2', 'task-2', 'assistant-1', 'run-1', 'Two', 'Goal', 'queued', '{}', '{}', '{}', 'x', 'x');",
        )?;

        assert!(initialize(&path).is_err());

        let connection = Connection::open(&path)?;
        let version: i64 =
            connection.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        let tasks: i64 =
            connection.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))?;
        let has_finalized_at: i64 = connection.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('schedule_runs') WHERE name='finalized_at'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(version, 5);
        assert_eq!(
            tasks, 2,
            "the failed migration must preserve both task rows"
        );
        assert_eq!(
            has_finalized_at, 0,
            "the failed migration rolls back as a unit"
        );
        assert!(migration_backup_path(&path, 5).is_file());
        drop(connection);
        std::fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn upgrading_a_v4_database_keeps_a_restorable_backup_and_the_user_data() -> Result<()> {
        let root = std::env::temp_dir().join(format!("juniper-v5-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("directory");
        let path = root.join("juniper.db");
        v4_database(&path)?;
        initialize(&path)?;
        let backup = migration_backup_path(&path, 4);
        assert!(backup.is_file(), "a v4 backup is kept before migrating");
        let restored = Connection::open(&backup)?;
        let version: i64 =
            restored.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        assert_eq!(version, 4, "the backup is the unmigrated database");
        let migrated = open(&path)?;
        let payload: String = migrated.query_row(
            "SELECT payload FROM app_state WHERE id = 'singleton'",
            [],
            |row| row.get(0),
        )?;
        assert!(payload.contains("v4-user-data"));
        let version: i64 =
            migrated.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        assert_eq!(version, 6);
        // A second start neither re-backs-up nor fails.
        initialize(&path)?;
        std::fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn an_interrupted_v5_migration_leaves_the_v4_schema_untouched() -> Result<()> {
        let root = std::env::temp_dir().join(format!("juniper-v5-fail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("directory");
        let path = root.join("juniper.db");
        v4_database(&path)?;
        // A conflicting object makes the v5 batch fail part-way through.
        Connection::open(&path)?.execute_batch("CREATE VIEW tasks AS SELECT 1 AS id;")?;
        assert!(initialize(&path).is_err());
        let connection = Connection::open(&path)?;
        let version: i64 =
            connection.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        assert_eq!(version, 4);
        let agent_tables: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('projects', 'agent_profiles')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(agent_tables, 0, "no part of v5 was committed");
        let payload: String = connection.query_row(
            "SELECT payload FROM app_state WHERE id = 'singleton'",
            [],
            |row| row.get(0),
        )?;
        assert!(payload.contains("v4-user-data"));
        std::fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn a_stale_interface_snapshot_cannot_touch_native_agent_state() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("juniper-v5-stale-{}.db", uuid::Uuid::new_v4()));
        initialize(&path)?;
        let connection = open(&path)?;
        connection.execute_batch(
            "INSERT INTO tasks(id, root_id, assistant_id, title, goal, status, budget, usage, provenance, created_at, updated_at)
             VALUES('task-1', 'task-1', 'assistant-1', 'T', 'G', 'waiting-approval', '{}', '{}', '{}', 'x', 'x');
             INSERT INTO task_actions(id, task_id, step, call_id, tool, arguments, binding, status, created_at, updated_at)
             VALUES('action-1', 'task-1', 1, 'call-1', 'workspace.write', '{}', 'b', 'awaiting-approval', 'x', 'x');
             INSERT INTO approvals(id, task_id, action_id, tool, capability, preview, binding, status, created_at, expires_at)
             VALUES('approval-1', 'task-1', 'action-1', 'workspace.write', 'workspace.write', 'p', 'b', 'pending', 'x', 'x');",
        )?;
        drop(connection);
        // An interface snapshot that knows nothing about the task, saved twice.
        let snapshot = json!({"assistants": [], "providers": [], "models": [], "conversations": [], "memories": [], "permissions": [], "settings": {}});
        save_app_data_with_paths(&path, &snapshot, &HashMap::new())?;
        save_app_data_with_paths(&path, &snapshot, &HashMap::new())?;
        let connection = open(&path)?;
        let status: String = connection.query_row(
            "SELECT status FROM approvals WHERE id = 'approval-1'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(status, "pending");
        let tasks: i64 =
            connection.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))?;
        assert_eq!(tasks, 1);
        drop(connection);
        std::fs::remove_file(path).ok();
        Ok(())
    }
    /// Upgrades a *copy* of a real database (path in JUNIPER_UPGRADE_DB) and
    /// checks that the stored state survives unchanged. The original file is
    /// only read.
    #[test]
    #[ignore = "needs a real database copy; run deliberately"]
    fn live_upgrade_of_a_real_database_copy() -> Result<()> {
        let source = std::path::PathBuf::from(
            std::env::var("JUNIPER_UPGRADE_DB").expect("JUNIPER_UPGRADE_DB"),
        );
        let root = std::env::temp_dir().join(format!("juniper-upgrade-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("directory");
        let copy = root.join("juniper.db");
        std::fs::copy(&source, &copy).expect("copy");
        let before_version = existing_version(&copy)?;
        let before: Option<Value> = {
            let connection = Connection::open(&copy)?;
            connection
                .query_row(
                    "SELECT payload FROM app_state WHERE id = 'singleton'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .map(|text| serde_json::from_str(&text).expect("json"))
        };
        initialize(&copy)?;
        let after = load_app_data(&copy)?;
        let backup = migration_backup_path(&copy, before_version.unwrap_or(0));
        let count = |value: &Option<Value>, key: &str| {
            value
                .as_ref()
                .and_then(|value| value[key].as_array().map(Vec::len))
        };
        println!(
            "{}",
            json!({
                "fromVersion": before_version,
                "toVersion": existing_version(&copy)?,
                "backupKept": backup.is_file(),
                "assistants": [count(&before, "assistants"), count(&after, "assistants")],
                "conversations": [count(&before, "conversations"), count(&after, "conversations")],
                "memories": [count(&before, "memories"), count(&after, "memories")],
                "models": [count(&before, "models"), count(&after, "models")],
                "settingsEqual": before.as_ref().map(|value| &value["settings"]) == after.as_ref().map(|value| &value["settings"]),
            })
        );
        for key in [
            "assistants",
            "conversations",
            "memories",
            "models",
            "providers",
        ] {
            assert_eq!(count(&before, key), count(&after, key), "{key}");
        }
        // Restore rehearsal, as the phase report documents it: with Juniper
        // stopped, copy the backup over the database (setting the upgraded
        // file and any WAL/SHM sidecars aside first), then open it again.
        let upgraded = root.join("juniper.db.upgraded");
        std::fs::rename(&copy, &upgraded).expect("set the upgraded file aside");
        for sidecar in ["juniper.db-wal", "juniper.db-shm"] {
            let path = root.join(sidecar);
            if path.exists() {
                std::fs::rename(&path, root.join(format!("{sidecar}.upgraded")))
                    .expect("set a sidecar aside");
            }
        }
        std::fs::copy(&backup, &copy).expect("restore the backup");
        let integrity: String =
            Connection::open(&copy)?.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        let restored_version = existing_version(&copy)?;
        let restored: Option<Value> = Connection::open(&copy)?
            .query_row(
                "SELECT payload FROM app_state WHERE id = 'singleton'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|text| serde_json::from_str(&text).expect("json"));
        // Opening the restored file upgrades it again, as on the next launch.
        initialize(&copy)?;
        let reupgraded = load_app_data(&copy)?;
        println!(
            "{}",
            json!({
                "restore": {
                    "integrity": integrity,
                    "version": restored_version,
                    "payloadEqualsOriginal": restored == before,
                    "reupgradedVersion": existing_version(&copy)?,
                    "reupgradedConversations": count(&reupgraded, "conversations"),
                },
                "sourceCommit": option_env!("JUNIPER_LIVE_SOURCE_COMMIT"),
            })
        );
        assert_eq!(integrity, "ok");
        assert_eq!(restored_version, before_version);
        assert!(
            restored == before,
            "the restored payload differs from the original"
        );
        assert_eq!(
            count(&reupgraded, "conversations"),
            count(&before, "conversations")
        );
        std::fs::remove_dir_all(root).ok();
        Ok(())
    }
}
