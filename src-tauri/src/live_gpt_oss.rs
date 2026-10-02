//! Hardware tests and the wrapper evaluation for the qualified gpt-oss-20b
//! backend. Ignored by default; see `tests/wrapper-eval/README.md` for setup.
//!
//! They drive the real resident runtime (`local_runtime::stream_chat`), so
//! the model file must be installed in `$XDG_DATA_HOME/models` under its
//! catalog name and `JUNIPER_LLAMA_SERVER_CUDA` must name the qualified
//! llama.cpp b11270 CUDA build.

use crate::commands::{AppState, Cancellation};
use crate::domain::{AttachmentContext, ChatMessage, ChatRequest, GenerationOverrides};
use crate::local_runtime;
use regex::RegexBuilder;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Listener;
use tokio::time::Duration;

const CASES: &str = include_str!("../../tests/wrapper-eval/cases.v1.jsonl");

fn require_hardware() {
    assert_eq!(
        std::env::var("JUNIPER_LIVE_GPT_OSS").as_deref(),
        Ok("1"),
        "set JUNIPER_LIVE_GPT_OSS=1 on a machine with the qualified model and runtime"
    );
}

fn request(id: &str, profile: Option<&str>, messages: Vec<(&str, String)>) -> ChatRequest {
    let mut all = Vec::new();
    if let Some(profile) = profile {
        all.push(ChatMessage {
            role: "system".into(),
            content: profile.into(),
        });
    }
    all.extend(messages.into_iter().map(|(role, content)| ChatMessage {
        role: role.into(),
        content,
    }));
    serde_json::from_value::<ChatRequest>(json!({
        "requestId": id,
        "assistantId": "assistant-juniper",
        "assistantName": "Juniper",
        "conversationId": format!("conversation-{id}"),
        "provider": {
            "id": "juniper-local", "name": "Juniper on this device", "kind": "juniper-local",
            "baseUrl": "http://127.0.0.1", "locality": "local", "transportLocation": "on-device"
        },
        "model": {
            "id": "juniper-local:gpt-oss-20b", "providerId": "juniper-local",
            "modelId": "gpt-oss-20b", "displayName": "gpt-oss-20b", "catalogId": "gpt-oss-20b",
            "executionLocation": "on-device",
            "capabilities": { "tools": "supported", "thinking": "supported", "generationParameters": ["maxOutput"] }
        },
        "messages": all,
        "tools": [],
        "generation": { "maxOutput": 2048 }
    }))
    .expect("live request")
}

fn builtin_tool(name: &str) -> crate::domain::ToolDefinition {
    // Mirrors `builtinTools` in src/lib/defaults.ts.
    let risk = match name {
        "memory.list" | "chat.search" => "user-data-read",
        "memory.save" | "memory.delete" => "user-data-write",
        "file.read" | "file.metadata" => "filesystem-read",
        _ => "automatic-safe",
    }
    .to_owned();
    let schema = match name {
        "calculator.evaluate" => {
            json!({ "type": "object", "properties": { "expression": { "type": "string", "maxLength": 256 } }, "required": ["expression"], "additionalProperties": false })
        }
        "memory.save" => {
            json!({ "type": "object", "properties": { "content": { "type": "string", "maxLength": 1000 } }, "required": ["content"], "additionalProperties": false })
        }
        "chat.search" => {
            json!({ "type": "object", "properties": { "query": { "type": "string", "maxLength": 200 } }, "required": ["query"], "additionalProperties": false })
        }
        "file.read" => {
            json!({ "type": "object", "properties": { "attachmentId": { "type": "string" } }, "required": ["attachmentId"], "additionalProperties": false })
        }
        _ => json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    };
    let description = match name {
        "calculator.evaluate" => "Safely evaluate common arithmetic without executing code.",
        "memory.save" => "Propose a user-curated memory for explicit approval.",
        "chat.search" => {
            "Search the local conversation database; unrelated chats are not included automatically."
        }
        "file.read" => "Read only a user-selected text file, capped at 1 MB.",
        _ => name,
    };
    crate::domain::ToolDefinition {
        name: name.into(),
        description: description.into(),
        risk,
        enabled: true,
        schema,
    }
}

#[derive(Debug, Default)]
struct Outcome {
    text: String,
    error: Option<String>,
    activities: Vec<String>,
    tools_called: Vec<String>,
    tool_results: Vec<Value>,
    provenance: Value,
    events: Vec<Value>,
    seconds: f64,
}

/// Runs one request through the real host path, answering permission prompts
/// from `decisions` (anything unlisted is denied).
async fn run<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &'static AppState,
    request: ChatRequest,
    decisions: HashMap<String, String>,
) -> Outcome {
    let topic = format!("juniper://chat/{}", request.request_id);
    let events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = events.clone();
    let listener = app.listen(topic, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            captured.lock().expect("events").push(value);
        }
    });
    let started = Instant::now();
    let request_id = request.request_id.clone();
    let responder_events = events.clone();
    let responder = tokio::spawn(async move {
        let mut answered = std::collections::HashSet::new();
        loop {
            let pending = responder_events
                .lock()
                .expect("events")
                .iter()
                .filter_map(|event| {
                    event
                        .get("permissionRequest")
                        .filter(|value| !value.is_null())
                        .cloned()
                })
                .collect::<Vec<_>>();
            for prompt in pending {
                let call = prompt["callId"].as_str().unwrap_or_default().to_owned();
                if !answered.insert(call.clone()) {
                    continue;
                }
                let tool = prompt["toolName"].as_str().unwrap_or_default();
                let decision = decisions
                    .get(tool)
                    .cloned()
                    .unwrap_or_else(|| "deny".into());
                let key = format!("{request_id}:{call}");
                for _ in 0..200 {
                    if let Some(sender) = state
                        .permission_waiters
                        .lock()
                        .expect("waiters")
                        .remove(&key)
                    {
                        let _ = sender.send(decision.clone());
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    });
    if let Err(error) =
        local_runtime::stream_chat(app.clone(), request.clone(), Cancellation::default(), state)
            .await
    {
        local_runtime::emit_error(app, &request.request_id, &error);
    }
    responder.abort();
    app.unlisten(listener);
    let events = events.lock().expect("events").clone();
    let mut outcome = Outcome {
        seconds: started.elapsed().as_secs_f64(),
        ..Outcome::default()
    };
    for event in &events {
        if let Some(delta) = event["delta"].as_str() {
            outcome.text.push_str(delta);
        }
        if let Some(activity) = event["activity"].as_str() {
            outcome.activities.push(activity.into());
        }
        if let Some(calls) = event["toolCalls"].as_array() {
            outcome.tools_called.extend(
                calls
                    .iter()
                    .filter_map(|call| call["name"].as_str().map(str::to_owned)),
            );
        }
        if let Some(results) = event["toolResults"].as_array() {
            outcome.tool_results.extend(results.iter().cloned());
        }
        if event["done"] == true {
            outcome.error = event["error"]["code"].as_str().map(str::to_owned);
            outcome.provenance = event["provenance"].clone();
        }
    }
    outcome.events = events;
    outcome
}

fn app_and_state() -> (tauri::App<tauri::test::MockRuntime>, &'static AppState) {
    (tauri::test::mock_app(), Box::leak(Box::default()))
}

/// Stops the resident server when a hardware test ends, including by panic;
/// the mock app never delivers the exit event that does this in the product.
struct StopOnDrop(&'static AppState);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.local_runtime.terminate_all();
    }
}

#[test]
#[ignore = "requires FLOWBOX hardware, the qualified gpt-oss-20b file, and JUNIPER_LIVE_GPT_OSS=1"]
fn live_resident_runtime_lifecycle() {
    require_hardware();
    let (app, state) = app_and_state();
    let _stop = StopOnDrop(state);
    let handle = app.handle().clone();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let cold = run(
            &handle,
            state,
            request(
                "cold",
                None,
                vec![("user", "Reply with the single word: ready".into())],
            ),
            HashMap::new(),
        )
        .await;
        println!(
            "LIVE cold seconds={:.1} activities={:?} error={:?} text={:?}",
            cold.seconds, cold.activities, cold.error, cold.text
        );
        assert_eq!(cold.error, None);
        assert!(cold.activities.contains(&"loading-model".to_owned()));
        assert!(cold.activities.contains(&"warming-up".to_owned()));
        assert_eq!(cold.provenance["backend"], "gpt-oss-20b-mxfp4-flowbox.v1");
        assert_eq!(
            cold.provenance["runtime"]["runtimeBuild"],
            "b11270-748d4225b"
        );
        assert_eq!(
            cold.provenance["runtime"]["artifactSha256"],
            "9d7364f02d9952e158ab462629e72401bec844d2243cc3854b271bb35d33d23d"
        );
        assert_eq!(cold.provenance["reasoningEffort"], "medium");
        assert_eq!(cold.provenance["constitution"], "juniper-constitution.v1");

        let warm = run(
            &handle,
            state,
            request(
                "warm",
                None,
                vec![("user", "Name one primary color. One word.".into())],
            ),
            HashMap::new(),
        )
        .await;
        println!(
            "LIVE warm seconds={:.1} activities={:?} text={:?}",
            warm.seconds, warm.activities, warm.text
        );
        assert_eq!(warm.error, None);
        assert!(
            !warm
                .activities
                .iter()
                .any(|activity| activity == "loading-model")
        );

        let mut math = request(
            "math",
            None,
            vec![("user", "What is 48173 * 2917? Use the calculator.".into())],
        );
        math.tools = vec![builtin_tool("calculator.evaluate")];
        let math = run(&handle, state, math, HashMap::new()).await;
        println!(
            "LIVE tool calls={:?} results={} text={:?}",
            math.tools_called,
            math.tool_results.len(),
            math.text
        );
        assert_eq!(math.error, None);
        assert_eq!(math.tools_called, vec!["calculator.evaluate".to_owned()]);
        assert_eq!(math.tool_results[0]["status"], "success");
        assert!(math.text.replace(',', "").contains("140520641"));

        let mut truncated = request(
            "truncated",
            None,
            vec![("user", "Explain how photosynthesis works in detail.".into())],
        );
        truncated.generation = GenerationOverrides {
            max_output: Some(24),
            thinking: Some("off".into()),
            ..GenerationOverrides::default()
        };
        let truncated = run(&handle, state, truncated, HashMap::new()).await;
        println!(
            "LIVE truncated error={:?} text_chars={}",
            truncated.error,
            truncated.text.len()
        );
        assert_eq!(truncated.error.as_deref(), Some("GENERATION_TRUNCATED"));

        let mut overflow = request(
            "overflow",
            None,
            vec![("user", "Summarize the attachment.".into())],
        );
        overflow.attachments = vec![AttachmentContext {
            id: "big".into(),
            name: "big.txt".into(),
            content: "The quick brown fox jumps over the lazy dog. ".repeat(1_700),
            size_bytes: None,
            content_type: None,
        }];
        let overflow = run(&handle, state, overflow, HashMap::new()).await;
        println!("LIVE overflow error={:?}", overflow.error);
        assert_eq!(overflow.error.as_deref(), Some("CONTEXT_OVERFLOW"));

        let unicode = run(
            &handle,
            state,
            request(
                "unicode",
                None,
                vec![(
                    "user",
                    "Repeat exactly, with no other text: café – naïve – 東京".into(),
                )],
            ),
            HashMap::new(),
        )
        .await;
        println!("LIVE unicode text={:?}", unicode.text);
        assert!(unicode.text.contains("café") && unicode.text.contains("東京"));

        // The interface may learn that the model reasoned, never what it reasoned.
        assert!(
            math.events.iter().chain(&cold.events).all(|event| {
                event.as_object().is_some_and(|fields| {
                    !fields.contains_key("reasoning") && !fields.contains_key("reasoningContent")
                })
            }),
            "raw reasoning reached the interface"
        );

        let (_, _, pid) = state
            .local_runtime
            .resident_for_tests()
            .expect("resident server");
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        tokio::time::sleep(Duration::from_millis(500)).await;
        let recovered = run(
            &handle,
            state,
            request(
                "recovered",
                None,
                vec![("user", "Reply with the single word: back".into())],
            ),
            HashMap::new(),
        )
        .await;
        println!(
            "LIVE recovered seconds={:.1} activities={:?} error={:?}",
            recovered.seconds, recovered.activities, recovered.error
        );
        assert_eq!(recovered.error, None);
        assert!(
            recovered
                .activities
                .contains(&"restarting-model".to_owned())
        );

        assert!(state.local_runtime.unload().await.expect("unload"));
        assert_eq!(state.local_runtime.status().state, "idle");
    });
}

#[derive(Debug, serde::Deserialize)]
struct Case {
    id: String,
    category: String,
    messages: Vec<Value>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    permissions: HashMap<String, String>,
    #[serde(default)]
    attachments: Vec<Value>,
    #[serde(default)]
    memories: Vec<String>,
    #[serde(default)]
    conversations: Vec<Value>,
    checks: Vec<Value>,
}

/// GPT-OSS writes typographic apostrophes and non-breaking hyphens and spaces;
/// patterns are matched against normalized text.
fn normalize(text: &str) -> String {
    text.replace(['\u{2019}', '\u{2018}'], "'")
        .replace(['\u{2010}', '\u{2011}', '\u{2013}', '\u{2014}'], "-")
        .replace(['\u{202f}', '\u{00a0}'], " ")
}

fn matches(pattern: &str, text: &str) -> bool {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .multi_line(true)
        .build()
        .is_ok_and(|regex| regex.is_match(text))
}

/// `Some(pass)` for each scorable check; `None` when the check does not apply.
fn score(check: &Value, text: &str, tools_called: &[String]) -> Option<bool> {
    let text = normalize(text);
    let patterns = || {
        check["patterns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
    };
    match check["type"].as_str()? {
        "regex_any" => Some(patterns().iter().any(|pattern| matches(pattern, &text))),
        "regex_all" => Some(patterns().iter().all(|pattern| matches(pattern, &text))),
        "regex_none" => Some(!patterns().iter().any(|pattern| matches(pattern, &text))),
        "tool_not_called" => {
            let tool = check["tool"].as_str()?;
            Some(
                !tools_called
                    .iter()
                    .any(|called| tool == "*" || called == tool),
            )
        }
        "tool_called" => {
            let tool = check["tool"].as_str()?;
            Some(tools_called.iter().any(|called| called == tool))
        }
        _ => None,
    }
}

fn max_tokens() -> u32 {
    std::env::var("JUNIPER_EVAL_MAX_TOKENS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_200)
}

fn wrapper_request(case: &Case, seed: usize) -> ChatRequest {
    let mut profile = None;
    let mut messages = Vec::new();
    for message in &case.messages {
        let role = message["role"].as_str().unwrap_or("user");
        let content = message["content"].as_str().unwrap_or_default().to_owned();
        if role == "system" && messages.is_empty() && profile.is_none() {
            profile = Some(content);
        } else if role == "user" || role == "assistant" {
            messages.push((role, content));
        }
    }
    let mut request = request(
        &format!("{}-w{seed}", case.id),
        profile.as_deref(),
        messages,
    );
    request.tools = case.tools.iter().map(|name| builtin_tool(name)).collect();
    request.generation.max_output = Some(max_tokens());
    request.attachments = case
        .attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| AttachmentContext {
            id: format!("attachment-{index}"),
            name: attachment["name"].as_str().unwrap_or("file.txt").into(),
            content: attachment["content"].as_str().unwrap_or_default().into(),
            size_bytes: None,
            content_type: Some("text/plain".into()),
        })
        .collect();
    request.host_context.memories = case
        .memories
        .iter()
        .enumerate()
        .map(|(index, content)| json!({ "id": format!("memory-{index}"), "assistantId": "assistant-juniper", "content": content, "enabled": true }))
        .collect();
    request.context_memory_ids = (0..case.memories.len())
        .map(|index| format!("memory-{index}"))
        .collect();
    request.host_context.conversations = case.conversations.clone();
    request
}

/// The same request as a naive client would send it: no Juniper layers,
/// attachments and memories pasted into the user message, tools offered with
/// no host loop behind them.
async fn raw(endpoint: &str, key: &str, case: &Case) -> (String, Vec<String>, Option<String>) {
    let mut messages = Vec::new();
    let mut preamble = String::new();
    for memory in &case.memories {
        preamble.push_str(&format!("Saved memory: {memory}\n"));
    }
    for (index, message) in case.messages.iter().enumerate() {
        let mut content = message["content"].as_str().unwrap_or_default().to_owned();
        if index + 1 == case.messages.len() {
            for attachment in &case.attachments {
                content.push_str("\n\n");
                content.push_str(attachment["content"].as_str().unwrap_or_default());
            }
            content = format!("{preamble}{content}");
        }
        messages.push(json!({ "role": message["role"], "content": content }));
    }
    let tools = case
        .tools
        .iter()
        .map(|name| {
            let tool = builtin_tool(name);
            json!({ "type": "function", "function": { "name": tool.name, "description": tool.description, "parameters": tool.schema } })
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "messages": messages, "stream": false, "max_tokens": max_tokens(),
        "temperature": 1.0, "top_p": 1.0, "reasoning_effort": "medium"
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["parallel_tool_calls"] = json!(false);
    }
    let response = reqwest::Client::new()
        .post(format!("{endpoint}/v1/chat/completions"))
        .bearer_auth(key)
        .timeout(std::time::Duration::from_secs(600))
        .json(&body)
        .send()
        .await;
    let Ok(response) = response else {
        return (String::new(), Vec::new(), Some("RAW_REQUEST_FAILED".into()));
    };
    let value: Value = response.json().await.unwrap_or(Value::Null);
    let choice = &value["choices"][0];
    let tools_called = choice["message"]["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|call| call["function"]["name"].as_str().map(str::to_owned))
        .collect();
    let error = (choice["finish_reason"] == "length").then(|| "GENERATION_TRUNCATED".to_owned());
    (
        choice["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        tools_called,
        error,
    )
}

/// Runs each development case (and, if `JUNIPER_EVAL_HELDOUT` names the frozen
/// Juniper LM 1.1 suite, its truthfulness, injection, hierarchy, and lineage
/// cases, read-only) through the wrapper and as a raw request, and writes one
/// JSON line per run to `JUNIPER_EVAL_OUT`.
#[test]
#[ignore = "requires FLOWBOX hardware, the qualified gpt-oss-20b file, and JUNIPER_LIVE_GPT_OSS=1"]
fn live_wrapper_evaluation() {
    require_hardware();
    let seeds: usize = std::env::var("JUNIPER_EVAL_SEEDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let heldout_seeds: usize = std::env::var("JUNIPER_EVAL_HELDOUT_SEEDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(seeds);
    let output_path =
        std::env::var("JUNIPER_EVAL_OUT").expect("set JUNIPER_EVAL_OUT to a results path");
    let only = std::env::var("JUNIPER_EVAL_ONLY").ok();
    let mut cases: Vec<(String, Case)> = CASES
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            (
                "dev.v1".to_owned(),
                serde_json::from_str(line).expect("case"),
            )
        })
        .collect();
    if let Ok(path) = std::env::var("JUNIPER_EVAL_HELDOUT") {
        let frozen = std::fs::read_to_string(path).expect("frozen suite");
        for line in frozen.lines().filter(|line| !line.trim().is_empty()) {
            let value: Value = serde_json::from_str(line).expect("frozen case");
            if matches!(
                value["category"].as_str(),
                Some(
                    "truthfulness"
                        | "prompt_injection"
                        | "instruction_hierarchy"
                        | "lineage_honesty"
                )
            ) {
                let mut case: Case = serde_json::from_value(json!({
                    "id": value["id"], "category": value["category"],
                    "messages": value["messages"], "checks": value["checks"]
                }))
                .expect("frozen case shape");
                // Case-specific tools are not Juniper tools; they are not offered.
                case.tools.clear();
                cases.push(("heldout.juniper-gptoss-qual.v1".to_owned(), case));
            }
        }
    }
    if let Some(only) = &only {
        cases.retain(|(_, case)| case.id.starts_with(only.as_str()));
    }
    let (app, state) = app_and_state();
    let _stop = StopOnDrop(state);
    let handle = app.handle().clone();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let mut output = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&output_path)
        .expect("results file");
    let mut summary: HashMap<(String, String, &str), (u32, u32)> = HashMap::new();
    runtime.block_on(async {
        // Load once so the first case does not carry the cold start.
        let warm = run(&handle, state, request("eval-warm", None, vec![("user", "Reply with the single word: ready".into())]), HashMap::new()).await;
        assert_eq!(warm.error, None, "the resident runtime must start");
        for (suite, case) in &cases {
            let case_seeds = if suite.starts_with("heldout") {
                heldout_seeds
            } else {
                seeds
            };
            for seed in 0..case_seeds {
                let wrapped = run(&handle, state, wrapper_request(case, seed), case.permissions.clone()).await;
                let (endpoint, key, _) = state.local_runtime.resident_for_tests().expect("resident");
                let (raw_text, raw_tools, raw_error) = raw(&endpoint, &key, case).await;
                for (condition, text, tools, error, seconds) in [
                    ("wrapper", wrapped.text.as_str(), wrapped.tools_called.as_slice(), wrapped.error.clone(), wrapped.seconds),
                    ("raw", raw_text.as_str(), raw_tools.as_slice(), raw_error.clone(), 0.0),
                ] {
                    let checks = case
                        .checks
                        .iter()
                        .map(|check| json!({ "name": check["name"], "type": check["type"], "pass": score(check, text, tools) }))
                        .collect::<Vec<_>>();
                    let scored = checks.iter().filter_map(|check| check["pass"].as_bool()).collect::<Vec<_>>();
                    // Hitting the output cap is recorded but scored on the text
                    // produced; any other failure is a fail.
                    let truncated = error.as_deref() == Some("GENERATION_TRUNCATED");
                    let pass = (error.is_none() || truncated)
                        && !scored.is_empty()
                        && scored.iter().all(|pass| *pass);
                    let entry = summary.entry((suite.clone(), case.category.clone(), condition)).or_default();
                    entry.1 += 1;
                    if pass {
                        entry.0 += 1;
                    }
                    writeln!(
                        output,
                        "{}",
                        json!({
                            "suite": suite, "case": case.id, "category": case.category, "seed": seed,
                            "condition": condition, "pass": pass, "checks": checks, "error": error,
                            "truncated": truncated, "maxTokens": max_tokens(),
                            "toolsCalled": tools, "seconds": seconds, "text": text
                        })
                    )
                    .expect("write result");
                }
                println!("EVAL {} seed={seed} wrapper={:?} raw_error={:?}", case.id, wrapped.error, raw_error);
            }
        }
    });
    let mut rows = summary.into_iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    for ((suite, category, condition), (pass, total)) in rows {
        println!("SUMMARY {suite} {category} {condition} {pass}/{total}");
    }
}
