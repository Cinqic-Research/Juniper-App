//! End-to-end runner tests against a scripted OpenAI-compatible fixture
//! server. The fixture stands in for a model: it returns the tool calls and
//! answers each test scripts, so every host decision — policy, approvals,
//! budgets, delegation, recovery — runs for real against real files.

use super::store;
use super::types::*;
use crate::commands::AppState;
use crate::providers::{TranscriptMessage, TranscriptToolCall};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Manager;

struct Fixture {
    root: PathBuf,
    folder: PathBuf,
    app: tauri::App<tauri::test::MockRuntime>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn sse_tool_calls(calls: &[(&str, &str, Value)]) -> String {
    let mut body = String::new();
    for (index, (id, name, arguments)) in calls.iter().enumerate() {
        let delta = json!({ "choices": [{ "delta": { "tool_calls": [{ "index": index, "id": id, "function": { "name": name, "arguments": arguments.to_string() } }] } }] });
        body.push_str(&format!("data: {delta}\n\n"));
    }
    body.push_str("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n");
    body.push_str("data: [DONE]\n\n");
    body
}

fn sse_answer(text: &str) -> String {
    let delta = json!({ "choices": [{ "delta": { "content": text } }] });
    format!(
        "data: {delta}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    )
}

/// Answers each model request with the next scripted body.
fn model_server(script: Vec<String>, requests: Arc<Mutex<Vec<Value>>>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    std::thread::spawn(move || {
        for body in script {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut received = Vec::new();
            let mut chunk = [0u8; 65536];
            loop {
                let read = stream.read(&mut chunk).unwrap_or(0);
                received.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&received).into_owned();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if received.len() >= split + 4 + length {
                        if let Ok(value) =
                            serde_json::from_slice(&received[split + 4..split + 4 + length])
                        {
                            requests.lock().unwrap().push(value);
                        }
                        break;
                    }
                }
                if read == 0 {
                    break;
                }
            }
            let body = if let Some(delayed) = body.strip_prefix("DELAY_MS:") {
                let (delay, response) = delayed.split_once('\n').expect("delay fixture format");
                std::thread::sleep(std::time::Duration::from_millis(
                    delay.parse().expect("delay milliseconds"),
                ));
                response.to_owned()
            } else {
                body
            };
            let response = if body.starts_with("HTTP/") {
                body
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}")
}

fn new_fixture(script: Vec<String>) -> Fixture {
    new_fixture_with(move |_| script)
}

/// A fixture whose model script is written after `setup` has prepared the
/// database (for example, to name a plugin's generated id).
fn new_fixture_with(setup: impl FnOnce(&std::path::Path) -> Vec<String>) -> Fixture {
    let root = std::env::temp_dir().join(format!("juniper-agent-e2e-{}", uuid::Uuid::new_v4()));
    let folder = root.join("project");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("notes.md"),
        "Budget for Q4 is 12,000 dollars.\n",
    )
    .unwrap();
    let database = root.join("juniper.db");
    crate::storage::initialize(&database).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let script = setup(&database);
    let base_url = model_server(script, requests.clone());
    let connection = store::open(&database).unwrap();
    let snapshot = json!({
        "assistants": [{ "id": "agent-1", "name": "Juno", "modelProfileId": "model-1", "generation": {}, "memoryPolicy": "curated" }],
        "providers": [{ "id": "provider-1", "name": "Fixture", "kind": "openai-compatible", "baseUrl": base_url, "locality": "local", "transportLocation": "on-device", "enabled": true }],
        "models": [{ "id": "model-1", "providerId": "provider-1", "modelId": "fixture", "displayName": "Fixture model", "executionLocation": "on-device", "capabilities": { "tools": "supported", "thinking": "unsupported", "generationParameters": [] } }],
        "memories": [{ "id": "memory-1", "assistantId": "agent-1", "content": "Prefers short reports.", "enabled": true }],
        "settings": {}
    });
    connection
        .execute(
            "INSERT INTO app_state(id, payload, updated_at) VALUES('singleton', ?1, 'x')",
            [snapshot.to_string()],
        )
        .unwrap();
    connection
        .execute_batch(&format!(
            "INSERT INTO projects(id, name, created_at, updated_at) VALUES('project-1', 'Budget', 'x', 'x');
             INSERT INTO project_roots(id, project_id, path, label, access, created_at) VALUES('root-1', 'project-1', '{}', 'project', 'read-write', 'x');",
            folder.to_string_lossy()
        ))
        .unwrap();
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app");
    app.state::<AppState>()
        .agent
        .set_paths_for_tests(database, root.join("data"));
    Fixture {
        root,
        folder,
        app,
        requests,
    }
}

#[test]
fn task_disclosure_uses_current_provider_route_when_model_metadata_is_stale() {
    let fixture = new_fixture(vec![]);
    let id = fixture.start("Describe the route.", false);
    let connection = fixture.connection();
    let mut snapshot = store::app_snapshot(&connection).unwrap();
    let provider = &mut snapshot["providers"][0];
    provider["baseUrl"] = json!("https://api.example.com/v1");
    provider["locality"] = json!("local");
    provider["transportLocation"] = json!("on-device");
    let model = &mut snapshot["models"][0];
    model["executionLocation"] = json!("on-device");
    connection
        .execute(
            "UPDATE app_state SET payload = ?1 WHERE id = 'singleton'",
            [snapshot.to_string()],
        )
        .unwrap();

    let task = store::load_task(&connection, &id).unwrap();
    let context = super::context::resolve(&connection, &task).unwrap();
    assert_eq!(context.provider.transport_location, "remote");
    assert_eq!(context.model.execution_location, "remote");
    assert!(
        super::context::instructions(&context, &task, "lineage")
            .unwrap()
            .contains("on a remote service; task content is sent to it")
    );
}

#[test]
fn changing_a_provider_route_pauses_an_active_task_using_its_saved_run_context() {
    let fixture = new_fixture(vec![]);
    let id = fixture.start("Describe the route.", false);
    let connection = fixture.connection();
    let old_snapshot = store::app_snapshot(&connection).unwrap();
    let mut new_snapshot = old_snapshot.clone();
    new_snapshot["providers"][0]["baseUrl"] = json!("http://127.0.0.1:11435");
    let agent = &fixture.app.state::<AppState>().agent;
    agent.begin(&id).expect("active run should register");

    super::context::pause_tasks_with_route_changes(
        agent,
        &connection,
        Some(&old_snapshot),
        &new_snapshot,
    )
    .unwrap();

    assert_eq!(agent.intent(&id), Some(super::Intent::PauseForRouteChange));
}

#[test]
fn route_change_pauses_a_waiting_approval_with_private_project_context_until_resume() {
    let fixture = new_fixture(vec![
        sse_tool_calls(&[(
            "read-private-note",
            "workspace.read",
            json!({ "rootId": "root-1", "path": "notes.md" }),
        )]),
        sse_tool_calls(&[(
            "write-after-review",
            "workspace.write",
            json!({ "rootId": "root-1", "path": "out/summary.md", "content": "Budget reviewed.\n", "expectedVersion": "absent" }),
        )]),
    ]);
    let id = fixture.start("Read the budget note, then save a summary.", true);
    let waiting = fixture.run(&id);
    assert_eq!(waiting.status, TaskStatus::WaitingApproval);
    assert!(
        serde_json::to_string(&waiting.transcript)
            .unwrap()
            .contains("Budget for Q4 is 12,000 dollars.")
    );
    let connection = fixture.connection();
    let old_snapshot = store::app_snapshot(&connection).unwrap();
    let mut new_snapshot = old_snapshot.clone();
    new_snapshot["providers"][0]["baseUrl"] = json!("https://example.com/v1");
    let agent = &fixture.app.state::<AppState>().agent;
    assert!(!agent.active_ids().contains(&id));

    super::context::pause_tasks_with_route_changes(
        agent,
        &connection,
        Some(&old_snapshot),
        &new_snapshot,
    )
    .unwrap();
    crate::storage::save_app_data_with_paths(
        &fixture.root.join("juniper.db"),
        &new_snapshot,
        &std::collections::HashMap::new(),
    )
    .unwrap();

    let paused = store::load_task(&connection, &id).unwrap();
    assert_eq!(paused.status, TaskStatus::Paused);
    assert!(
        paused
            .status_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Review the task content and destination model"))
    );
    assert!(
        store::runnable_tasks(&connection)
            .unwrap()
            .iter()
            .all(|task| task.id != id)
    );

    let approval = store::approvals_for_task(&connection, &id)
        .unwrap()
        .into_iter()
        .find(|approval| approval.status == "pending")
        .expect("the write approval remains available for review");
    let handle = fixture.app.handle().clone();
    super::commands::approval_decide(
        handle,
        fixture.app.state::<AppState>(),
        approval.id,
        "allow-once".into(),
    )
    .unwrap();
    assert_eq!(
        store::load_task(&connection, &id).unwrap().status,
        TaskStatus::Paused,
        "approving the tool does not resume a task whose model destination changed"
    );
    assert_eq!(fixture.requests.lock().unwrap().len(), 2);

    let handle = fixture.app.handle().clone();
    super::commands::task_resume(handle, fixture.app.state::<AppState>(), id.clone()).unwrap();
    assert_eq!(
        store::load_task(&connection, &id).unwrap().status,
        TaskStatus::Queued,
        "only an explicit user resume releases the task on the changed route"
    );
    assert_eq!(fixture.requests.lock().unwrap().len(), 2);
}

#[test]
fn profile_change_pauses_waiting_tasks_until_the_user_resumes_them() {
    let fixture = new_fixture(vec![sse_tool_calls(&[(
        "write-after-profile-change",
        "workspace.write",
        json!({ "rootId": "root-1", "path": "out/profile.md", "content": "Review required.\n", "expectedVersion": "absent" }),
    )])]);
    let id = fixture.start("Write a project note.", true);
    assert_eq!(fixture.run(&id).status, TaskStatus::WaitingApproval);

    super::commands::agent_profile_save(
        fixture.app.state::<AppState>(),
        AgentProfile::default_for("agent-1"),
        0,
    )
    .unwrap();

    let connection = fixture.connection();
    let task = store::load_task(&connection, &id).unwrap();
    assert_eq!(task.status, TaskStatus::Paused);
    assert!(
        task.status_reason
            .as_deref()
            .is_some_and(|reason| { reason.contains("Review its permissions, model destination") })
    );
    assert!(
        store::runnable_tasks(&connection)
            .unwrap()
            .iter()
            .all(|task| task.id != id)
    );
}

#[test]
fn route_change_while_the_runner_enters_approval_wait_still_requires_resume() {
    let fixture = new_fixture(vec![sse_tool_calls(&[(
        "wait-for-approval",
        "workspace.write",
        json!({ "rootId": "root-1", "path": "out/route-race.md", "content": "private summary\n", "expectedVersion": "absent" }),
    )])]);
    let id = fixture.start("Write a private project summary.", true);
    let waiting = fixture.run(&id);
    assert_eq!(waiting.status, TaskStatus::WaitingApproval);

    let connection = fixture.connection();
    let agent = &fixture.app.state::<AppState>().agent;
    agent.begin(&id).unwrap();
    assert!(agent.signal(&id, super::Intent::PauseForRouteChange));
    let intent = agent.end(&id);
    let mut task = store::load_task(&connection, &id).unwrap();
    super::runner::finalize_run(&connection, agent, &mut task, &Ok(()), intent).unwrap();
    assert_eq!(task.status, TaskStatus::Paused);

    let approval = store::approvals_for_task(&connection, &id)
        .unwrap()
        .into_iter()
        .find(|approval| approval.status == "pending")
        .unwrap();
    let handle = fixture.app.handle().clone();
    super::commands::approval_decide(
        handle,
        fixture.app.state::<AppState>(),
        approval.id,
        "allow-once".into(),
    )
    .unwrap();
    assert_eq!(
        store::load_task(&connection, &id).unwrap().status,
        TaskStatus::Paused,
        "finishing the approval cannot release a route-change pause"
    );
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
}

#[test]
fn cancellation_while_the_runner_enters_approval_wait_expires_the_approval() {
    let fixture = new_fixture(vec![sse_tool_calls(&[(
        "cancel-at-approval",
        "workspace.write",
        json!({ "rootId": "root-1", "path": "out/cancel-race.md", "content": "must not write\n", "expectedVersion": "absent" }),
    )])]);
    let id = fixture.start("Write a project note.", true);
    let waiting = fixture.run(&id);
    assert_eq!(waiting.status, TaskStatus::WaitingApproval);

    let agent = &fixture.app.state::<AppState>().agent;
    agent.begin(&id).unwrap();
    assert!(agent.signal(&id, super::Intent::Cancel));
    let intent = agent.end(&id);
    let connection = fixture.connection();
    let mut task = store::load_task(&connection, &id).unwrap();
    super::runner::finalize_run(&connection, agent, &mut task, &Ok(()), intent).unwrap();

    assert_eq!(task.status, TaskStatus::Cancelled);
    let approval = store::approvals_for_task(&connection, &id)
        .unwrap()
        .into_iter()
        .find(|approval| approval.tool == "workspace.write")
        .unwrap();
    assert_eq!(approval.status, "expired");
    assert_eq!(
        store::actions(&connection, &id)
            .unwrap()
            .into_iter()
            .find(|action| action.call_id == "cancel-at-approval")
            .unwrap()
            .status,
        "cancelled"
    );
    let handle = fixture.app.handle().clone();
    assert!(
        super::commands::approval_decide(
            handle,
            fixture.app.state::<AppState>(),
            approval.id,
            "allow-once".into(),
        )
        .is_err()
    );
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
}

#[test]
fn route_pause_serializes_with_delegated_child_creation() {
    let fixture = new_fixture(vec![]);
    let parent_id = fixture.start("Delegate a project summary.", true);
    let connection = fixture.connection();
    let parent = store::load_task(&connection, &parent_id).unwrap();
    let mut child = parent.clone();
    child.id = store::new_id("task");
    child.parent_id = Some(parent_id.clone());
    child.depth = parent.depth + 1;
    child.role = Some("researcher".into());
    child.title = "Researcher · project summary".into();
    child.goal = "summarize the private project context".into();
    child.status = TaskStatus::Queued;
    child.current_action = None;
    child.plan.clear();
    child.usage = TaskUsage::default();
    child.provenance = TaskProvenance::default();
    child.transcript = vec![TranscriptMessage {
        role: "user".into(),
        content: child.goal.clone(),
        tool_calls: Vec::new(),
        tool_call_id: None,
        tool_name: None,
    }];
    child.pending = None;
    child.result = None;
    child.revision = 1;
    child.created_at = store::now();
    child.updated_at = child.created_at.clone();
    child.finished_at = None;

    let agent = &fixture.app.state::<AppState>().agent;
    let cancellation = agent.begin(&parent_id).unwrap();
    let old_snapshot = store::app_snapshot(&connection).unwrap();
    let mut new_snapshot = old_snapshot.clone();
    new_snapshot["providers"][0]["baseUrl"] = json!("https://remote.example/v1");
    {
        let _route_configuration = agent.lock_route_configuration();
        super::context::pause_tasks_with_route_changes(
            agent,
            &connection,
            Some(&old_snapshot),
            &new_snapshot,
        )
        .unwrap();
    }

    assert!(
        super::runner::checkpoint_child_spawn_if_active(
            agent,
            &cancellation,
            &connection,
            &parent,
            &child,
        )
        .is_err()
    );
    assert!(store::load_task(&connection, &child.id).is_err());
    let intent = agent.end(&parent_id);
    let mut parent = store::load_task(&connection, &parent_id).unwrap();
    super::runner::finalize_run(
        &connection,
        agent,
        &mut parent,
        &Err("TASK_INTERRUPTED: route changed before child checkpoint".into()),
        intent,
    )
    .unwrap();
    assert_eq!(parent.status, TaskStatus::Paused);
    assert!(
        store::runnable_tasks(&connection)
            .unwrap()
            .iter()
            .all(|task| task.id != child.id)
    );
}

#[test]
fn changing_only_provider_health_status_does_not_pause_an_active_task() {
    let fixture = new_fixture(vec![]);
    let id = fixture.start("Keep using the selected route.", false);
    let connection = fixture.connection();
    let old_snapshot = store::app_snapshot(&connection).unwrap();
    let mut new_snapshot = old_snapshot.clone();
    new_snapshot["providers"][0]["status"] = json!("connected");
    let agent = &fixture.app.state::<AppState>().agent;
    agent.begin(&id).expect("active run should register");

    super::context::pause_tasks_with_route_changes(
        agent,
        &connection,
        Some(&old_snapshot),
        &new_snapshot,
    )
    .unwrap();

    assert_eq!(agent.intent(&id), None);
}

#[test]
fn changing_assistant_default_model_pauses_an_active_task_without_an_override() {
    let fixture = new_fixture(vec![]);
    let id = fixture.start("Keep using the selected route.", false);
    let connection = fixture.connection();
    let old_snapshot = store::app_snapshot(&connection).unwrap();
    let mut new_snapshot = old_snapshot.clone();
    new_snapshot["assistants"][0]["modelProfileId"] = json!("model-2");
    let agent = &fixture.app.state::<AppState>().agent;
    agent.begin(&id).expect("active run should register");

    super::context::pause_tasks_with_route_changes(
        agent,
        &connection,
        Some(&old_snapshot),
        &new_snapshot,
    )
    .unwrap();

    assert_eq!(agent.intent(&id), Some(super::Intent::PauseForRouteChange));
}

#[test]
fn failed_model_turns_are_charged_to_steps_tokens_and_active_time() {
    let fixture = new_fixture(vec![
        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .into(),
    ]);
    let id = fixture.start("Answer this briefly.", false);
    let connection = fixture.connection();
    let mut task = store::load_task(&connection, &id).unwrap();
    task.budget.max_retries_per_cause = 0;
    store::save_task(&connection, &mut task).unwrap();

    let task = fixture.run(&id);

    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.usage.steps, 1);
    assert!(task.usage.input_tokens > 0);
    assert!(task.usage.tokens_estimated);
    assert!(task.usage.active_ms > 0);
}

#[test]
fn a_model_turn_cannot_run_past_the_task_time_budget() {
    let fixture = new_fixture(vec![format!(
        "DELAY_MS:5000\n{}",
        sse_answer("This arrives after the task deadline.")
    )]);
    let id = fixture.start("Answer after the deadline.", false);
    let connection = fixture.connection();
    let mut task = store::load_task(&connection, &id).unwrap();
    task.budget.max_seconds = 2;
    store::save_task(&connection, &mut task).unwrap();

    let started = std::time::Instant::now();
    let task = fixture.run(&id);

    assert!(started.elapsed() < std::time::Duration::from_millis(4500));
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.result.unwrap().outcome, "partial");
    assert_eq!(task.usage.steps, 1);
    assert!(task.usage.input_tokens > 0);
    assert!(task.usage.active_ms >= 1900);
}

impl Fixture {
    fn connection(&self) -> rusqlite::Connection {
        self.app.state::<AppState>().agent.connection().unwrap()
    }

    fn start(&self, goal: &str, project: bool) -> String {
        let request = super::commands::StartTask {
            assistant_id: "agent-1".into(),
            goal: goal.into(),
            project_id: project.then(|| "project-1".into()),
            title: None,
        };
        super::commands::create_task(&self.connection(), &request, None)
            .unwrap()
            .id
    }

    fn run(&self, task_id: &str) -> Task {
        let handle = self.app.handle().clone();
        let id = task_id.to_owned();
        tauri::async_runtime::block_on(async move { super::runner::run_task(&handle, &id).await });
        store::load_task(&self.connection(), task_id).unwrap()
    }

    fn approve(&self, task: &Task, decision: &str) {
        let connection = self.connection();
        let approval = store::approvals_for_task(&connection, &task.id)
            .unwrap()
            .into_iter()
            .find(|approval| approval.status == "pending")
            .expect("a pending approval");
        let status = if decision == "deny" {
            "denied"
        } else {
            "approved"
        };
        store::decide_approval(&connection, &approval.id, status, decision).unwrap();
        let mut task = store::load_task(&connection, &task.id).unwrap();
        store::transition(&connection, &mut task, TaskStatus::Queued, None).unwrap();
    }
}

#[test]
fn a_goal_runs_through_real_tools_to_a_source_linked_deliverable_with_an_approved_write() {
    let script = vec![
        sse_tool_calls(&[
            (
                "c1",
                "task.update_plan",
                json!({ "items": [{ "text": "Read notes", "status": "in-progress" }, { "text": "Write summary", "status": "pending" }] }),
            ),
            (
                "c2",
                "workspace.read",
                json!({ "rootId": "root-1", "path": "notes.md" }),
            ),
        ]),
        sse_tool_calls(&[(
            "c3",
            "artifact.create",
            json!({ "title": "Budget summary", "fileName": "summary.md", "content": "# Summary\nThe Q4 budget is 12,000 dollars [S1].", "sourceIds": ["S1"] }),
        )]),
        sse_tool_calls(&[(
            "c4",
            "workspace.write",
            json!({ "rootId": "root-1", "path": "out/summary.md", "content": "Q4 budget: 12,000 dollars.\n", "expectedVersion": "absent" }),
        )]),
        sse_answer("I summarized the budget [S1] and saved it to out/summary.md."),
    ];
    let fixture = new_fixture(script);
    let id = fixture.start(
        "Summarize the budget notes and save a copy in the project.",
        true,
    );
    let task = fixture.run(&id);
    assert_eq!(
        task.status,
        TaskStatus::WaitingApproval,
        "{:?}",
        task.status_reason
    );
    assert!(
        !fixture.folder.join("out/summary.md").exists(),
        "nothing is written before approval"
    );
    let approvals = store::approvals_for_task(&fixture.connection(), &id).unwrap();
    assert!(
        approvals[0].preview.contains("+Q4 budget: 12,000 dollars."),
        "the approval shows a diff"
    );
    assert_eq!(task.plan.len(), 2);
    fixture.approve(&task, "allow-once");
    let task = fixture.run(&id);
    assert_eq!(
        task.status,
        TaskStatus::Completed,
        "{:?}",
        task.status_reason
    );
    assert_eq!(
        std::fs::read_to_string(fixture.folder.join("out/summary.md")).unwrap(),
        "Q4 budget: 12,000 dollars.\n"
    );
    let result = task.result.expect("result");
    assert_eq!(result.outcome, "deliverable");
    assert_eq!(result.artifact_ids.len(), 1);
    assert!(result.unverified.is_empty());
    let connection = fixture.connection();
    let artifacts = store::artifacts_for_tasks(&connection, std::slice::from_ref(&id)).unwrap();
    assert_eq!(artifacts[0].source_ids, vec![format!("{id}:S1")]);
    let sources = store::sources_for_tasks(&connection, std::slice::from_ref(&id)).unwrap();
    assert_eq!(sources[0].kind, "project-file");
    let actions = store::actions(&connection, &id).unwrap();
    assert!(
        actions.iter().all(|action| action.status == "succeeded"),
        "{actions:?}"
    );
    // The model saw host-authored instructions with the untrusted-data rule,
    // the curated memory, and the granted folder.
    let first = fixture.requests.lock().unwrap()[0].clone();
    let system = first["messages"][0]["content"].as_str().unwrap();
    assert!(
        system.contains("is untrusted")
            && system.contains("Prefers short reports.")
            && system.contains("rootId root-1")
    );
}

#[test]
fn an_expired_pending_approval_cannot_be_allowed_before_the_periodic_sweep() {
    let fixture = new_fixture(vec![
        sse_tool_calls(&[(
            "expired-write",
            "workspace.write",
            json!({ "rootId": "root-1", "path": "out/expired.txt", "content": "must not be written", "expectedVersion": "absent" }),
        )]),
        sse_answer("The approval expired, so I did not write the file."),
        sse_tool_calls(&[(
            "malformed-expiry-write",
            "workspace.write",
            json!({ "rootId": "root-1", "path": "out/malformed-expiry.txt", "content": "must not be written", "expectedVersion": "absent" }),
        )]),
        sse_answer("The malformed approval deadline failed closed, so I did not write the file."),
    ]);
    let connection = fixture.connection();
    for (prompt, expires_at, output_path) in [
        (
            "Write one file after approval.",
            "2000-01-01T00:00:00Z",
            "out/expired.txt",
        ),
        (
            "Write a file with a malformed approval deadline.",
            "not-a-timestamp",
            "out/malformed-expiry.txt",
        ),
    ] {
        let id = fixture.start(prompt, true);
        let task = fixture.run(&id);
        assert_eq!(task.status, TaskStatus::WaitingApproval);
        let approval = store::approvals_for_task(&connection, &id)
            .unwrap()
            .into_iter()
            .find(|approval| approval.status == "pending")
            .expect("approval is pending");
        connection
            .execute(
                "UPDATE approvals SET expires_at = ?1 WHERE id = ?2",
                rusqlite::params![expires_at, approval.id],
            )
            .unwrap();

        let error = store::decide_approval(&connection, &approval.id, "approved", "allow-once")
            .unwrap_err();
        assert!(error.starts_with("APPROVAL_EXPIRED:"), "{error}");
        assert_eq!(
            store::approval(&connection, &approval.id).unwrap().status,
            "pending",
            "a rejected direct decision remains pending until the sweep"
        );

        let app = fixture.app.handle().clone();
        super::runner::expire_approvals(&app, &connection);
        assert_eq!(
            store::approval(&connection, &approval.id).unwrap().status,
            "expired"
        );
        let queued = store::load_task(&connection, &id).unwrap();
        assert_eq!(queued.status, TaskStatus::Queued);
        assert!(
            queued
                .status_reason
                .as_deref()
                .unwrap_or_default()
                .contains("expired")
        );

        let resumed = fixture.run(&id);
        assert_eq!(
            resumed.status,
            TaskStatus::Completed,
            "{:?}",
            resumed.status_reason
        );
        assert!(
            resumed.pending.is_none(),
            "the expired action must leave the pending cursor"
        );
        assert!(resumed.transcript.iter().any(|message| {
            message.role == "tool" && message.content.contains("APPROVAL_EXPIRED")
        }));
        assert_eq!(
            store::actions(&connection, &id).unwrap()[0].status,
            "denied",
            "the expired write is recorded as denied, never started"
        );
        assert!(!fixture.folder.join(output_path).exists());
    }
}

#[test]
fn a_denied_action_stays_denied_and_a_changed_file_invalidates_its_approval() {
    let script = vec![
        sse_tool_calls(&[(
            "c1",
            "workspace.write",
            json!({ "rootId": "root-1", "path": "plan.md", "content": "v1\n", "expectedVersion": "absent" }),
        )]),
        sse_tool_calls(&[(
            "c2",
            "workspace.write",
            json!({ "rootId": "root-1", "path": "plan.md", "content": "v2\n", "expectedVersion": "absent" }),
        )]),
        sse_answer("I could not write the file."),
    ];
    let fixture = new_fixture(script);
    let id = fixture.start("Write a plan file.", true);
    let task = fixture.run(&id);
    fixture.approve(&task, "deny");
    let task = fixture.run(&id);
    assert_eq!(
        task.status,
        TaskStatus::WaitingApproval,
        "the model tried again and must ask again"
    );
    assert!(!fixture.folder.join("plan.md").exists());
    // The file appears after the preview; the approval no longer matches.
    std::fs::write(fixture.folder.join("plan.md"), "someone else's edit\n").unwrap();
    fixture.approve(&task, "allow-once");
    let task = fixture.run(&id);
    assert_eq!(
        std::fs::read_to_string(fixture.folder.join("plan.md")).unwrap(),
        "someone else's edit\n"
    );
    assert_eq!(task.status, TaskStatus::Completed);
    let actions = store::actions(&fixture.connection(), &id).unwrap();
    assert_eq!(actions[0].status, "denied");
    assert_eq!(
        actions[1].status, "failed",
        "a write against a changed file is refused, not applied"
    );
}

#[test]
fn citations_without_sources_are_reported_and_budgets_stop_runaway_tasks() {
    let fixture = new_fixture(vec![sse_answer("The answer is 42 [S9].")]);
    let id = fixture.start("What is the answer?", false);
    let task = fixture.run(&id);
    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(task.result.unwrap().unverified, vec!["S9"]);
    let calls: Vec<String> = (0..3)
        .map(|index| {
            sse_tool_calls(&[(
                &format!("k{index}"),
                "calculator.evaluate",
                json!({ "expression": "1+1" }),
            )])
        })
        .collect();
    let fixture = fixture_with_budget(calls, 2);
    let task = fixture.run(&fixture.start("Keep calculating.", false));
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.result.unwrap().outcome, "partial");
    assert!(
        task.status_reason
            .unwrap()
            .contains("limit of 2 model steps")
    );
}

/// F-01 (review of 406a0be): a reply that claims a save no tool performed is
/// recorded as needing review, never as completed work. The first case is
/// the recorded gate workflow: reads and a document, then "saved as
/// `summary.md` in the project folder" with no workspace write.
#[test]
fn an_answer_claiming_an_unperformed_save_needs_review_instead_of_completing() {
    let script = vec![
        sse_tool_calls(&[(
            "r1",
            "workspace.read",
            json!({ "rootId": "root-1", "path": "notes.md" }),
        )]),
        sse_tool_calls(&[(
            "r2",
            "artifact.create",
            json!({ "title": "Summary", "fileName": "summary.md", "content": "# Summary\nQ4 budget [S1].", "sourceIds": ["S1"] }),
        )]),
        sse_answer(
            "The summary has already been saved as `summary.md` in the project folder [S1].",
        ),
    ];
    let fixture = new_fixture(script);
    let id = fixture.start(
        "Summarize the notes and save summary.md in the project.",
        true,
    );
    let task = fixture.run(&id);
    assert!(task.needs_review(), "{:?} {:?}", task.status, task.result);
    let result = task.result.clone().unwrap();
    assert_eq!(result.outcome, "needs-review");
    assert!(
        result
            .unverified
            .contains(&"claim: saved or changed a file".to_owned())
    );
    assert!(
        task.status_reason
            .as_deref()
            .unwrap()
            .contains("no tool in this task did that")
    );
    assert!(!fixture.folder.join("summary.md").exists());
    let connection = fixture.connection();
    let events = store::events(&connection, &id, 100).unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.kind == "status" && event.summary.starts_with("Needs review")),
        "the status event says Needs review"
    );
    assert!(
        !events
            .iter()
            .any(|event| event.kind == "status" && event.summary.starts_with("Completed")),
        "no status event calls it completed"
    );
    let inbox = store::inbox(&connection, 20).unwrap();
    assert!(
        inbox
            .iter()
            .any(|item| item.kind == "review" && item.title.starts_with("Needs review"))
    );

    // A tool-free reply claiming a save: the same.
    let fixture = new_fixture(vec![sse_answer(
        "I've saved the cleaned notes to notes-clean.md in your project folder.",
    )]);
    let task = fixture.run(&fixture.start("Clean up my notes.", true));
    assert!(task.needs_review());
    // A tool-free reply claiming it sent an email: no tool can send here.
    let fixture = new_fixture(vec![sse_answer("Done. I emailed the report to Dana.")]);
    let task = fixture.run(&fixture.start("Email the report to Dana.", false));
    assert_eq!(
        task.result.unwrap().unverified,
        vec!["claim: sent or posted something"]
    );
    // An answer that only describes or offers an effect is still completed.
    let fixture = new_fixture(vec![sse_answer(
        "The file was not saved. I can save it as summary.md if you approve a write.",
    )]);
    let task = fixture.run(&fixture.start("Summarize.", false));
    assert_eq!(task.status, TaskStatus::Completed);
    assert!(!task.needs_review());
    assert_eq!(task.result.unwrap().outcome, "answer");
}

/// F-03 (review of 406a0be): under "Within projects", an ordinary project
/// file is written without a prompt, but a build manifest asks with its diff
/// every time (no standing grant offered or honoured), and a shell start-up
/// file is refused.
#[test]
fn the_project_preset_never_writes_build_manifests_or_shell_files_without_approval() {
    let with_project_preset = |script: Vec<String>| {
        let fixture = new_fixture(script);
        let mut profile = AgentProfile::default_for("agent-1");
        profile.autonomy = "project".into();
        store::save_agent_profile(&fixture.connection(), &profile, 0).unwrap();
        // Even a standing write permission for this project does not cover
        // the ask-every-time class.
        store::insert_grant(
            &fixture.connection(),
            &Grant {
                id: "grant-write".into(),
                assistant_id: "agent-1".into(),
                project_id: Some("project-1".into()),
                capability: "workspace.write".into(),
                scope: json!({ "projectId": "project-1" }),
                created_at: store::now(),
                expires_at: None,
                revoked_at: None,
            },
        )
        .unwrap();
        fixture
    };
    let write = |path: &str| {
        sse_tool_calls(&[(
            "w1",
            "workspace.write",
            json!({ "rootId": "root-1", "path": path, "content": "x\n", "expectedVersion": "absent" }),
        )])
    };

    let fixture = with_project_preset(vec![
        write("out/notes.md"),
        sse_answer("Wrote out/notes.md."),
    ]);
    let task = fixture.run(&fixture.start("Write notes.", true));
    assert_eq!(
        task.status,
        TaskStatus::Completed,
        "{:?}",
        task.status_reason
    );
    assert!(fixture.folder.join("out/notes.md").exists());

    let fixture = with_project_preset(vec![write("package.json")]);
    let task = fixture.run(&fixture.start("Add a build script.", true));
    assert_eq!(
        task.status,
        TaskStatus::WaitingApproval,
        "{:?}",
        task.status_reason
    );
    let approval = store::approvals_for_task(&fixture.connection(), &task.id)
        .unwrap()
        .into_iter()
        .find(|approval| approval.status == "pending")
        .unwrap();
    assert!(
        !approval.standing_allowed,
        "no standing grant for manifests"
    );
    assert!(
        approval.preview.contains("+x"),
        "the approval shows the diff"
    );
    assert!(!fixture.folder.join("package.json").exists());

    let fixture = with_project_preset(vec![
        write(".bashrc"),
        sse_answer("I could not change .bashrc."),
    ]);
    let task = fixture.run(&fixture.start("Add an alias.", true));
    assert!(!fixture.folder.join(".bashrc").exists());
    let actions = store::actions(&fixture.connection(), &task.id).unwrap();
    assert_ne!(actions[0].status, "succeeded");
    assert!(
        actions[0]
            .result
            .as_ref()
            .is_some_and(|result| result.to_string().contains("PROTECTED_PATH")),
        "{:?}",
        actions[0].result
    );
}

fn fixture_with_budget(script: Vec<String>, steps: u32) -> Fixture {
    let fixture = new_fixture(script);
    let mut profile = AgentProfile::default_for("agent-1");
    profile.budget.max_steps = steps;
    store::save_agent_profile(&fixture.connection(), &profile, 0).unwrap();
    fixture
}

/// Delegation is opt-in per agent.
fn with_delegation(fixture: Fixture) -> Fixture {
    let mut profile = AgentProfile::default_for("agent-1");
    profile.enabled_tools.push("agent.delegate".into());
    store::save_agent_profile(&fixture.connection(), &profile, 0).unwrap();
    fixture
}

#[test]
fn delegated_work_joins_within_role_and_parent_limits_and_cancellation_reaches_children() {
    let script = vec![
        sse_tool_calls(&[(
            "p1",
            "agent.delegate",
            json!({ "role": "file-analyst", "objective": "Find the Q4 budget in the notes.", "context": "Look in notes.md" }),
        )]),
        // The child's turn.
        sse_tool_calls(&[(
            "k1",
            "workspace.read",
            json!({ "rootId": "root-1", "path": "notes.md" }),
        )]),
        sse_answer("The Q4 budget is 12,000 dollars [S1]."),
        // The parent resumes with the child's report.
        sse_answer("The helper found that the Q4 budget is 12,000 dollars [S1]."),
    ];
    let fixture = with_delegation(new_fixture(script));
    let id = fixture.start("Use a helper to find the budget.", true);
    let parent = fixture.run(&id);
    assert_eq!(
        parent.status,
        TaskStatus::Queued,
        "the parent waits for its subtask"
    );
    let children = store::children(&fixture.connection(), &id).unwrap();
    assert_eq!(children.len(), 1);
    let child = fixture.run(&children[0].id);
    assert_eq!(
        child.status,
        TaskStatus::Completed,
        "{:?}",
        child.status_reason
    );
    assert!(child.budget.max_steps < parent.budget.max_steps);
    assert!(
        child.budget.max_steps <= parent.budget.max_steps.saturating_sub(parent.usage.steps) / 2
    );
    assert!(
        child.budget.max_tool_calls
            <= parent
                .budget
                .max_tool_calls
                .saturating_sub(parent.usage.tool_calls)
                / 2
    );
    assert!(
        child.budget.max_tokens
            <= parent
                .budget
                .max_tokens
                .saturating_sub(parent.usage.tokens())
                / 2
    );
    assert!(
        child.budget.max_seconds
            <= parent
                .budget
                .max_seconds
                .saturating_sub(parent.usage.active_ms / 1000)
                / 2
    );
    assert!(
        child.budget.max_artifacts
            <= parent
                .budget
                .max_artifacts
                .saturating_sub(parent.usage.artifacts)
                / 2
    );
    assert!(child.provenance.tools.iter().all(|tool| {
        super::policy::role_tools("file-analyst").contains(&tool.as_str())
            || tool == "task.update_plan"
    }));
    assert!(
        !child
            .provenance
            .tools
            .contains(&"workspace.write".to_owned())
    );
    // The child saw only what it was given, not the parent's conversation.
    let child_request = fixture.requests.lock().unwrap()[1].clone();
    let transcript = child_request["messages"].to_string();
    assert!(!transcript.contains("Use a helper to find the budget."));
    let parent = fixture.run(&id);
    assert_eq!(
        parent.status,
        TaskStatus::Completed,
        "{:?}",
        parent.status_reason
    );
    assert!(parent.usage.steps >= child.usage.steps + 2);
    assert!(parent.usage.tool_calls > child.usage.tool_calls);
    assert!(parent.usage.tokens() > child.usage.tokens());
    assert!(parent.usage.active_ms >= child.usage.active_ms);
    assert!(
        parent.result.unwrap().unverified.is_empty(),
        "a child's source is citable by the parent"
    );

    // Cancellation reaches queued descendants.
    let fixture = with_delegation(new_fixture(vec![sse_tool_calls(&[(
        "p1",
        "agent.delegate",
        json!({ "role": "researcher", "objective": "Research something." }),
    )])]));
    let id = fixture.start("Delegate and stop.", true);
    fixture.run(&id);
    let connection = fixture.connection();
    let mut parent = store::load_task(&connection, &id).unwrap();
    super::runner::cancel_tree(
        &connection,
        &fixture.app.state::<AppState>().agent,
        &mut parent,
        "Cancelled by you.",
    )
    .unwrap();
    let children = store::children(&connection, &id).unwrap();
    assert!(
        children
            .iter()
            .all(|child| child.status == TaskStatus::Cancelled)
    );
    assert_eq!(
        store::load_task(&connection, &id).unwrap().status,
        TaskStatus::Cancelled
    );
}

#[test]
fn delegation_is_denied_when_remaining_tool_budget_cannot_fund_a_child() {
    let fixture = with_delegation(new_fixture(vec![sse_tool_calls(&[(
        "p1",
        "agent.delegate",
        json!({ "role": "researcher", "objective": "Search the web." }),
    )])]));
    let id = fixture.start("Use one helper.", true);
    let connection = fixture.connection();
    let mut task = store::load_task(&connection, &id).unwrap();
    task.budget.max_tool_calls = 1;
    store::save_task(&connection, &mut task).unwrap();

    let task = fixture.run(&id);

    assert_eq!(task.usage.tool_calls, 1);
    assert!(store::children(&connection, &id).unwrap().is_empty());
}

#[test]
fn action_result_and_task_cursor_commit_in_one_transaction() {
    let fixture = new_fixture(vec![]);
    let id = fixture.start("Checkpoint a result.", false);
    let connection = fixture.connection();
    let mut task = store::load_task(&connection, &id).unwrap();
    store::transition(&connection, &mut task, TaskStatus::Running, None).unwrap();
    let call = TranscriptToolCall {
        id: "call-checkpoint".into(),
        name: "workspace.write".into(),
        arguments: json!({ "rootId": "root-1", "path": "answer.txt" }),
    };
    task.transcript.push(TranscriptMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![call.clone()],
        tool_call_id: None,
        tool_name: None,
    });
    task.pending = Some(Pending {
        calls: vec![call.clone()],
        next: 0,
        wait: None,
    });
    store::save_task(&connection, &mut task).unwrap();
    let action = TaskAction {
        id: store::new_id("action"),
        task_id: id.clone(),
        step: 1,
        call_id: call.id.clone(),
        tool: call.name.clone(),
        arguments: call.arguments.clone(),
        binding: "binding".into(),
        status: "started".into(),
        result: None,
        created_at: store::now(),
        updated_at: store::now(),
    };
    store::insert_action(&connection, &action).unwrap();

    connection
        .execute_batch(
            "CREATE TRIGGER reject_action_event BEFORE INSERT ON task_events
             WHEN NEW.kind = 'action' BEGIN SELECT RAISE(ABORT, 'simulated checkpoint failure'); END;",
        )
        .unwrap();
    let mut next = task.clone();
    next.transcript.push(TranscriptMessage {
        role: "tool".into(),
        content: "{\"result\":{\"written\":true}}".into(),
        tool_calls: Vec::new(),
        tool_call_id: Some(call.id.clone()),
        tool_name: Some(call.name.clone()),
    });
    next.pending.as_mut().unwrap().next = 1;
    let result = json!({ "result": { "written": true } });
    assert!(
        store::checkpoint_tool_result(
            &connection,
            &next,
            Some((&action.id, "succeeded", &result)),
            "Wrote answer.txt",
            None,
        )
        .is_err()
    );
    let rolled_back = store::load_task(&connection, &id).unwrap();
    assert_eq!(rolled_back.pending.as_ref().unwrap().next, 0);
    assert!(
        !rolled_back
            .transcript
            .iter()
            .any(|message| message.role == "tool")
    );
    assert_eq!(
        store::actions(&connection, &id).unwrap()[0].status,
        "started"
    );

    connection
        .execute_batch("DROP TRIGGER reject_action_event;")
        .unwrap();
    let committed = store::checkpoint_tool_result(
        &connection,
        &next,
        Some((&action.id, "succeeded", &result)),
        "Wrote answer.txt",
        None,
    )
    .unwrap();
    assert_eq!(committed.pending.as_ref().unwrap().next, 1);
    assert!(
        committed
            .transcript
            .iter()
            .any(|message| message.role == "tool")
    );
    assert_eq!(
        store::actions(&connection, &id).unwrap()[0].status,
        "succeeded"
    );
}

#[test]
fn delegated_child_and_parent_wait_checkpoint_commit_together() {
    let fixture = new_fixture(vec![]);
    let parent_id = fixture.start("Delegate safely.", false);
    let connection = fixture.connection();
    let mut parent = store::load_task(&connection, &parent_id).unwrap();
    store::transition(&connection, &mut parent, TaskStatus::Running, None).unwrap();

    let child_id = store::new_id("task");
    parent.usage.children = 1;
    parent.pending = Some(Pending {
        calls: vec![],
        next: 0,
        wait: Some(Wait::Children {
            task_ids: vec![child_id.clone()],
        }),
    });
    let mut child = parent.clone();
    child.id = child_id.clone();
    child.parent_id = Some(parent_id.clone());
    child.root_id = parent.root_id.clone();
    child.depth = 1;
    child.role = Some("researcher".into());
    child.status = TaskStatus::Queued;
    child.status_reason = None;
    child.current_action = None;
    child.usage = TaskUsage::default();
    child.transcript = vec![TranscriptMessage {
        role: "user".into(),
        content: "Find a source.".into(),
        tool_calls: vec![],
        tool_call_id: None,
        tool_name: None,
    }];
    child.pending = None;
    child.result = None;
    child.revision = 1;
    child.created_at = store::now();
    child.updated_at = child.created_at.clone();
    child.finished_at = None;

    connection
        .execute_batch(&format!(
            "CREATE TRIGGER reject_parent_spawn BEFORE UPDATE ON tasks WHEN OLD.id = '{parent_id}' BEGIN SELECT RAISE(ABORT, 'simulated parent checkpoint failure'); END;"
        ))
        .unwrap();
    assert!(store::checkpoint_child_spawn(&connection, &parent, &child).is_err());
    assert!(store::children(&connection, &parent_id).unwrap().is_empty());
    assert_eq!(
        store::load_task(&connection, &parent_id).unwrap().status,
        TaskStatus::Running
    );

    connection
        .execute_batch("DROP TRIGGER reject_parent_spawn;")
        .unwrap();
    let committed = store::checkpoint_child_spawn(&connection, &parent, &child).unwrap();
    assert_eq!(committed.status, TaskStatus::Queued);
    assert_eq!(committed.usage.children, 1);
    assert!(matches!(
        committed.pending.as_ref().and_then(|pending| pending.wait.as_ref()),
        Some(Wait::Children { task_ids }) if task_ids == &[child_id]
    ));
    assert_eq!(store::children(&connection, &parent_id).unwrap().len(), 1);
}

#[test]
fn a_restart_interrupts_running_work_and_resume_reconciles_an_uncertain_write() {
    let script = vec![
        sse_tool_calls(&[(
            "c1",
            "workspace.write",
            json!({ "rootId": "root-1", "path": "report.md", "content": "final\n", "expectedVersion": "absent" }),
        )]),
        sse_answer("Done."),
    ];
    let fixture = new_fixture(script);
    let id = fixture.start("Write the report.", true);
    let task = fixture.run(&id);
    fixture.approve(&task, "allow-once");
    // Simulate a crash after the write took effect but before its result was
    // committed: the action is `started` and the task `running`.
    let connection = fixture.connection();
    std::fs::write(fixture.folder.join("report.md"), "final\n").unwrap();
    let action = store::actions(&connection, &id).unwrap().remove(0);
    store::set_action(&connection, &action.id, "started", None).unwrap();
    let mut task = store::load_task(&connection, &id).unwrap();
    store::transition(&connection, &mut task, TaskStatus::Running, None).unwrap();
    assert_eq!(store::recover_after_restart(&connection).unwrap(), 1);
    let task = store::load_task(&connection, &id).unwrap();
    assert_eq!(task.status, TaskStatus::Interrupted);
    assert_eq!(
        store::actions(&connection, &id).unwrap()[0].status,
        "uncertain"
    );
    // Resume: the host checks the file, finds the write applied, and does
    // not write again.
    let handle = fixture.app.handle().clone();
    super::commands::task_resume(handle, fixture.app.state::<AppState>(), id.clone()).unwrap();
    let task = fixture.run(&id);
    assert_eq!(
        task.status,
        TaskStatus::Completed,
        "{:?}",
        task.status_reason
    );
    let actions = store::actions(&fixture.connection(), &id).unwrap();
    assert_eq!(actions[0].status, "succeeded");
    assert_eq!(
        fixture.requests.lock().unwrap().len(),
        2,
        "no extra model turn was needed to repeat the write"
    );
}

#[test]
fn resuming_after_a_partial_uncertain_reconciliation_keeps_the_action_outcome() {
    let fixture = new_fixture(vec![sse_answer("The action was already completed.")]);
    let id = fixture.start("Write the report.", true);
    let connection = fixture.connection();
    let mut task = store::load_task(&connection, &id).unwrap();
    store::transition(&connection, &mut task, TaskStatus::Running, None).unwrap();

    let call = TranscriptToolCall {
        id: "call-recovered".into(),
        name: "workspace.write".into(),
        arguments: json!({
            "rootId": "root-1",
            "path": "report.md",
            "content": "already written\n",
            "expectedVersion": "absent"
        }),
    };
    task.usage.steps = 1;
    task.usage.tool_calls = 1;
    task.transcript.push(TranscriptMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![call.clone()],
        tool_call_id: None,
        tool_name: None,
    });
    task.pending = Some(Pending {
        calls: vec![call.clone()],
        next: 0,
        wait: None,
    });
    store::save_task(&connection, &mut task).unwrap();

    // Simulate a crash after the recovery helper marked an already-applied
    // write successful but before it saved the transcript/cursor advancement.
    store::insert_action(
        &connection,
        &TaskAction {
            id: "action-recovered".into(),
            task_id: id.clone(),
            step: 1,
            call_id: call.id.clone(),
            tool: call.name.clone(),
            arguments: call.arguments.clone(),
            binding: "binding".into(),
            status: "succeeded".into(),
            result: Some(crate::tools::host_result(
                &call.id,
                &call.name,
                "success",
                Some(json!({ "note": "The write was already applied." })),
                None,
            )),
            created_at: store::now(),
            updated_at: store::now(),
        },
    )
    .unwrap();
    let mut task = store::load_task(&connection, &id).unwrap();
    store::transition(&connection, &mut task, TaskStatus::Interrupted, None).unwrap();

    let handle = fixture.app.handle().clone();
    super::commands::task_resume(handle, fixture.app.state::<AppState>(), id.clone()).unwrap();
    let task = fixture.run(&id);

    assert_eq!(
        task.status,
        TaskStatus::Completed,
        "{:?}",
        task.status_reason
    );
    assert_eq!(
        store::actions(&fixture.connection(), &id).unwrap()[0].status,
        "succeeded"
    );
    assert!(task.transcript.iter().any(|message| {
        message.role == "tool" && message.tool_call_id.as_deref() == Some("call-recovered")
    }));
}

#[test]
fn a_question_waits_for_the_user_and_an_unknown_tool_is_denied() {
    let script = vec![
        sse_tool_calls(&[
            (
                "c1",
                "task.ask_user",
                json!({ "question": "Which quarter?" }),
            ),
            ("c2", "shell.exec", json!({ "command": "rm -rf /" })),
        ]),
        sse_answer("Q4 it is."),
    ];
    let fixture = new_fixture(script);
    let id = fixture.start("Report the budget.", false);
    let task = fixture.run(&id);
    assert_eq!(task.status, TaskStatus::WaitingInput);
    let connection = fixture.connection();
    let mut task = store::load_task(&connection, &id).unwrap();
    if let Some(Pending {
        wait: Some(Wait::Input { answer, .. }),
        ..
    }) = &mut task.pending
    {
        *answer = Some("Q4".into());
    }
    store::transition(&connection, &mut task, TaskStatus::Queued, None).unwrap();
    let task = fixture.run(&id);
    assert_eq!(task.status, TaskStatus::Completed);
    let actions = store::actions(&fixture.connection(), &id).unwrap();
    let unknown = actions
        .iter()
        .find(|action| action.tool == "shell.exec")
        .unwrap();
    assert_eq!(unknown.status, "failed");
    assert_eq!(
        unknown.result.as_ref().unwrap()["error"]["code"],
        "TOOL_NOT_ENABLED"
    );
}

#[cfg(unix)]
#[test]
fn a_plugin_tool_needs_approval_a_standing_grant_binds_to_its_tool_set_and_disabling_blocks_it() {
    use super::mcp;
    let server = format!(
        "{}/tests/fixtures/mcp_fixture_server.py",
        env!("CARGO_MANIFEST_DIR")
    );
    let plugin_id = Arc::new(Mutex::new(String::new()));
    let recorded = plugin_id.clone();
    let fixture = new_fixture_with(move |database| {
        let connection = store::open(database).unwrap();
        let plugin = mcp::register(
            &connection,
            mcp::PluginConfig {
                name: "Fixture".into(),
                kind: "mcp-stdio".into(),
                command: Some("/usr/bin/python3".into()),
                args: vec![server, "--era".into(), "modern".into()],
                env: vec![],
                url: None,
                credential_ref: None,
            },
        )
        .unwrap();
        let tested = tauri::async_runtime::block_on(mcp::test_connection(
            database.to_path_buf(),
            plugin.id.clone(),
        ))
        .unwrap();
        assert_eq!(tested.state.status, "connected");
        mcp::set_enabled(&connection, &plugin.id, true).unwrap();
        let echo = mcp::exposed_name(&plugin.id, "echo");
        let inject = mcp::exposed_name(&plugin.id, "inject");
        *recorded.lock().unwrap() = plugin.id.clone();
        vec![
            sse_tool_calls(&[("c1", &echo, json!({ "text": "first" }))]),
            sse_tool_calls(&[
                ("c2", &echo, json!({ "text": "second" })),
                ("c3", &inject, json!({})),
            ]),
            sse_answer("The server said first and second."),
            sse_tool_calls(&[("d1", &echo, json!({ "text": "after disable" }))]),
            sse_answer("The tool was unavailable."),
        ]
    });
    let plugin_id = plugin_id.lock().unwrap().clone();
    let echo = mcp::exposed_name(&plugin_id, "echo");
    let inject = mcp::exposed_name(&plugin_id, "inject");
    let mut profile = AgentProfile::default_for("agent-1");
    profile.enabled_tools.extend([echo.clone(), inject.clone()]);
    store::save_agent_profile(&fixture.connection(), &profile, 0).unwrap();

    let id = fixture.start("Echo twice.", false);
    let task = fixture.run(&id);
    assert_eq!(
        task.status,
        TaskStatus::WaitingApproval,
        "a plugin call asks first: {:?}",
        task.status_reason
    );
    let approval = store::approvals_for_task(&fixture.connection(), &id)
        .unwrap()
        .remove(0);
    assert!(approval.standing_allowed && approval.preview.contains("echo"));
    // Allow for this agent: a standing grant bound to the server's tool set.
    fixture.approve(&task, "allow-standing");
    let task = fixture.run(&id);
    // The second echo ran under the grant; `inject` is a different tool and asks.
    assert_eq!(task.status, TaskStatus::WaitingApproval);
    let grants = store::grants(&fixture.connection(), false).unwrap();
    assert_eq!(grants.len(), 1);
    assert!(grants[0].scope["toolsDigest"].is_string());
    fixture.approve(&task, "allow-once");
    let task = fixture.run(&id);
    assert_eq!(
        task.status,
        TaskStatus::Completed,
        "{:?}",
        task.status_reason
    );
    let actions = store::actions(&fixture.connection(), &id).unwrap();
    assert_eq!(
        actions
            .iter()
            .filter(|action| action.status == "succeeded")
            .count(),
        3
    );
    let injected = actions.iter().find(|action| action.tool == inject).unwrap();
    assert!(
        injected.result.as_ref().unwrap()["result"]["note"]
            .as_str()
            .unwrap()
            .contains("untrusted")
    );
    // The injected text did not become authority: no new grants appeared.
    assert_eq!(
        store::grants(&fixture.connection(), false).unwrap().len(),
        1
    );

    // A changed tool set (new digest) no longer matches the standing grant.
    let connection = fixture.connection();
    let context = super::policy::DecisionContext {
        profile: &profile,
        assistant_id: "agent-1",
        project_id: None,
        grants: &store::grants(&connection, false).unwrap(),
        scope: json!({ "toolsDigest": "sha256:changed", "server": plugin_id }),
        offered: true,
    };
    assert_eq!(
        super::policy::decide(
            super::policy::Risk::Plugin,
            &format!("mcp:{plugin_id}:echo"),
            &context
        ),
        super::policy::Decision::Ask { standing: true }
    );

    // Turning the server off removes its tools from later tasks.
    mcp::set_enabled(&connection, &plugin_id, false).unwrap();
    let second = fixture.start("Echo again.", false);
    let task = fixture.run(&second);
    assert_eq!(task.status, TaskStatus::Completed);
    let blocked = store::actions(&fixture.connection(), &second)
        .unwrap()
        .remove(0);
    assert_eq!(blocked.result.unwrap()["error"]["code"], "TOOL_NOT_ENABLED");
    mcp::disconnect(&plugin_id);
}

#[cfg(unix)]
#[test]
fn a_standing_plugin_grant_does_not_survive_replacing_the_stdio_executable() {
    use super::mcp;
    use std::os::unix::fs::PermissionsExt;

    let server = format!(
        "{}/tests/fixtures/mcp_fixture_server.py",
        env!("CARGO_MANIFEST_DIR")
    );
    let plugin_id = Arc::new(Mutex::new(String::new()));
    let recorded_id = plugin_id.clone();
    let executable = Arc::new(Mutex::new(PathBuf::new()));
    let recorded_executable = executable.clone();
    let fixture = new_fixture_with(move |database| {
        let path = database.parent().unwrap().join("mutable-mcp-server");
        std::fs::write(&path, "#!/bin/sh\nexec /usr/bin/python3 \"$@\"\n").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();

        let connection = store::open(database).unwrap();
        let plugin = mcp::register(
            &connection,
            mcp::PluginConfig {
                name: "Mutable fixture".into(),
                kind: "mcp-stdio".into(),
                command: Some(path.to_string_lossy().into_owned()),
                args: vec![server, "--era".into(), "modern".into()],
                env: vec![],
                url: None,
                credential_ref: None,
            },
        )
        .unwrap();
        let tested = tauri::async_runtime::block_on(mcp::test_connection(
            database.to_path_buf(),
            plugin.id.clone(),
        ))
        .unwrap();
        assert_eq!(tested.state.status, "connected");
        mcp::set_enabled(&connection, &plugin.id, true).unwrap();
        let echo = mcp::exposed_name(&plugin.id, "echo");
        *recorded_id.lock().unwrap() = plugin.id;
        *recorded_executable.lock().unwrap() = path;
        vec![
            sse_tool_calls(&[("grant-call", &echo, json!({ "text": "first" }))]),
            sse_answer("First call completed."),
            sse_tool_calls(&[("replacement-call", &echo, json!({ "text": "second" }))]),
        ]
    });

    let plugin_id = plugin_id.lock().unwrap().clone();
    let executable = executable.lock().unwrap().clone();
    let echo = mcp::exposed_name(&plugin_id, "echo");
    let mut profile = AgentProfile::default_for("agent-1");
    profile.enabled_tools.push(echo.clone());
    store::save_agent_profile(&fixture.connection(), &profile, 0).unwrap();

    let first = fixture.start("Use the plugin once.", false);
    let waiting = fixture.run(&first);
    assert_eq!(waiting.status, TaskStatus::WaitingApproval);
    fixture.approve(&waiting, "allow-standing");
    assert_eq!(fixture.run(&first).status, TaskStatus::Completed);
    let grants = store::grants(&fixture.connection(), false).unwrap();
    assert_eq!(grants.len(), 1);
    assert!(grants[0].scope["targetDigest"].is_string());

    // Keep the same path, server id, and tool list, but change the bytes that
    // will be executed. The old standing grant must ask again.
    let replacement =
        "#!/bin/sh\n# replacement executable identity\nexec /usr/bin/python3 \"$@\"\n";
    std::fs::write(&executable, replacement).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let second = fixture.start("Use the plugin after replacement.", false);
    let task = fixture.run(&second);
    assert_eq!(task.status, TaskStatus::WaitingApproval);
    let approval = store::approvals_for_task(&fixture.connection(), &second)
        .unwrap()
        .into_iter()
        .find(|approval| approval.status == "pending")
        .unwrap();
    assert_eq!(approval.tool, echo);
    assert!(approval.standing_allowed);
    assert!(
        store::actions(&fixture.connection(), &second)
            .unwrap()
            .iter()
            .all(|action| action.status != "succeeded")
    );
    mcp::disconnect(&plugin_id);
}

#[test]
fn a_schedule_proposed_in_conversation_is_created_only_after_approval_with_concrete_runs() {
    let script = vec![
        sse_tool_calls(&[(
            "s1",
            "schedule.create",
            json!({ "title": "Morning brief", "prompt": "Summarize my notes.", "kind": "weekly", "days": [1, 3, 5], "time": "08:30" }),
        )]),
        sse_answer("Scheduled."),
    ];
    let fixture = new_fixture(script);
    store::set_agent_settings(
        &fixture.connection(),
        &json!({ "timezone": "America/New_York" }),
    )
    .unwrap();
    let id = fixture.start(
        "Every Monday, Wednesday, and Friday at 8:30 in the morning, summarize my notes.",
        false,
    );
    let task = fixture.run(&id);
    assert_eq!(task.status, TaskStatus::WaitingApproval);
    assert!(
        super::schedule::list(&fixture.connection())
            .unwrap()
            .is_empty(),
        "nothing is scheduled before approval"
    );
    let approval = store::approvals_for_task(&fixture.connection(), &id)
        .unwrap()
        .remove(0);
    assert!(
        !approval.standing_allowed,
        "schedules are approved one at a time"
    );
    assert!(
        approval.preview.contains("Next runs:") && approval.preview.contains("8:30 AM"),
        "{}",
        approval.preview
    );
    fixture.approve(&task, "allow-once");
    let task = fixture.run(&id);
    assert_eq!(task.status, TaskStatus::Completed);
    let schedules = super::schedule::list(&fixture.connection()).unwrap();
    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules[0].spec.timezone, "America/New_York");
    assert_eq!(schedules[0].spec.assistant_id, "agent-1");
}
