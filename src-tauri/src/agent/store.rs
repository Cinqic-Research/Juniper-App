//! Native persistence for agent state (schema v5). Every task change is a
//! revision-checked write; a writer holding a stale revision is refused
//! instead of overwriting newer progress.

use super::types::*;
use crate::providers::TranscriptMessage;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Value, json};
use std::path::Path;

pub type StoreResult<T> = Result<T, String>;

pub fn db_error(error: impl std::fmt::Display) -> String {
    format!("AGENT_STORE_ERROR: {error}")
}

pub fn open(path: &Path) -> StoreResult<Connection> {
    crate::storage::open(path).map_err(db_error)
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

fn json_column<T: serde::de::DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn optional_json<T: serde::de::DeserializeOwned>(
    row: &Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<T>> {
    let text: Option<String> = row.get(index)?;
    text.map(|text| {
        serde_json::from_str(&text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    })
    .transpose()
}

fn encode<T: serde::Serialize>(value: &T) -> StoreResult<String> {
    serde_json::to_string(value).map_err(db_error)
}

const TASK_COLUMNS: &str = "id, parent_id, root_id, depth, role, assistant_id, project_id, schedule_run_id, title, goal, status, status_reason, current_action, plan, budget, usage, provenance, transcript, pending, result, revision, created_at, updated_at, finished_at";

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<Task> {
    let status: String = row.get(10)?;
    Ok(Task {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        root_id: row.get(2)?,
        depth: row.get(3)?,
        role: row.get(4)?,
        assistant_id: row.get(5)?,
        project_id: row.get(6)?,
        schedule_run_id: row.get(7)?,
        title: row.get(8)?,
        goal: row.get(9)?,
        status: TaskStatus::parse(&status).unwrap_or(TaskStatus::Failed),
        status_reason: row.get(11)?,
        current_action: row.get(12)?,
        plan: json_column(row, 13)?,
        budget: json_column(row, 14)?,
        usage: json_column(row, 15)?,
        provenance: json_column(row, 16)?,
        transcript: json_column(row, 17)?,
        pending: optional_json(row, 18)?,
        result: optional_json(row, 19)?,
        revision: row.get(20)?,
        created_at: row.get(21)?,
        updated_at: row.get(22)?,
        finished_at: row.get(23)?,
    })
}

pub fn insert_task(connection: &Connection, task: &Task) -> StoreResult<()> {
    connection
        .execute(
            &format!("INSERT INTO tasks({TASK_COLUMNS}) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)"),
            params![
                task.id,
                task.parent_id,
                task.root_id,
                task.depth,
                task.role,
                task.assistant_id,
                task.project_id,
                task.schedule_run_id,
                task.title,
                task.goal,
                task.status.as_str(),
                task.status_reason,
                task.current_action,
                encode(&task.plan)?,
                encode(&task.budget)?,
                encode(&task.usage)?,
                encode(&task.provenance)?,
                encode(&task.transcript)?,
                task.pending.as_ref().map(encode).transpose()?,
                task.result.as_ref().map(encode).transpose()?,
                task.revision,
                task.created_at,
                task.updated_at,
                task.finished_at,
            ],
        )
        .map_err(db_error)?;
    Ok(())
}

pub fn load_task(connection: &Connection, id: &str) -> StoreResult<Task> {
    connection
        .query_row(
            &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
            [id],
            task_from_row,
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "TASK_NOT_FOUND: That task no longer exists.".into())
}

/// Writes every mutable field if the stored revision still matches the one
/// this task was loaded at, then advances it.
pub fn save_task(connection: &Connection, task: &mut Task) -> StoreResult<()> {
    let updated_at = now();
    let changed = connection
        .execute(
            "UPDATE tasks SET status = ?1, status_reason = ?2, current_action = ?3, plan = ?4, budget = ?5, usage = ?6, provenance = ?7, transcript = ?8, pending = ?9, result = ?10, title = ?11, goal = ?12, finished_at = ?13, updated_at = ?14, revision = revision + 1 WHERE id = ?15 AND revision = ?16",
            params![
                task.status.as_str(),
                task.status_reason,
                task.current_action,
                encode(&task.plan)?,
                encode(&task.budget)?,
                encode(&task.usage)?,
                encode(&task.provenance)?,
                encode(&task.transcript)?,
                task.pending.as_ref().map(encode).transpose()?,
                task.result.as_ref().map(encode).transpose()?,
                task.title,
                task.goal,
                task.finished_at,
                updated_at,
                task.id,
                task.revision,
            ],
        )
        .map_err(db_error)?;
    if changed != 1 {
        return Err("TASK_REVISION_CONFLICT: The task changed since it was read.".into());
    }
    task.revision += 1;
    task.updated_at = updated_at;
    Ok(())
}

/// Moves a task to `next` if the transition model allows it, recording the
/// change as an event.
pub fn transition(
    connection: &Connection,
    task: &mut Task,
    next: TaskStatus,
    reason: Option<String>,
) -> StoreResult<()> {
    if task.status != next && !task.status.can_become(next) {
        return Err(format!(
            "TASK_TRANSITION_REFUSED: A {} task cannot become {}.",
            task.status.as_str(),
            next.as_str()
        ));
    }
    let previous = task.status;
    task.status = next;
    task.status_reason = reason.clone();
    if next.is_terminal() {
        task.finished_at = Some(now());
        task.current_action = None;
    } else if previous.is_terminal() {
        task.finished_at = None;
    }
    save_task(connection, task)?;
    if previous != next {
        add_event(
            connection,
            &task.id,
            "status",
            &match &reason {
                Some(reason) => format!("{} — {reason}", status_label(next)),
                None => status_label(next).to_owned(),
            },
            Some(json!({ "from": previous.as_str(), "to": next.as_str() })),
        )?;
    }
    Ok(())
}

pub fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Queued => "Queued",
        TaskStatus::Running => "Running",
        TaskStatus::WaitingInput => "Waiting for your answer",
        TaskStatus::WaitingApproval => "Waiting for your approval",
        TaskStatus::Paused => "Paused",
        TaskStatus::Interrupted => "Interrupted",
        TaskStatus::Completed => "Completed",
        TaskStatus::Failed => "Failed",
        TaskStatus::Cancelled => "Cancelled",
    }
}

pub fn list_tasks(
    connection: &Connection,
    include_children: bool,
    limit: u32,
) -> StoreResult<Vec<Task>> {
    let sql = format!(
        "SELECT {TASK_COLUMNS} FROM tasks {} ORDER BY updated_at DESC LIMIT ?1",
        if include_children {
            ""
        } else {
            "WHERE parent_id IS NULL"
        }
    );
    let mut statement = connection.prepare(&sql).map_err(db_error)?;
    statement
        .query_map([limit], task_from_row)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

pub fn children(connection: &Connection, parent_id: &str) -> StoreResult<Vec<Task>> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE parent_id = ?1 ORDER BY created_at"
        ))
        .map_err(db_error)?;
    statement
        .query_map([parent_id], task_from_row)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

/// All descendants of a task, depth first.
pub fn descendants(connection: &Connection, root: &str) -> StoreResult<Vec<Task>> {
    let mut found = Vec::new();
    let mut frontier = vec![root.to_owned()];
    while let Some(id) = frontier.pop() {
        for child in children(connection, &id)? {
            frontier.push(child.id.clone());
            found.push(child);
        }
    }
    Ok(found)
}

/// Queued tasks in run order: deeper tasks first so children finish before
/// their parents resume, then oldest first.
pub fn runnable_tasks(connection: &Connection) -> StoreResult<Vec<Task>> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE status = 'queued' ORDER BY depth DESC, created_at ASC LIMIT 64"
        ))
        .map_err(db_error)?;
    statement
        .query_map([], task_from_row)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

pub fn add_event(
    connection: &Connection,
    task_id: &str,
    kind: &str,
    summary: &str,
    detail: Option<Value>,
) -> StoreResult<()> {
    let summary: String = summary.chars().take(500).collect();
    connection
        .execute(
            "INSERT INTO task_events(task_id, kind, summary, detail, created_at) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![
                task_id,
                kind,
                summary,
                detail.map(|value| value.to_string()),
                now()
            ],
        )
        .map_err(db_error)?;
    Ok(())
}

pub fn events(connection: &Connection, task_id: &str, limit: u32) -> StoreResult<Vec<TaskEvent>> {
    let mut statement = connection
        .prepare("SELECT id, task_id, kind, summary, detail, created_at FROM (SELECT * FROM task_events WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2) ORDER BY id")
        .map_err(db_error)?;
    statement
        .query_map(params![task_id, limit], |row| {
            Ok(TaskEvent {
                id: row.get(0)?,
                task_id: row.get(1)?,
                kind: row.get(2)?,
                summary: row.get(3)?,
                detail: optional_json(row, 4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

// ---------------------------------------------------------------- actions

pub fn insert_action(connection: &Connection, action: &TaskAction) -> StoreResult<()> {
    connection
        .execute(
            "INSERT INTO task_actions(id, task_id, step, call_id, tool, arguments, binding, status, result, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                action.id,
                action.task_id,
                action.step,
                action.call_id,
                action.tool,
                action.arguments.to_string(),
                action.binding,
                action.status,
                action.result.as_ref().map(Value::to_string),
                action.created_at,
                action.updated_at,
            ],
        )
        .map_err(|error| {
            if error.to_string().contains("UNIQUE") {
                "DUPLICATE_CALL_ID: This tool call reused an earlier call ID.".to_owned()
            } else {
                db_error(error)
            }
        })?;
    Ok(())
}

pub fn set_action(
    connection: &Connection,
    action_id: &str,
    status: &str,
    result: Option<&Value>,
) -> StoreResult<()> {
    connection
        .execute(
            "UPDATE task_actions SET status = ?1, result = COALESCE(?2, result), updated_at = ?3 WHERE id = ?4",
            params![status, result.map(Value::to_string), now(), action_id],
        )
        .map_err(db_error)?;
    Ok(())
}

fn action_from_row(row: &Row<'_>) -> rusqlite::Result<TaskAction> {
    Ok(TaskAction {
        id: row.get(0)?,
        task_id: row.get(1)?,
        step: row.get(2)?,
        call_id: row.get(3)?,
        tool: row.get(4)?,
        arguments: json_column(row, 5)?,
        binding: row.get(6)?,
        status: row.get(7)?,
        result: optional_json(row, 8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

const ACTION_COLUMNS: &str =
    "id, task_id, step, call_id, tool, arguments, binding, status, result, created_at, updated_at";

pub fn actions(connection: &Connection, task_id: &str) -> StoreResult<Vec<TaskAction>> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {ACTION_COLUMNS} FROM task_actions WHERE task_id = ?1 ORDER BY created_at"
        ))
        .map_err(db_error)?;
    statement
        .query_map([task_id], action_from_row)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

pub fn action_for_call(
    connection: &Connection,
    task_id: &str,
    call_id: &str,
) -> StoreResult<Option<TaskAction>> {
    connection
        .query_row(
            &format!(
                "SELECT {ACTION_COLUMNS} FROM task_actions WHERE task_id = ?1 AND call_id = ?2"
            ),
            params![task_id, call_id],
            action_from_row,
        )
        .optional()
        .map_err(db_error)
}

// ---------------------------------------------------------------- approvals

fn approval_from_row(row: &Row<'_>) -> rusqlite::Result<Approval> {
    Ok(Approval {
        id: row.get(0)?,
        task_id: row.get(1)?,
        action_id: row.get(2)?,
        tool: row.get(3)?,
        capability: row.get(4)?,
        preview: row.get(5)?,
        binding: row.get(6)?,
        standing_allowed: row.get(7)?,
        status: row.get(8)?,
        decision: row.get(9)?,
        created_at: row.get(10)?,
        decided_at: row.get(11)?,
        expires_at: row.get(12)?,
    })
}

const APPROVAL_COLUMNS: &str = "id, task_id, action_id, tool, capability, preview, binding, standing_allowed, status, decision, created_at, decided_at, expires_at";

pub fn insert_approval(connection: &Connection, approval: &Approval) -> StoreResult<()> {
    connection
        .execute(
            &format!("INSERT INTO approvals({APPROVAL_COLUMNS}) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"),
            params![
                approval.id,
                approval.task_id,
                approval.action_id,
                approval.tool,
                approval.capability,
                approval.preview,
                approval.binding,
                approval.standing_allowed,
                approval.status,
                approval.decision,
                approval.created_at,
                approval.decided_at,
                approval.expires_at,
            ],
        )
        .map_err(db_error)?;
    Ok(())
}

pub fn approval(connection: &Connection, id: &str) -> StoreResult<Approval> {
    connection
        .query_row(
            &format!("SELECT {APPROVAL_COLUMNS} FROM approvals WHERE id = ?1"),
            [id],
            approval_from_row,
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "APPROVAL_NOT_FOUND: That approval request no longer exists.".into())
}

pub fn approvals_for_task(connection: &Connection, task_id: &str) -> StoreResult<Vec<Approval>> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE task_id = ?1 ORDER BY created_at"
        ))
        .map_err(db_error)?;
    statement
        .query_map([task_id], approval_from_row)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

pub fn pending_approvals(connection: &Connection) -> StoreResult<Vec<Approval>> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE status = 'pending' ORDER BY created_at"
        ))
        .map_err(db_error)?;
    statement
        .query_map([], approval_from_row)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

/// Records a decision on a still-pending approval. A decided, expired, or
/// stale approval cannot be decided again.
pub fn decide_approval(
    connection: &Connection,
    id: &str,
    status: &str,
    decision: &str,
) -> StoreResult<()> {
    let changed = connection
        .execute(
            "UPDATE approvals SET status = ?1, decision = ?2, decided_at = ?3 WHERE id = ?4 AND status = 'pending'",
            params![status, decision, now(), id],
        )
        .map_err(db_error)?;
    if changed != 1 {
        return Err("APPROVAL_NOT_PENDING: This approval request is no longer pending.".into());
    }
    Ok(())
}

// ---------------------------------------------------------------- grants

pub fn grants(connection: &Connection, include_revoked: bool) -> StoreResult<Vec<Grant>> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT id, assistant_id, project_id, capability, scope, created_at, expires_at, revoked_at FROM grants {} ORDER BY created_at DESC",
            if include_revoked { "" } else { "WHERE revoked_at IS NULL" }
        ))
        .map_err(db_error)?;
    statement
        .query_map([], |row| {
            Ok(Grant {
                id: row.get(0)?,
                assistant_id: row.get(1)?,
                project_id: row.get(2)?,
                capability: row.get(3)?,
                scope: json_column(row, 4)?,
                created_at: row.get(5)?,
                expires_at: row.get(6)?,
                revoked_at: row.get(7)?,
            })
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

pub fn insert_grant(connection: &Connection, grant: &Grant) -> StoreResult<()> {
    connection
        .execute(
            "INSERT INTO grants(id, assistant_id, project_id, capability, scope, created_at, expires_at, revoked_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                grant.id,
                grant.assistant_id,
                grant.project_id,
                grant.capability,
                grant.scope.to_string(),
                grant.created_at,
                grant.expires_at,
            ],
        )
        .map_err(db_error)?;
    Ok(())
}

pub fn revoke_grant(connection: &Connection, id: &str) -> StoreResult<()> {
    connection
        .execute(
            "UPDATE grants SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
            params![now(), id],
        )
        .map_err(db_error)?;
    Ok(())
}

// ---------------------------------------------------------------- artifacts and sources

pub fn insert_artifact(connection: &Connection, artifact: &Artifact) -> StoreResult<()> {
    connection
        .execute(
            "INSERT INTO artifacts(id, task_id, project_id, kind, title, file_name, media_type, size_bytes, sha256, source_ids, status, created_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                artifact.id,
                artifact.task_id,
                artifact.project_id,
                artifact.kind,
                artifact.title,
                artifact.file_name,
                artifact.media_type,
                artifact.size_bytes,
                artifact.sha256,
                encode(&artifact.source_ids)?,
                artifact.status,
                artifact.created_at,
            ],
        )
        .map_err(db_error)?;
    Ok(())
}

fn artifact_from_row(row: &Row<'_>) -> rusqlite::Result<Artifact> {
    Ok(Artifact {
        id: row.get(0)?,
        task_id: row.get(1)?,
        project_id: row.get(2)?,
        kind: row.get(3)?,
        title: row.get(4)?,
        file_name: row.get(5)?,
        media_type: row.get(6)?,
        size_bytes: row.get(7)?,
        sha256: row.get(8)?,
        source_ids: json_column(row, 9)?,
        status: row.get(10)?,
        created_at: row.get(11)?,
    })
}

const ARTIFACT_COLUMNS: &str = "id, task_id, project_id, kind, title, file_name, media_type, size_bytes, sha256, source_ids, status, created_at";

pub fn artifacts_for_tasks(
    connection: &Connection,
    task_ids: &[String],
) -> StoreResult<Vec<Artifact>> {
    let mut found = Vec::new();
    let mut statement = connection
        .prepare(&format!(
            "SELECT {ARTIFACT_COLUMNS} FROM artifacts WHERE task_id = ?1 ORDER BY created_at"
        ))
        .map_err(db_error)?;
    for id in task_ids {
        found.extend(
            statement
                .query_map([id], artifact_from_row)
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?,
        );
    }
    Ok(found)
}

pub fn artifact(connection: &Connection, id: &str) -> StoreResult<Artifact> {
    connection
        .query_row(
            &format!("SELECT {ARTIFACT_COLUMNS} FROM artifacts WHERE id = ?1"),
            [id],
            artifact_from_row,
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "ARTIFACT_NOT_FOUND: That artifact no longer exists.".into())
}

pub fn insert_source(connection: &Connection, source: &Source) -> StoreResult<()> {
    connection
        .execute(
            "INSERT INTO sources(id, task_id, kind, title, url, final_url, location, retrieved_at, content_sha256, excerpt) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                source.id,
                source.task_id,
                source.kind,
                source.title,
                source.url,
                source.final_url,
                source.location,
                source.retrieved_at,
                source.content_sha256,
                source.excerpt,
            ],
        )
        .map_err(db_error)?;
    Ok(())
}

pub fn sources_for_tasks(connection: &Connection, task_ids: &[String]) -> StoreResult<Vec<Source>> {
    let mut found = Vec::new();
    let mut statement = connection
        .prepare("SELECT id, task_id, kind, title, url, final_url, location, retrieved_at, content_sha256, excerpt FROM sources WHERE task_id = ?1 ORDER BY retrieved_at")
        .map_err(db_error)?;
    for id in task_ids {
        found.extend(
            statement
                .query_map([id], |row| {
                    Ok(Source {
                        id: row.get(0)?,
                        task_id: row.get(1)?,
                        kind: row.get(2)?,
                        title: row.get(3)?,
                        url: row.get(4)?,
                        final_url: row.get(5)?,
                        location: row.get(6)?,
                        retrieved_at: row.get(7)?,
                        content_sha256: row.get(8)?,
                        excerpt: row.get(9)?,
                    })
                })
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?,
        );
    }
    Ok(found)
}

// ---------------------------------------------------------------- profiles and settings

pub fn agent_profile(
    connection: &Connection,
    assistant_id: &str,
) -> StoreResult<Option<(AgentProfile, i64)>> {
    connection
        .query_row(
            "SELECT payload, revision FROM agent_profiles WHERE assistant_id = ?1",
            [assistant_id],
            |row| Ok((json_column::<AgentProfile>(row, 0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(db_error)
}

pub fn agent_profiles(connection: &Connection) -> StoreResult<Vec<(AgentProfile, i64)>> {
    let mut statement = connection
        .prepare("SELECT payload, revision FROM agent_profiles ORDER BY updated_at DESC")
        .map_err(db_error)?;
    statement
        .query_map([], |row| {
            Ok((json_column::<AgentProfile>(row, 0)?, row.get(1)?))
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

/// Saves a profile when `expected_revision` matches the stored one (or the
/// profile is new and `expected_revision` is 0). Returns the new revision.
pub fn save_agent_profile(
    connection: &Connection,
    profile: &AgentProfile,
    expected_revision: i64,
) -> StoreResult<i64> {
    let payload = encode(profile)?;
    let changed = if expected_revision == 0 {
        connection
            .execute(
                "INSERT OR IGNORE INTO agent_profiles(assistant_id, payload, revision, updated_at) VALUES(?1, ?2, 1, ?3)",
                params![profile.assistant_id, payload, now()],
            )
            .map_err(db_error)?
    } else {
        connection
            .execute(
                "UPDATE agent_profiles SET payload = ?1, revision = revision + 1, updated_at = ?2 WHERE assistant_id = ?3 AND revision = ?4",
                params![payload, now(), profile.assistant_id, expected_revision],
            )
            .map_err(db_error)?
    };
    if changed != 1 {
        return Err(
            "PROFILE_REVISION_CONFLICT: This agent changed elsewhere. Reload and try again.".into(),
        );
    }
    Ok(expected_revision + 1)
}

pub fn agent_settings(connection: &Connection) -> StoreResult<Value> {
    Ok(connection
        .query_row(
            "SELECT payload FROM agent_settings WHERE id = 'singleton'",
            [],
            |row| json_column::<Value>(row, 0),
        )
        .optional()
        .map_err(db_error)?
        .unwrap_or_else(|| json!({})))
}

pub fn set_agent_settings(connection: &Connection, payload: &Value) -> StoreResult<()> {
    connection
        .execute(
            "INSERT INTO agent_settings(id, payload, updated_at) VALUES('singleton', ?1, ?2) ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
            params![payload.to_string(), now()],
        )
        .map_err(db_error)?;
    Ok(())
}

/// The interface's persisted snapshot (assistants, models, providers,
/// memories, settings). The agent runtime reads configuration from it and
/// never writes it.
pub fn app_snapshot(connection: &Connection) -> StoreResult<Value> {
    Ok(connection
        .query_row(
            "SELECT payload FROM app_state WHERE id = 'singleton'",
            [],
            |row| json_column::<Value>(row, 0),
        )
        .optional()
        .map_err(db_error)?
        .unwrap_or_else(|| json!({})))
}

// ---------------------------------------------------------------- projects

pub fn projects(connection: &Connection, include_archived: bool) -> StoreResult<Vec<Project>> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT id, name, instructions, archived, revision, created_at, updated_at FROM projects {} ORDER BY updated_at DESC",
            if include_archived { "" } else { "WHERE archived = 0" }
        ))
        .map_err(db_error)?;
    let mut projects = statement
        .query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                instructions: row.get(2)?,
                archived: row.get(3)?,
                roots: Vec::new(),
                revision: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    for project in &mut projects {
        project.roots = roots(connection, &project.id, false)?;
    }
    Ok(projects)
}

pub fn project(connection: &Connection, id: &str) -> StoreResult<Project> {
    projects(connection, true)?
        .into_iter()
        .find(|project| project.id == id)
        .ok_or_else(|| "PROJECT_NOT_FOUND: That project no longer exists.".into())
}

pub fn roots(
    connection: &Connection,
    project_id: &str,
    include_revoked: bool,
) -> StoreResult<Vec<ProjectRoot>> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT id, project_id, label, path, access, created_at, revoked_at FROM project_roots WHERE project_id = ?1 {} ORDER BY created_at",
            if include_revoked { "" } else { "AND revoked_at IS NULL" }
        ))
        .map_err(db_error)?;
    statement
        .query_map([project_id], |row| {
            Ok(ProjectRoot {
                id: row.get(0)?,
                project_id: row.get(1)?,
                label: row.get(2)?,
                path: row.get(3)?,
                access: row.get(4)?,
                created_at: row.get(5)?,
                revoked_at: row.get(6)?,
            })
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

pub fn root(connection: &Connection, id: &str) -> StoreResult<ProjectRoot> {
    connection
        .query_row(
            "SELECT id, project_id, label, path, access, created_at, revoked_at FROM project_roots WHERE id = ?1 AND revoked_at IS NULL",
            [id],
            |row| {
                Ok(ProjectRoot {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    label: row.get(2)?,
                    path: row.get(3)?,
                    access: row.get(4)?,
                    created_at: row.get(5)?,
                    revoked_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "ROOT_NOT_GRANTED: That folder is not granted to this project.".into())
}

// ---------------------------------------------------------------- inbox and memory outbox

pub fn add_inbox(
    connection: &Connection,
    task_id: Option<&str>,
    kind: &str,
    title: &str,
    body: &str,
) -> StoreResult<()> {
    connection
        .execute(
            "INSERT INTO inbox(task_id, kind, title, body, read, created_at) VALUES(?1, ?2, ?3, ?4, 0, ?5)",
            params![task_id, kind, title, body.chars().take(2000).collect::<String>(), now()],
        )
        .map_err(db_error)?;
    Ok(())
}

pub fn inbox(connection: &Connection, limit: u32) -> StoreResult<Vec<InboxItem>> {
    let mut statement = connection
        .prepare("SELECT id, task_id, kind, title, body, read, created_at FROM inbox ORDER BY id DESC LIMIT ?1")
        .map_err(db_error)?;
    statement
        .query_map([limit], |row| {
            Ok(InboxItem {
                id: row.get(0)?,
                task_id: row.get(1)?,
                kind: row.get(2)?,
                title: row.get(3)?,
                body: row.get(4)?,
                read: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

pub fn mark_inbox_read(connection: &Connection, ids: &[i64]) -> StoreResult<()> {
    for id in ids {
        connection
            .execute("UPDATE inbox SET read = 1 WHERE id = ?1", [id])
            .map_err(db_error)?;
    }
    Ok(())
}

pub fn queue_memory_change(
    connection: &Connection,
    assistant_id: &str,
    operation: &str,
    payload: &Value,
    task_id: &str,
) -> StoreResult<String> {
    let id = new_id("memory-change");
    connection
        .execute(
            "INSERT INTO memory_outbox(id, assistant_id, operation, payload, task_id, created_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, assistant_id, operation, payload.to_string(), task_id, now()],
        )
        .map_err(db_error)?;
    Ok(id)
}

pub fn memory_outbox(connection: &Connection) -> StoreResult<Vec<Value>> {
    let mut statement = connection
        .prepare("SELECT id, assistant_id, operation, payload, created_at FROM memory_outbox ORDER BY created_at")
        .map_err(db_error)?;
    statement
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "assistantId": row.get::<_, String>(1)?,
                "operation": row.get::<_, String>(2)?,
                "payload": json_column::<Value>(row, 3)?,
                "createdAt": row.get::<_, String>(4)?,
            }))
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

pub fn acknowledge_memory_changes(connection: &Connection, ids: &[String]) -> StoreResult<()> {
    for id in ids {
        connection
            .execute("DELETE FROM memory_outbox WHERE id = ?1", [id])
            .map_err(db_error)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- recovery

/// After a restart nothing is executing: running tasks become interrupted,
/// and an action that had started may or may not have taken effect, so it
/// is marked uncertain for reconciliation rather than retried blindly.
/// Returns the number of interrupted tasks.
pub fn recover_after_restart(connection: &Connection) -> StoreResult<usize> {
    let uncertain = connection
        .execute(
            "UPDATE task_actions SET status = 'uncertain', updated_at = ?1 WHERE status = 'started'",
            [now()],
        )
        .map_err(db_error)?;
    let running = {
        let mut statement = connection
            .prepare("SELECT id FROM tasks WHERE status = 'running'")
            .map_err(db_error)?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?
    };
    for id in &running {
        let mut task = load_task(connection, id)?;
        transition(
            connection,
            &mut task,
            TaskStatus::Interrupted,
            Some(
                "Juniper closed while this task was running. Resume it when you are ready.".into(),
            ),
        )?;
    }
    if uncertain > 0 {
        add_inbox_note(connection, uncertain)?;
    }
    Ok(running.len())
}

fn add_inbox_note(connection: &Connection, uncertain: usize) -> StoreResult<()> {
    add_inbox(
        connection,
        None,
        "recovery",
        "Some actions need checking",
        &format!(
            "{uncertain} action(s) were in progress when Juniper closed. Juniper will check their outcome before repeating them."
        ),
    )
}

/// The user-visible part of a transcript: goals, follow-ups, questions, and
/// final answers, without tool traffic.
pub fn visible_messages(transcript: &[TranscriptMessage]) -> Vec<TranscriptMessage> {
    transcript
        .iter()
        .filter(|message| {
            (message.role == "user" || message.role == "assistant")
                && message.tool_calls.is_empty()
                && !message.content.trim().is_empty()
        })
        .cloned()
        .collect()
}
