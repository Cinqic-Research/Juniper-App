//! End-to-end runner tests against a scripted OpenAI-compatible fixture
//! server. The fixture stands in for a model: it returns the tool calls and
//! answers each test scripts, so every host decision — policy, approvals,
//! budgets, delegation, recovery — runs for real against real files.

use super::store;
use super::types::*;
use crate::commands::AppState;
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
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
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
