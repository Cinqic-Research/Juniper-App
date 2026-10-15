//! IPC commands for the Agent screen. The interface sends intents (start,
//! steer, answer, approve, pause, cancel); the host validates each against
//! stored state and the transition model before changing anything.

use super::policy;
use super::runner;
use super::store;
use super::types::*;
use super::{AgentState, Intent};
use crate::commands::AppState;
use crate::providers::TranscriptMessage;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;

fn bounded(text: &str, limit: usize) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("INVALID_INPUT: Enter some text.".into());
    }
    if trimmed.chars().count() > limit || trimmed.contains('\0') {
        return Err(format!("INVALID_INPUT: Keep it under {limit} characters."));
    }
    Ok(trimmed.to_owned())
}

fn valid_id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 200 || value.chars().any(char::is_control) {
        return Err("INVALID_INPUT: Invalid identifier.".into());
    }
    Ok(())
}

fn assistant_exists(connection: &Connection, assistant_id: &str) -> Result<(), String> {
    let snapshot = store::app_snapshot(connection)?;
    snapshot["assistants"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|assistant| assistant["id"].as_str() == Some(assistant_id))
        .then_some(())
        .ok_or_else(|| "AGENT_NOT_FOUND: That agent does not exist.".into())
}

#[tauri::command]
pub fn agent_overview(state: State<'_, AppState>) -> Result<Value, String> {
    let agent = &state.agent;
    let connection = agent.connection()?;
    let profiles: Vec<Value> = store::agent_profiles(&connection)?
        .into_iter()
        .map(|(profile, revision)| json!({ "profile": profile, "revision": revision }))
        .collect();
    let inbox = store::inbox(&connection, 50)?;
    let presets: Vec<Value> = ["ask", "project", "custom"]
        .iter()
        .map(|preset| json!({ "id": preset, "description": policy::preset_description(preset) }))
        .collect();
    Ok(json!({
        "profiles": profiles,
        "projects": store::projects(&connection, true)?,
        "tasks": store::list_tasks(&connection, false, 100)?,
        "pendingApprovals": store::pending_approvals(&connection)?,
        "inbox": inbox,
        "stopped": agent.is_stopped(),
        "timeZone": super::context::timezone(&connection),
        "settings": store::agent_settings(&connection)?,
        "capabilities": capabilities(),
        "presets": presets,
    }))
}

/// What this platform can do, with reasons for what it cannot.
fn capabilities() -> Value {
    let reason = |result: Result<(), String>| match result {
        Ok(()) => json!({ "available": true }),
        Err(reason) => json!({ "available": false, "reason": reason }),
    };
    json!({
        "workspace": reason(if cfg!(any(target_os = "android", target_os = "ios")) { Err("Project folders are available on desktop.".into()) } else { Ok(()) }),
        "web": reason(super::web::availability()),
        "sandbox": reason(super::sandbox::availability()),
        "mcpStdio": reason(super::mcp::stdio_availability()),
        "mcpHttp": json!({ "available": true }),
        "scheduling": json!({ "available": true, "note": "Scheduled tasks run while Juniper is running. A run missed while Juniper was closed runs once at the next launch." }),
    })
}

#[tauri::command]
pub fn agent_tools(state: State<'_, AppState>, assistant_id: String) -> Result<Value, String> {
    valid_id(&assistant_id)?;
    let connection = state.agent.connection()?;
    let profile = store::agent_profile(&connection, &assistant_id)?
        .map(|(profile, _)| profile)
        .unwrap_or_else(|| AgentProfile::default_for(&assistant_id));
    let mut tools: Vec<Value> = policy::builtin_tools()
        .into_iter()
        .filter(|tool| !policy::CONTROL_TOOLS.contains(&tool.name.as_str()))
        .map(|tool| {
            let availability = match tool.name.as_str() {
                "web.search" | "web.fetch" => super::web::availability(),
                "code.python" | "code.run" => super::sandbox::availability(),
                _ => Ok(()),
            };
            json!({
                "name": tool.name,
                "displayName": tool.display_name,
                "description": tool.description,
                "risk": tool.risk,
                "source": tool.source,
                "enabled": profile.enabled_tools.contains(&tool.name),
                "available": availability.is_ok(),
                "reason": availability.err(),
            })
        })
        .collect();
    for tool in super::mcp::all_tools(&connection)? {
        tools.push(json!({
            "name": tool.name,
            "displayName": tool.display_name,
            "description": tool.description,
            "risk": tool.risk,
            "source": tool.source,
            "enabled": profile.enabled_tools.contains(&tool.name),
            "available": true,
            "reason": Value::Null,
        }));
    }
    Ok(json!({ "tools": tools }))
}

#[tauri::command]
pub fn agent_profile_get(
    state: State<'_, AppState>,
    assistant_id: String,
) -> Result<Value, String> {
    valid_id(&assistant_id)?;
    let connection = state.agent.connection()?;
    let (profile, revision) = store::agent_profile(&connection, &assistant_id)?
        .unwrap_or_else(|| (AgentProfile::default_for(&assistant_id), 0));
    Ok(json!({ "profile": profile, "revision": revision }))
}

/// Validates and stores an agent profile. Unknown tools and skills are
/// dropped; budgets are clamped to the host's ceilings.
pub fn sanitize_profile(
    connection: &Connection,
    mut profile: AgentProfile,
) -> Result<AgentProfile, String> {
    if profile.schema_version != AGENT_PROFILE_SCHEMA {
        return Err(
            "PROFILE_SCHEMA_UNSUPPORTED: This agent profile version is not supported.".into(),
        );
    }
    if !matches!(profile.autonomy.as_str(), "ask" | "project" | "custom") {
        profile.autonomy = "ask".into();
    }
    let known: Vec<String> = policy::builtin_tools()
        .into_iter()
        .map(|tool| tool.name)
        .chain(
            super::mcp::all_tools(connection)?
                .into_iter()
                .map(|tool| tool.name),
        )
        .collect();
    profile.enabled_tools.retain(|tool| known.contains(tool));
    profile.enabled_tools.dedup();
    let skills: Vec<String> = super::skills::list(connection)?
        .into_iter()
        .map(|skill| skill.id)
        .collect();
    profile
        .enabled_skills
        .retain(|skill| skills.contains(skill));
    let projects: Vec<String> = store::projects(connection, true)?
        .into_iter()
        .map(|project| project.id)
        .collect();
    profile
        .project_ids
        .retain(|project| projects.contains(project));
    profile.budget = profile.budget.bounded();
    profile.preferred_language = profile
        .preferred_language
        .map(|language| language.trim().chars().take(40).collect::<String>())
        .filter(|language| !language.is_empty());
    profile.composed_profile = profile.composed_profile.chars().take(8000).collect();
    Ok(profile)
}

#[tauri::command]
pub fn agent_profile_save(
    state: State<'_, AppState>,
    profile: AgentProfile,
    expected_revision: i64,
) -> Result<i64, String> {
    valid_id(&profile.assistant_id)?;
    let _route_configuration = state.agent.lock_route_configuration();
    let connection = state.agent.connection()?;
    assistant_exists(&connection, &profile.assistant_id)?;
    let profile = sanitize_profile(&connection, profile)?;
    let current_revision = store::agent_profile(&connection, &profile.assistant_id)?
        .map_or(0, |(_, revision)| revision);
    if current_revision != expected_revision {
        return Err(
            "PROFILE_REVISION_CONFLICT: This agent changed elsewhere. Reload and try again.".into(),
        );
    }
    // A resumed task resolves its profile again, so require an informed
    // continuation before committing the profile change. If pausing fails,
    // keep the prior profile active.
    super::context::pause_tasks_for_profile_change(
        &state.agent,
        &connection,
        &profile.assistant_id,
    )?;
    store::save_agent_profile(&connection, &profile, expected_revision)
}

/// An agent profile for export: no grants exist on a profile, and project
/// links and the model choice are machine-specific, so they are left out.
#[tauri::command]
pub fn agent_profile_export(
    state: State<'_, AppState>,
    assistant_id: String,
) -> Result<Value, String> {
    valid_id(&assistant_id)?;
    let connection = state.agent.connection()?;
    let mut profile = store::agent_profile(&connection, &assistant_id)?
        .map(|(profile, _)| profile)
        .unwrap_or_else(|| AgentProfile::default_for(&assistant_id));
    profile.project_ids.clear();
    profile.model_profile_id = None;
    profile.allow_unqualified_model = false;
    Ok(
        json!({ "format": "juniper-agent-profile", "schemaVersion": AGENT_PROFILE_SCHEMA, "profile": profile }),
    )
}

/// Imports a profile for an assistant the interface just created. Imported
/// configuration starts inert: it asks before every action, uses only local
/// default tools, and is linked to no project or model.
#[tauri::command]
pub fn agent_profile_import(
    state: State<'_, AppState>,
    assistant_id: String,
    document: Value,
) -> Result<i64, String> {
    valid_id(&assistant_id)?;
    if document["format"] != "juniper-agent-profile" {
        return Err("IMPORT_INVALID: This is not a Juniper agent profile.".into());
    }
    let mut profile: AgentProfile = serde_json::from_value(document["profile"].clone())
        .map_err(|_| "IMPORT_INVALID: The agent profile is malformed.".to_owned())?;
    profile.assistant_id = assistant_id;
    profile.autonomy = "ask".into();
    profile.project_ids.clear();
    profile.model_profile_id = None;
    profile.allow_unqualified_model = false;
    profile
        .enabled_tools
        .retain(|tool| policy::DEFAULT_TOOLS.contains(&tool.as_str()));
    let connection = state.agent.connection()?;
    assistant_exists(&connection, &profile.assistant_id)?;
    let profile = sanitize_profile(&connection, profile)?;
    let revision = store::agent_profile(&connection, &profile.assistant_id)?
        .map_or(0, |(_, revision)| revision);
    store::save_agent_profile(&connection, &profile, revision)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTask {
    pub assistant_id: String,
    pub goal: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

pub fn create_task(
    connection: &Connection,
    request: &StartTask,
    schedule_run_id: Option<String>,
) -> Result<Task, String> {
    valid_id(&request.assistant_id)?;
    let goal = bounded(&request.goal, 20_000)?;
    assistant_exists(connection, &request.assistant_id)?;
    if let Some(project) = &request.project_id {
        valid_id(project)?;
        let project = store::project(connection, project)?;
        if project.archived {
            return Err(
                "PROJECT_ARCHIVED: Restore this project before starting tasks in it.".into(),
            );
        }
    }
    let profile = store::agent_profile(connection, &request.assistant_id)?
        .map(|(profile, _)| profile)
        .unwrap_or_else(|| AgentProfile::default_for(&request.assistant_id));
    let now = store::now();
    let id = store::new_id("task");
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(|title| title.chars().take(100).collect::<String>())
        .unwrap_or_else(|| {
            let first_line = goal.lines().next().unwrap_or_default();
            let mut title: String = first_line.chars().take(80).collect();
            if first_line.chars().count() > 80 {
                title.push('…');
            }
            title
        });
    let task = Task {
        id: id.clone(),
        parent_id: None,
        root_id: id,
        depth: 0,
        role: None,
        assistant_id: request.assistant_id.clone(),
        project_id: request.project_id.clone(),
        schedule_run_id,
        title,
        goal: goal.clone(),
        status: TaskStatus::Queued,
        status_reason: None,
        current_action: Some("Waiting to start".into()),
        plan: Vec::new(),
        budget: profile.budget.bounded(),
        usage: TaskUsage::default(),
        provenance: TaskProvenance::default(),
        transcript: vec![TranscriptMessage {
            role: "user".into(),
            content: goal,
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }],
        pending: None,
        result: None,
        revision: 1,
        created_at: now.clone(),
        updated_at: now,
        finished_at: None,
    };
    store::insert_task(connection, &task)?;
    store::add_event(connection, &task.id, "status", "Queued", None)?;
    Ok(task)
}

#[tauri::command]
pub fn task_start<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    request: StartTask,
) -> Result<String, String> {
    let agent = &state.agent;
    let _route_configuration = agent.lock_route_configuration();
    if agent.is_stopped() {
        return Err(
            "AGENT_STOPPED: Agent work is stopped. Resume it before starting a task.".into(),
        );
    }
    let connection = agent.connection()?;
    let task = create_task(&connection, &request, None)?;
    agent.wake.notify_one();
    super::notify(&app, Some(&task.id), "task");
    Ok(task.id)
}

#[tauri::command]
pub fn task_detail(state: State<'_, AppState>, task_id: String) -> Result<TaskDetail, String> {
    valid_id(&task_id)?;
    let connection = state.agent.connection()?;
    let task = store::load_task(&connection, &task_id)?;
    let children = store::descendants(&connection, &task.id)?;
    let mut ids = vec![task.id.clone()];
    ids.extend(children.iter().map(|child| child.id.clone()));
    let mut actions = Vec::new();
    let mut approvals = Vec::new();
    for id in &ids {
        actions.extend(store::actions(&connection, id)?);
        approvals.extend(store::approvals_for_task(&connection, id)?);
    }
    Ok(TaskDetail {
        events: store::events(&connection, &task.id, 300)?,
        actions,
        approvals,
        artifacts: store::artifacts_for_tasks(&connection, &ids)?,
        sources: store::sources_for_tasks(&connection, &ids)?,
        messages: store::visible_messages(&task.transcript),
        children,
        task,
    })
}

#[tauri::command]
pub fn task_list(state: State<'_, AppState>, limit: Option<u32>) -> Result<Vec<Task>, String> {
    let connection = state.agent.connection()?;
    store::list_tasks(&connection, false, limit.unwrap_or(100).min(500))
}

fn with_task<R: Runtime>(
    app: &AppHandle<R>,
    agent: &AgentState,
    task_id: &str,
    change: impl FnOnce(&Connection, &mut Task) -> Result<(), String>,
) -> Result<(), String> {
    valid_id(task_id)?;
    let connection = agent.connection()?;
    let mut task = store::load_task(&connection, task_id)?;
    change(&connection, &mut task)?;
    agent.wake.notify_one();
    super::notify(app, Some(task_id), "task");
    Ok(())
}

#[tauri::command]
pub fn task_pause<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    let agent = &state.agent;
    let _route_configuration = agent.lock_route_configuration();
    if agent.signal(&task_id, Intent::Pause) {
        return Ok(());
    }
    with_task(&app, agent, &task_id, |connection, task| {
        store::transition(
            connection,
            task,
            TaskStatus::Paused,
            Some("Paused by you.".into()),
        )
    })
}

#[tauri::command]
pub fn task_resume<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    let agent = &state.agent;
    let _route_configuration = agent.lock_route_configuration();
    if agent.is_stopped() {
        return Err("AGENT_STOPPED: Agent work is stopped. Resume agent work first.".into());
    }
    with_task(&app, agent, &task_id, |connection, task| {
        if task.status == TaskStatus::Interrupted {
            reconcile_uncertain(connection, task)?;
        }
        if !matches!(task.status, TaskStatus::Paused | TaskStatus::Interrupted) {
            return Err(
                "TASK_NOT_RESUMABLE: Only paused or interrupted tasks can be resumed.".into(),
            );
        }
        task.usage.not_before = None;
        store::transition(
            connection,
            task,
            TaskStatus::Queued,
            Some("Resumed.".into()),
        )
    })
}

/// An action that was running when Juniper closed may or may not have taken
/// effect. Workspace writes are checked against the file; anything else is
/// reported to the model as uncertain, never silently repeated.
fn reconcile_uncertain(connection: &Connection, task: &mut Task) -> Result<(), String> {
    let Some(pending) = task.pending.clone() else {
        return Ok(());
    };
    let Some(call) = pending.calls.get(pending.next).cloned() else {
        return Ok(());
    };
    let Some(action) = store::action_for_call(connection, &task.id, &call.id)? else {
        return Ok(());
    };
    if action.status != "uncertain" {
        return Ok(());
    }
    let content_version = (call.name == "workspace.write").then(|| {
        super::workspace::version_of(
            call.arguments["content"]
                .as_str()
                .unwrap_or_default()
                .as_bytes(),
        )
    });
    let applied = match (&content_version, task.project_id.as_deref()) {
        (Some(expected), Some(_)) => {
            let root = store::root(
                connection,
                call.arguments["rootId"].as_str().unwrap_or_default(),
            )
            .ok();
            root.and_then(|root| super::workspace::open_root(&root.path).ok())
                .and_then(|dir| {
                    super::workspace::relative_path(
                        call.arguments["path"].as_str().unwrap_or_default(),
                    )
                    .ok()
                    .and_then(|path| super::workspace::current_version(&dir, &path).ok())
                })
                .map(|current| &current == expected)
        }
        _ => None,
    };
    let result = match applied {
        Some(true) => {
            json!({ "status": "success", "result": { "note": "This write finished before Juniper closed; the file has the new content." } })
        }
        Some(false) => {
            json!({ "status": "error", "error": { "code": "NOT_APPLIED", "message": "This write did not take effect before Juniper closed." } })
        }
        None => {
            json!({ "status": "error", "error": { "code": "OUTCOME_UNCERTAIN", "message": "Juniper closed while this action was running. Its outcome is unknown; check before repeating it." } })
        }
    };
    store::set_action(
        connection,
        &action.id,
        if applied == Some(true) {
            "succeeded"
        } else {
            "uncertain"
        },
        Some(&result),
    )?;
    task.transcript.push(TranscriptMessage {
        role: "tool".into(),
        content: crate::tools::host_result(
            &call.id,
            &call.name,
            result["status"].as_str().unwrap_or("error"),
            result.get("result").cloned(),
            result.get("error").cloned(),
        )
        .to_string(),
        tool_calls: Vec::new(),
        tool_call_id: Some(call.id.clone()),
        tool_name: Some(call.name.clone()),
    });
    if let Some(pending) = &mut task.pending {
        pending.next += 1;
        pending.wait = None;
        if pending.next >= pending.calls.len() {
            task.pending = None;
        }
    }
    store::add_event(
        connection,
        &task.id,
        "action",
        "Checked an action that was interrupted",
        Some(json!({ "tool": call.name, "applied": applied })),
    )?;
    Ok(())
}

#[tauri::command]
pub fn task_cancel<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    let agent = &state.agent;
    // Serialize the inactive-task database cancellation with worker startup.
    // Otherwise a queued task can become active after signal() returns false,
    // leaving its new cancellation token uncancelled.
    let _route_configuration = agent.lock_route_configuration();
    let signalled = agent.signal(&task_id, Intent::Cancel);
    with_task(&app, agent, &task_id, |connection, task| {
        // Descendants are cancelled whether or not the parent is executing.
        if signalled {
            for child in store::descendants(connection, &task.id)? {
                agent.signal(&child.id, Intent::Cancel);
            }
            Ok(())
        } else {
            runner::cancel_tree(connection, agent, task, "Cancelled by you.")
        }
    })
}

/// Adds the user's words to an unfinished task. The earlier goal stays; the
/// follow-up is a new instruction from the user, labelled as such.
#[tauri::command]
pub fn task_steer<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    task_id: String,
    text: String,
) -> Result<(), String> {
    let text = bounded(&text, 4000)?;
    let agent = &state.agent;
    let _route_configuration = agent.lock_route_configuration();
    if agent.active_ids().contains(&task_id) {
        // An executing task is paused first so the message is not lost in a
        // concurrent write; the user's message is applied on resume.
        agent.signal(&task_id, Intent::Pause);
        return Err("TASK_BUSY: The task is pausing so you can add to it. Send your message again in a moment.".into());
    }
    with_task(&app, agent, &task_id, |connection, task| {
        if task.parent_id.is_some() {
            return Err("TASK_NOT_STEERABLE: Steer the main task instead of a subtask.".into());
        }
        if task.pending.is_some() {
            return Err("TASK_WAITING: Answer or decide the open request first.".into());
        }
        task.transcript.push(TranscriptMessage {
            role: "user".into(),
            content: format!("Follow-up from the user (the earlier goal still applies unless this changes it): {text}"),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        });
        // A follow-up starts a fresh allowance of steps on the same budget.
        task.budget.max_steps = task.budget.max_steps.max(task.usage.steps + 8).min(64);
        task.result = None;
        store::add_event(
            connection,
            &task.id,
            "steer",
            &format!("You added: {text}"),
            None,
        )?;
        let next = TaskStatus::Queued;
        if task.status == TaskStatus::Queued {
            store::save_task(connection, task)
        } else {
            store::transition(
                connection,
                task,
                next,
                Some("Continuing with your follow-up.".into()),
            )
        }
    })
}

#[tauri::command]
pub fn task_answer<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    task_id: String,
    answer: String,
) -> Result<(), String> {
    let answer = bounded(&answer, 4000)?;
    let _route_configuration = state.agent.lock_route_configuration();
    with_task(&app, &state.agent, &task_id, |connection, task| {
        let waiting = matches!(
            task.pending
                .as_ref()
                .and_then(|pending| pending.wait.as_ref()),
            Some(Wait::Input { answer: None, .. })
        );
        if task.status != TaskStatus::WaitingInput || !waiting {
            return Err("TASK_NOT_WAITING: This task is not waiting for an answer.".into());
        }
        if let Some(Pending {
            wait: Some(Wait::Input { answer: slot, .. }),
            ..
        }) = &mut task.pending
        {
            *slot = Some(answer.clone());
        }
        store::add_event(
            connection,
            &task.id,
            "input",
            &format!("You answered: {answer}"),
            None,
        )?;
        store::transition(connection, task, TaskStatus::Queued, None)
    })
}

#[tauri::command]
pub fn approval_decide<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    approval_id: String,
    decision: String,
) -> Result<(), String> {
    valid_id(&approval_id)?;
    let agent = &state.agent;
    let _route_configuration = agent.lock_route_configuration();
    let connection = agent.connection()?;
    let approval = store::approval(&connection, &approval_id)?;
    let (status, recorded) = match decision.as_str() {
        "allow-once" => ("approved", "allow-once"),
        "allow-standing" if approval.standing_allowed => ("approved", "allow-standing"),
        "allow-standing" => {
            return Err("APPROVAL_ONCE_ONLY: This action is approved one call at a time.".into());
        }
        "deny" => ("denied", "deny"),
        _ => return Err("INVALID_INPUT: Unknown decision.".into()),
    };
    store::decide_approval(&connection, &approval_id, status, recorded)?;
    store::set_action(
        &connection,
        &approval.action_id,
        if status == "approved" {
            "approved"
        } else {
            "denied"
        },
        None,
    )?;
    let mut task = store::load_task(&connection, &approval.task_id)?;
    store::add_event(
        &connection,
        &task.id,
        "approval",
        &format!(
            "You {} {}",
            if status == "approved" {
                "approved"
            } else {
                "denied"
            },
            approval.preview.lines().next().unwrap_or_default()
        ),
        Some(json!({ "decision": recorded, "tool": approval.tool })),
    )?;
    if task.status == TaskStatus::WaitingApproval {
        store::transition(&connection, &mut task, TaskStatus::Queued, None)?;
    }
    agent.wake.notify_one();
    super::notify(&app, Some(&approval.task_id), "task");
    Ok(())
}

#[tauri::command]
pub fn grants_list(state: State<'_, AppState>) -> Result<Vec<Grant>, String> {
    store::grants(&state.agent.connection()?, true)
}

/// Revokes a standing permission. Later actions are decided without it, and
/// running tasks of that agent are paused so nothing already in flight
/// continues on the revoked authority unseen.
#[tauri::command]
pub fn grant_revoke<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    grant_id: String,
) -> Result<(), String> {
    valid_id(&grant_id)?;
    let agent = &state.agent;
    let connection = agent.connection()?;
    let grant = store::grants(&connection, true)?
        .into_iter()
        .find(|grant| grant.id == grant_id)
        .ok_or_else(|| "GRANT_NOT_FOUND: That permission does not exist.".to_owned())?;
    store::revoke_grant(&connection, &grant_id)?;
    for id in agent.active_ids() {
        if let Ok(task) = store::load_task(&connection, &id)
            && task.assistant_id == grant.assistant_id
        {
            agent.signal(&id, Intent::Pause);
        }
    }
    super::notify(&app, None, "grants");
    Ok(())
}

/// The global stop: interrupts every executing task and starts nothing new
/// until agent work is resumed. The setting survives a restart.
#[tauri::command]
pub fn agent_stop_all<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    stopped: bool,
) -> Result<(), String> {
    let agent = &state.agent;
    let connection = agent.connection()?;
    let mut settings = store::agent_settings(&connection)?;
    settings["stopped"] = json!(stopped);
    store::set_agent_settings(&connection, &settings)?;
    agent.stopped.store(stopped, Ordering::Release);
    if stopped {
        for id in agent.active_ids() {
            agent.signal(&id, Intent::Stop);
        }
    } else {
        agent.wake.notify_one();
    }
    super::notify(&app, None, "stopped");
    Ok(())
}

#[tauri::command]
pub fn agent_settings_set(state: State<'_, AppState>, changes: Value) -> Result<Value, String> {
    let connection = state.agent.connection()?;
    let mut settings = store::agent_settings(&connection)?;
    let Some(object) = changes.as_object() else {
        return Err("INVALID_INPUT: Settings must be an object.".into());
    };
    for (key, value) in object {
        match key.as_str() {
            "timezone" => {
                let zone = value.as_str().unwrap_or_default();
                zone.parse::<chrono_tz::Tz>()
                    .map_err(|_| "INVALID_INPUT: Unknown time zone.".to_owned())?;
                settings["timezone"] = json!(zone);
            }
            "web" => {
                settings["web"] = super::web::validated_settings(value)?;
            }
            _ => return Err(format!("INVALID_INPUT: Unknown setting “{key}”.")),
        }
    }
    store::set_agent_settings(&connection, &settings)?;
    Ok(settings)
}

// ------------------------------------------------------------------ projects

#[tauri::command]
pub fn project_save(
    state: State<'_, AppState>,
    id: Option<String>,
    name: String,
    instructions: String,
    archived: bool,
    expected_revision: i64,
) -> Result<Project, String> {
    let name = bounded(&name, 100)?;
    let instructions: String = instructions.trim().chars().take(8000).collect();
    let connection = state.agent.connection()?;
    let now = store::now();
    let id = match id {
        Some(id) => {
            valid_id(&id)?;
            let changed = connection
                .execute(
                    "UPDATE projects SET name = ?1, instructions = ?2, archived = ?3, updated_at = ?4, revision = revision + 1 WHERE id = ?5 AND revision = ?6",
                    rusqlite::params![name, instructions, archived, now, id, expected_revision],
                )
                .map_err(store::db_error)?;
            if changed != 1 {
                return Err("PROJECT_REVISION_CONFLICT: This project changed elsewhere. Reload and try again.".into());
            }
            id
        }
        None => {
            let id = store::new_id("project");
            connection
                .execute(
                    "INSERT INTO projects(id, name, instructions, archived, payload, revision, created_at, updated_at) VALUES(?1, ?2, ?3, 0, '{}', 1, ?4, ?4)",
                    rusqlite::params![id, name, instructions, now],
                )
                .map_err(store::db_error)?;
            id
        }
    };
    store::project(&connection, &id)
}

/// Deletes a project and its folder grants and document index. Tasks and
/// documents it produced remain, without the project link.
#[tauri::command]
pub fn project_delete(state: State<'_, AppState>, project_id: String) -> Result<(), String> {
    valid_id(&project_id)?;
    let connection = state.agent.connection()?;
    pause_project_tasks(&state.agent, &connection, &project_id);
    super::retrieval::forget_project(&connection, &project_id)?;
    connection
        .execute("DELETE FROM projects WHERE id = ?1", [&project_id])
        .map_err(store::db_error)?;
    connection
        .execute(
            "UPDATE tasks SET project_id = NULL WHERE project_id = ?1",
            [&project_id],
        )
        .map_err(store::db_error)?;
    connection
        .execute(
            "UPDATE artifacts SET project_id = NULL WHERE project_id = ?1",
            [&project_id],
        )
        .map_err(store::db_error)?;
    Ok(())
}

/// Grants a folder to a project. The path comes only from the native folder
/// picker; the interface can choose the access level, never the path.
#[tauri::command]
pub async fn project_add_root<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    project_id: String,
    access: String,
) -> Result<Option<Project>, String> {
    valid_id(&project_id)?;
    if !matches!(access.as_str(), "read" | "read-write") {
        return Err("INVALID_INPUT: Choose read or read-write access.".into());
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (app, state, project_id, access);
        Err("WORKSPACE_UNAVAILABLE: Project folders are available on desktop.".into())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .set_title("Choose a folder for this project")
            .pick_folder(move |selected| {
                let _ = sender.send(selected);
            });
        let Some(selected) = receiver
            .await
            .map_err(|_| "The folder picker closed unexpectedly.".to_owned())?
        else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|_| "Select a local folder.".to_owned())?;
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| "That folder is not available.".to_owned())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Choose a regular folder, not a link.".into());
        }
        let canonical =
            std::fs::canonicalize(&path).map_err(|_| "That folder is not available.".to_owned())?;
        let label = canonical
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| canonical.to_string_lossy().into_owned());
        let connection = state.agent.connection()?;
        store::project(&connection, &project_id)?;
        connection
            .execute(
                "INSERT INTO project_roots(id, project_id, path, label, access, created_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![store::new_id("root"), project_id, canonical.to_string_lossy(), label, access, store::now()],
            )
            .map_err(store::db_error)?;
        connection
            .execute(
                "UPDATE projects SET revision = revision + 1, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![store::now(), project_id],
            )
            .map_err(store::db_error)?;
        store::project(&connection, &project_id).map(Some)
    }
}

/// Pauses running tasks of a project whose folder access is changing, so
/// nothing in flight keeps using the old access.
fn pause_project_tasks(agent: &AgentState, connection: &Connection, project_id: &str) {
    for id in agent.active_ids() {
        if store::load_task(connection, &id)
            .is_ok_and(|task| task.project_id.as_deref() == Some(project_id))
        {
            agent.signal(&id, Intent::Pause);
        }
    }
}

/// Revokes a folder grant and removes everything indexed from it.
#[tauri::command]
pub fn project_remove_root(state: State<'_, AppState>, root_id: String) -> Result<(), String> {
    valid_id(&root_id)?;
    let connection = state.agent.connection()?;
    if let Ok(root) = store::root(&connection, &root_id) {
        pause_project_tasks(&state.agent, &connection, &root.project_id);
    }
    super::retrieval::forget_root(&connection, &root_id)?;
    connection
        .execute(
            "UPDATE project_roots SET revoked_at = ?1 WHERE id = ?2",
            rusqlite::params![store::now(), root_id],
        )
        .map_err(store::db_error)?;
    Ok(())
}

#[tauri::command]
pub async fn project_index(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Value, String> {
    valid_id(&project_id)?;
    let database = state.agent.paths()?.database.clone();
    tokio::task::spawn_blocking(move || {
        let connection = store::open(&database)?;
        let project = store::project(&connection, &project_id)?;
        super::retrieval::index_project(&connection, &project)
    })
    .await
    .map_err(|_| "INDEX_FAILED: Indexing stopped unexpectedly.".to_owned())?
}

/// A project for export: name, instructions, and folder labels with their
/// access level, but no paths, grants, index, or tasks.
#[tauri::command]
pub fn project_export(state: State<'_, AppState>, project_id: String) -> Result<Value, String> {
    valid_id(&project_id)?;
    let project = store::project(&state.agent.connection()?, &project_id)?;
    Ok(json!({
        "format": "juniper-project",
        "schemaVersion": 1,
        "name": project.name,
        "instructions": project.instructions,
        "folders": project.roots.iter().map(|root| json!({ "label": root.label, "access": root.access })).collect::<Vec<_>>(),
        "note": "Folder access is not exported. Grant folders again after importing."
    }))
}

#[tauri::command]
pub fn project_import(state: State<'_, AppState>, document: Value) -> Result<Project, String> {
    if document["format"] != "juniper-project" || document["schemaVersion"] != 1 {
        return Err("IMPORT_INVALID: This is not a Juniper project export.".into());
    }
    let name = bounded(document["name"].as_str().unwrap_or_default(), 100)?;
    let instructions = document["instructions"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    project_save(state, None, name, instructions, false, 0)
}

// ------------------------------------------------------------------ artifacts, inbox, memory

#[tauri::command]
pub fn artifact_content(state: State<'_, AppState>, artifact_id: String) -> Result<Value, String> {
    valid_id(&artifact_id)?;
    let agent = &state.agent;
    let connection = agent.connection()?;
    let artifact = store::artifact(&connection, &artifact_id)?;
    let root = artifact_root(&connection, &artifact)?;
    let bytes = std::fs::read(super::tools::artifact_path(
        agent.paths()?,
        &artifact,
        &root,
    ))
    .map_err(|_| "ARTIFACT_MISSING: The document's file is missing.".to_owned())?;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(2 * 1024 * 1024)]).into_owned();
    Ok(json!({ "artifact": artifact, "text": text, "truncated": bytes.len() > 2 * 1024 * 1024 }))
}

fn artifact_root(connection: &Connection, artifact: &Artifact) -> Result<String, String> {
    match &artifact.task_id {
        Some(task) => Ok(store::load_task(connection, task)?.root_id),
        None => Err("ARTIFACT_MISSING: The document's task was removed.".into()),
    }
}

#[tauri::command]
pub async fn artifact_export<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    artifact_id: String,
) -> Result<bool, String> {
    valid_id(&artifact_id)?;
    let agent = &state.agent;
    let connection = agent.connection()?;
    let artifact = store::artifact(&connection, &artifact_id)?;
    let root = artifact_root(&connection, &artifact)?;
    let source = super::tools::artifact_path(agent.paths()?, &artifact, &root);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&artifact.file_name)
        .save_file(move |selected| {
            let _ = sender.send(selected);
        });
    let Some(target) = receiver
        .await
        .map_err(|_| "The save dialog closed unexpectedly.".to_owned())?
    else {
        return Ok(false);
    };
    let target = target
        .into_path()
        .map_err(|_| "Choose a local file.".to_owned())?;
    std::fs::copy(source, target)
        .map_err(|_| "EXPORT_FAILED: The document could not be saved there.".to_owned())?;
    Ok(true)
}

#[tauri::command]
pub fn inbox_mark_read<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<(), String> {
    store::mark_inbox_read(&state.agent.connection()?, &ids)?;
    super::notify(&app, None, "inbox");
    Ok(())
}

#[tauri::command]
pub fn memory_changes(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    store::memory_outbox(&state.agent.connection()?)
}

// ------------------------------------------------------------------ skills

#[tauri::command]
pub fn skills_list(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    Ok(super::skills::list(&state.agent.connection()?)?
        .iter()
        .map(super::skills::summary)
        .collect())
}

#[tauri::command]
pub fn skill_set_enabled(
    state: State<'_, AppState>,
    skill_id: String,
    enabled: bool,
) -> Result<(), String> {
    valid_id(&skill_id)?;
    super::skills::set_enabled(&state.agent.connection()?, &skill_id, enabled)
}

#[tauri::command]
pub fn skill_remove(state: State<'_, AppState>, skill_id: String) -> Result<(), String> {
    valid_id(&skill_id)?;
    super::skills::remove(&state.agent.connection()?, &skill_id)
}

/// Imports a skill folder (or a lone SKILL.md) chosen in the native picker.
/// It arrives disabled; the interface shows its full text and requested
/// tools before the user enables it.
#[tauri::command]
pub async fn skill_import<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<Option<Value>, String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (app, state);
        Err("SKILL_IMPORT_UNAVAILABLE: Importing skill folders is available on desktop.".to_owned())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .set_title("Choose a skill folder (containing SKILL.md)")
            .pick_folder(move |selected| {
                let _ = sender.send(selected);
            });
        let Some(selected) = receiver
            .await
            .map_err(|_| "The folder picker closed unexpectedly.".to_owned())?
        else {
            return Ok(None);
        };
        let folder = selected
            .into_path()
            .map_err(|_| "Select a local folder.".to_owned())?;
        let skill = tokio::task::spawn_blocking(move || super::skills::read_folder(&folder))
            .await
            .map_err(|_| "SKILL_INVALID: Reading the skill stopped unexpectedly.".to_owned())??;
        let connection = state.agent.connection()?;
        let skill = super::skills::import(&connection, skill)?;
        Ok(Some(super::skills::summary(&skill)))
    }
}

#[tauri::command]
pub async fn skill_export<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    skill_id: String,
) -> Result<bool, String> {
    valid_id(&skill_id)?;
    let skill = super::skills::get(&state.agent.connection()?, &skill_id)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name("SKILL.md")
        .save_file(move |selected| {
            let _ = sender.send(selected);
        });
    let Some(target) = receiver
        .await
        .map_err(|_| "The save dialog closed unexpectedly.".to_owned())?
    else {
        return Ok(false);
    };
    let target = target
        .into_path()
        .map_err(|_| "Choose a local file.".to_owned())?;
    std::fs::write(target, super::skills::export(&skill))
        .map_err(|_| "EXPORT_FAILED: The skill could not be saved there.".to_owned())?;
    Ok(true)
}

// ------------------------------------------------------------------ plugins (MCP)

#[tauri::command]
pub fn plugins_list(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    Ok(super::mcp::list(&state.agent.connection()?)?
        .iter()
        .map(super::mcp::summary)
        .collect())
}

/// Registers a remote (HTTP) server. Local servers are registered with
/// `plugin_register_local`, whose program path comes from the native picker.
#[tauri::command]
pub fn plugin_register_http(
    state: State<'_, AppState>,
    name: String,
    url: String,
    credential_ref: Option<String>,
) -> Result<Value, String> {
    let config = super::mcp::PluginConfig {
        name,
        kind: "mcp-http".into(),
        command: None,
        args: Vec::new(),
        env: Vec::new(),
        url: Some(url),
        credential_ref,
    };
    let plugin = super::mcp::register(&state.agent.connection()?, config)?;
    Ok(super::mcp::summary(&plugin))
}

#[tauri::command]
pub async fn plugin_register_local<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    name: String,
    args: Vec<String>,
    env: Vec<super::mcp::EnvVar>,
) -> Result<Option<Value>, String> {
    super::mcp::stdio_availability()?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose the MCP server program")
        .pick_file(move |selected| {
            let _ = sender.send(selected);
        });
    let Some(selected) = receiver
        .await
        .map_err(|_| "The file picker closed unexpectedly.".to_owned())?
    else {
        return Ok(None);
    };
    let command = selected
        .into_path()
        .map_err(|_| "Select a local program.".to_owned())?;
    let config = super::mcp::PluginConfig {
        name,
        kind: "mcp-stdio".into(),
        command: Some(command.to_string_lossy().into_owned()),
        args,
        env,
        url: None,
        credential_ref: None,
    };
    let plugin = super::mcp::register(&state.agent.connection()?, config)?;
    Ok(Some(super::mcp::summary(&plugin)))
}

#[tauri::command]
pub async fn plugin_test(state: State<'_, AppState>, plugin_id: String) -> Result<Value, String> {
    valid_id(&plugin_id)?;
    let database = state.agent.paths()?.database.clone();
    let plugin = super::mcp::test_connection(database, plugin_id).await?;
    Ok(super::mcp::summary(&plugin))
}

#[tauri::command]
pub fn plugin_set_enabled<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    plugin_id: String,
    enabled: bool,
) -> Result<(), String> {
    valid_id(&plugin_id)?;
    let agent = &state.agent;
    super::mcp::set_enabled(&agent.connection()?, &plugin_id, enabled)?;
    if !enabled {
        pause_tasks_using_plugin(agent, &plugin_id)?;
    }
    super::notify(&app, None, "plugins");
    Ok(())
}

#[tauri::command]
pub fn plugin_remove<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<(), String> {
    valid_id(&plugin_id)?;
    let agent = &state.agent;
    pause_tasks_using_plugin(agent, &plugin_id)?;
    super::mcp::remove(&agent.connection()?, &plugin_id)?;
    super::notify(&app, None, "plugins");
    Ok(())
}

/// Turning a server off or removing it interrupts running tasks that were
/// offered its tools, so no call already in flight continues unseen.
fn pause_tasks_using_plugin(agent: &AgentState, plugin_id: &str) -> Result<(), String> {
    let connection = agent.connection()?;
    let prefix = format!("mcp.{plugin_id}.");
    for id in agent.active_ids() {
        if let Ok(task) = store::load_task(&connection, &id)
            && task
                .provenance
                .tools
                .iter()
                .any(|tool| tool.starts_with(&prefix))
        {
            agent.signal(&id, Intent::Pause);
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ schedules

#[tauri::command]
pub fn schedules_list(
    state: State<'_, AppState>,
) -> Result<Vec<super::schedule::Schedule>, String> {
    super::schedule::list(&state.agent.connection()?)
}

#[tauri::command]
pub fn schedule_preview(spec: super::schedule::ScheduleSpec) -> Result<Vec<String>, String> {
    super::schedule::validate(&spec)?;
    Ok(super::schedule::upcoming(&spec, chrono::Utc::now(), 5)
        .into_iter()
        .map(|instant| super::schedule::local_label(instant, &spec.timezone))
        .collect())
}

#[tauri::command]
pub fn schedule_save<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: Option<String>,
    spec: super::schedule::ScheduleSpec,
    enabled: bool,
    expected_revision: i64,
) -> Result<super::schedule::Schedule, String> {
    let connection = state.agent.connection()?;
    if spec.action == "agent-task" {
        assistant_exists(&connection, &spec.assistant_id)?;
    }
    if let Some(project) = &spec.project_id {
        store::project(&connection, project)?;
    }
    let schedule = super::schedule::save(
        &connection,
        id.as_deref(),
        spec,
        enabled,
        expected_revision,
        chrono::Utc::now(),
    )?;
    super::notify(&app, None, "schedule");
    Ok(schedule)
}

#[tauri::command]
pub fn schedule_delete(state: State<'_, AppState>, schedule_id: String) -> Result<(), String> {
    valid_id(&schedule_id)?;
    super::schedule::delete(&state.agent.connection()?, &schedule_id)
}

#[tauri::command]
pub fn schedule_run_now<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    schedule_id: String,
) -> Result<String, String> {
    valid_id(&schedule_id)?;
    let agent = &state.agent;
    if agent.is_stopped() {
        return Err("AGENT_STOPPED: Agent work is stopped. Resume it first.".into());
    }
    let run = super::schedule::run_now(&agent.connection()?, &schedule_id, chrono::Utc::now())?;
    agent.wake.notify_one();
    super::notify(&app, None, "schedule");
    Ok(run)
}

#[tauri::command]
pub fn schedule_runs(
    state: State<'_, AppState>,
    schedule_id: String,
) -> Result<Vec<Value>, String> {
    valid_id(&schedule_id)?;
    super::schedule::runs(&state.agent.connection()?, &schedule_id)
}
