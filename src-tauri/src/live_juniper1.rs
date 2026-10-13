//! Live qualification of the Juniper 1 token-ID backend against the pinned
//! llama.cpp b11270 server and the real F16 GGUF exports, through the App's
//! request path (`local_runtime::stream_chat` -> `juniper1::chat`). Ignored by
//! default; writes a machine-readable evidence record.
//!
//! ```text
//! XDG_DATA_HOME=<scratch dir on the GGUFs' filesystem> JUNIPER_LIVE_JUNIPER1=1 \
//! JUNIPER_LLAMA_SERVER_JUNIPER1=<llama.cpp b11270 llama-server> \
//! JUNIPER1_BASE_GGUF=<juniper1-base-v1-f16.gguf> JUNIPER1_SFT_GGUF=<juniper1-sft-pilot-v1-f16.gguf> \
//! JUNIPER1_TOKENIZER=<tokenizer.json> JUNIPER1_WRONG_RUNTIME=<a llama-server of another build> \
//! JUNIPER1_PARITY_FIXTURE=<tests/juniper1/python_parity.py output> \
//! JUNIPER1_LIVE_EVIDENCE=<out.json> CUDA_VISIBLE_DEVICES= \
//! cargo test --lib -- --ignored --nocapture --test-threads=1 live_juniper1
//! ```
//!
//! Tool calls and analysis messages here are forced through the real server
//! one ID at a time (`client::forcing`) to drive the host paths: the model
//! under test was not trained on tool use, and a forced call says nothing
//! about its competence. What the model does after a forced prefix is its own.

use crate::commands::{AppState, Cancellation};
use crate::domain::ChatRequest;
use crate::juniper1::chat::FORCED_SCRIPTS;
use crate::juniper1::client::{CompletionSource, Sampling, TokenIdClient};
use crate::juniper1::profile;
use crate::juniper1::protocol::{self, Message, Role};
use crate::juniper1::tokenizer::{self as tk, J1Tokenizer};
use crate::local_runtime;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Listener;
use tokio::time::Duration;

const BASE: &str = "juniper1-base-v1-f16";
const SFT: &str = "juniper1-sft-pilot-v1-f16";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
}

#[derive(Default)]
struct Evidence {
    checks: Vec<Value>,
}

impl Evidence {
    fn record(&mut self, name: &str, model: Option<&str>, passed: bool, detail: Value) {
        println!("LIVE {name} model={model:?} passed={passed} {detail}");
        self.checks.push(json!({
            "check": name, "model": model, "passed": passed, "detail": detail
        }));
    }
}

#[derive(Debug, Default, Clone)]
struct Outcome {
    text: String,
    error: Option<String>,
    activities: Vec<String>,
    tool_results: Vec<Value>,
    permission_prompts: usize,
    provenance: Value,
    usage: Value,
    events: Vec<Value>,
    seconds: f64,
}

impl Outcome {
    fn summary(&self) -> Value {
        json!({
            "error": self.error,
            "answer": self.text,
            "activities": self.activities,
            "tool_results": self.tool_results,
            "usage": self.usage,
            "seconds": (self.seconds * 1000.0).round() / 1000.0,
        })
    }

    /// Nothing but an accepted answer reaches the interface: no deltas on
    /// failure, no control spellings, no analysis fields.
    fn clean(&self) -> bool {
        let deltas = self
            .events
            .iter()
            .filter(|e| e["delta"].is_string())
            .count();
        let spelled = tk::control_spellings()
            .iter()
            .any(|spelling| self.text.contains(spelling.as_str()));
        let reasoning = self.events.iter().any(|event| {
            event.as_object().is_some_and(|fields| {
                fields.contains_key("reasoning") || fields.contains_key("reasoningContent")
            })
        });
        let one_done = self.events.iter().filter(|e| e["done"] == true).count() == 1;
        one_done
            && !spelled
            && !reasoning
            && if self.error.is_some() {
                deltas == 0
            } else {
                deltas == 1
            }
    }
}

fn request(id: &str, model: &str, messages: Vec<(&str, String)>, max_output: u32) -> ChatRequest {
    let messages: Vec<Value> = messages
        .into_iter()
        .map(|(role, content)| json!({ "role": role, "content": content }))
        .collect();
    serde_json::from_value(json!({
        "requestId": id,
        "assistantId": "assistant-juniper",
        "assistantName": "Juniper",
        "conversationId": format!("conversation-{id}"),
        "provider": {
            "id": "juniper-local", "name": "Juniper on this device", "kind": "juniper-local",
            "baseUrl": "http://127.0.0.1", "locality": "local", "transportLocation": "on-device"
        },
        "model": {
            "id": format!("juniper-local:{model}"), "providerId": "juniper-local",
            "modelId": model, "displayName": model, "catalogId": model,
            "executionLocation": "on-device",
            "capabilities": { "tools": "supported", "thinking": "unknown", "generationParameters": ["maxOutput"] }
        },
        "messages": messages,
        "tools": [],
        "generation": { "maxOutput": max_output }
    }))
    .expect("live request")
}

fn calculator() -> crate::domain::ToolDefinition {
    crate::domain::ToolDefinition {
        name: "calculator.evaluate".into(),
        description: "Safely evaluate common arithmetic without executing code.".into(),
        risk: "automatic-safe".into(),
        enabled: true,
        schema: json!({ "type": "object", "properties": { "expression": { "type": "string" } }, "required": ["expression"] }),
    }
}

fn memory_save() -> crate::domain::ToolDefinition {
    crate::domain::ToolDefinition {
        name: "memory.save".into(),
        description: "Propose a user-curated memory for explicit approval.".into(),
        risk: "user-data-write".into(),
        enabled: true,
        schema: json!({ "type": "object", "properties": { "content": { "type": "string" } }, "required": ["content"] }),
    }
}

/// One request through the real host path. Permission prompts are answered
/// from `decisions`; anything unlisted is denied.
async fn run<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &'static AppState,
    request: ChatRequest,
    decisions: HashMap<String, String>,
    cancellation: Cancellation,
) -> Outcome {
    let topic = format!("juniper://chat/{}", request.request_id);
    let events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = events.clone();
    let listener = app.listen(topic, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            captured.lock().expect("events").push(value);
        }
    });
    let request_id = request.request_id.clone();
    let watched = events.clone();
    let responder = tokio::spawn(async move {
        let mut answered = std::collections::HashSet::new();
        loop {
            let prompts: Vec<Value> = watched
                .lock()
                .expect("events")
                .iter()
                .filter(|event| !event["permissionRequest"].is_null())
                .map(|event| event["permissionRequest"].clone())
                .collect();
            for prompt in prompts {
                let call = prompt["callId"].as_str().unwrap_or_default().to_owned();
                if !answered.insert(call.clone()) {
                    continue;
                }
                let decision = decisions
                    .get(prompt["toolName"].as_str().unwrap_or_default())
                    .cloned()
                    .unwrap_or_else(|| "deny".into());
                let key = format!("{request_id}:{call}");
                for _ in 0..400 {
                    if let Some(sender) = state
                        .permission_waiters
                        .lock()
                        .expect("waiters")
                        .remove(&key)
                    {
                        let _ = sender.send(decision.clone());
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let started = Instant::now();
    if let Err(error) =
        local_runtime::stream_chat(app.clone(), request.clone(), cancellation, state).await
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
        if let Some(results) = event["toolResults"].as_array() {
            outcome.tool_results.extend(results.iter().cloned());
        }
        if !event["permissionRequest"].is_null() {
            outcome.permission_prompts += 1;
        }
        if event["done"] == true {
            outcome.error = event["error"]["code"].as_str().map(str::to_owned);
            outcome.provenance = event["provenance"].clone();
            outcome.usage = event["usage"].clone();
        }
    }
    outcome.events = events;
    outcome
}

/// llama-server processes whose parent is this test process.
fn server_children() -> Vec<u32> {
    let me = std::process::id();
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            let (head, tail) = stat.rsplit_once(')')?;
            let comm = head.split_once('(')?.1;
            let parent: u32 = tail.split_whitespace().nth(1)?.parse().ok()?;
            (parent == me && comm.starts_with("llama-server"))
                .then(|| entry.file_name().to_string_lossy().parse().ok())
                .flatten()
        })
        .collect()
}

fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !stat
            .rsplit_once(')')
            .is_some_and(|(_, tail)| tail.trim_start().starts_with('Z'))
    })
}

fn link(source: &Path, target: &Path) {
    let _ = std::fs::remove_file(target);
    if std::fs::hard_link(source, target).is_err() {
        std::fs::copy(source, target).expect("copy model file");
    }
}

fn sha256_file(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).expect("open");
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).expect("read");
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    format!("{:x}", hasher.finalize())
}

fn script(tok: &J1Tokenizer, parts: &[Result<u32, &str>]) -> Vec<u32> {
    parts
        .iter()
        .flat_map(|part| match part {
            Ok(id) => vec![*id],
            Err(text) => tok.encode(text).expect("encode"),
        })
        .collect()
}

fn force(request_id: &str, ids: Vec<u32>) {
    FORCED_SCRIPTS
        .lock()
        .expect("scripts")
        .insert(request_id.to_owned(), ids);
}

fn set_env(name: &str, value: &str) {
    // The live test runs single-threaded (--test-threads=1); nothing else reads these.
    unsafe { std::env::set_var(name, value) }
}

struct StopOnDrop(&'static AppState);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.local_runtime.terminate_all();
    }
}

/// Logprob gap between the two most likely next IDs after `prefix`, read
/// from the raw distribution (diagnostic only; the App never asks for it).
async fn top_two(endpoint: &str, key: &str, prefix: &[u32]) -> Value {
    let body = json!({
        "prompt": prefix, "n_predict": 1, "stream": false, "return_tokens": true,
        "n_probs": 2, "samplers": ["top_k", "top_p", "temperature"], "temperature": 0.0,
        "top_k": 1, "top_p": 1.0, "min_p": 0.0, "cache_prompt": false
    });
    let response: Value = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
        .post(format!("{endpoint}/completion"))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .expect("send")
        .json()
        .await
        .expect("json");
    let top = &response["completion_probabilities"][0]["top_logprobs"];
    json!({
        "top": [top[0]["id"], top[1]["id"]],
        "logprob_gap": top[0]["logprob"].as_f64().zip(top[1]["logprob"].as_f64()).map(|(a, b)| a - b),
    })
}

#[allow(clippy::too_many_lines)]
async fn parity<R: tauri::Runtime>(
    evidence: &mut Evidence,
    state: &'static AppState,
    model: &str,
    sha: &str,
    _app: &tauri::AppHandle<R>,
) {
    let Ok(path) = std::env::var("JUNIPER1_PARITY_FIXTURE") else {
        evidence.record(
            "greedy_parity_vs_python_client",
            Some(model),
            false,
            json!({ "skipped": "JUNIPER1_PARITY_FIXTURE not set" }),
        );
        return;
    };
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(path).expect("fixture")).expect("fixture json");
    let Some(reference) = fixture["models"]
        .as_array()
        .and_then(|models| models.iter().find(|m| m["gguf_sha256"] == sha))
    else {
        evidence.record(
            "greedy_parity_vs_python_client",
            Some(model),
            false,
            json!({ "skipped": "no reference for this GGUF" }),
        );
        return;
    };
    let runtime = state
        .local_runtime
        .token_ids_for_tests()
        .expect("resident Juniper 1 runtime");
    let (endpoint, key, _) = state.local_runtime.resident_for_tests().expect("resident");
    let mut identical = 0;
    let mut divergences = Vec::new();
    let mut prompt_hash = Vec::new();
    let generations = reference["generations"].as_array().expect("generations");
    for generation in generations {
        let prompt: Vec<u32> =
            serde_json::from_value(generation["prompt_ids"].clone()).expect("prompt");
        let expected: Vec<u32> = serde_json::from_value(generation["ids"].clone()).expect("ids");
        prompt_hash.extend(prompt.iter().map(u32::to_string));
        let got = runtime
            .client
            .complete(
                &prompt,
                24,
                &Sampling::greedy(),
                &[],
                &Cancellation::default(),
            )
            .await
            .expect("complete")
            .ids;
        if got == expected {
            identical += 1;
        } else {
            let k = got
                .iter()
                .zip(&expected)
                .take_while(|(a, b)| a == b)
                .count();
            let mut prefix = prompt.clone();
            prefix.extend(&got[..k]);
            divergences.push(json!({
                "first_divergence": k, "rust": got.get(k), "python": expected.get(k),
                "next_token_distribution": top_two(&endpoint, &key, &prefix).await,
            }));
        }
    }
    let passed = identical == generations.len();
    evidence.record(
        "greedy_parity_vs_python_client",
        Some(model),
        passed,
        json!({
            "prompts": generations.len(), "identical_sequences": identical,
            "bundle_sha256": fixture["bundle_sha256"], "selection": fixture["selection"],
            "python_runtime_build": reference["runtime_build"],
            "python_paths_called": reference["paths_called"],
            "prompt_ids_sha256": tk::sha256_hex(prompt_hash.join(",").as_bytes()),
            "divergences": divergences,
            "note": "Rust App client and Python reference client on separate llama-server b11270 processes, same GGUF, greedy, 24 tokens; a divergence is reported with the raw top-2 logprob gap to separate F16 near-ties from errors",
        }),
    );
}

#[test]
#[ignore = "requires FLOWBOX, the Juniper 1 GGUFs and tokenizer, llama.cpp b11270, and JUNIPER_LIVE_JUNIPER1=1"]
#[allow(clippy::too_many_lines)]
fn live_juniper1_token_id_backend() {
    assert_eq!(std::env::var("JUNIPER_LIVE_JUNIPER1").as_deref(), Ok("1"));
    let base_gguf = PathBuf::from(env("JUNIPER1_BASE_GGUF"));
    let sft_gguf = PathBuf::from(env("JUNIPER1_SFT_GGUF"));
    let tokenizer_source = PathBuf::from(env("JUNIPER1_TOKENIZER"));
    let server = env("JUNIPER_LLAMA_SERVER_JUNIPER1");
    let out = PathBuf::from(env("JUNIPER1_LIVE_EVIDENCE"));
    let app = tauri::test::mock_app();
    let state: &'static AppState = Box::leak(Box::default());
    let _stop = StopOnDrop(state);
    let handle = app.handle().clone();
    let profile = profile::profile().expect("profile");
    let models = crate::managed_models::models_directory(&handle).expect("models directory");
    let tokenizer_path = models.join(&profile.tokenizer.file);
    let base_path = models.join(format!("{BASE}.gguf"));
    let sft_path = models.join(format!("{SFT}.gguf"));
    link(&base_gguf, &base_path);
    link(&sft_gguf, &sft_path);
    std::fs::copy(&tokenizer_source, &tokenizer_path).expect("tokenizer");
    let tok = J1Tokenizer::load(
        &std::fs::read(&tokenizer_path).expect("tokenizer"),
        &profile.tokenizer.sha256,
        profile.tokenizer.vocab_size,
    )
    .expect("canonical tokenizer");
    let mut evidence = Evidence::default();
    let version = std::process::Command::new(&server)
        .arg("--version")
        .env("CUDA_VISIBLE_DEVICES", "")
        .output()
        .expect("version");
    let version_text = format!(
        "{}{}",
        String::from_utf8_lossy(&version.stdout),
        String::from_utf8_lossy(&version.stderr)
    );
    let identity = json!({
        "runtime_version": version_text.lines().filter(|l| l.starts_with("version") || l.starts_with("built")).collect::<Vec<_>>(),
        "models": {
            BASE: { "sha256": sha256_file(&base_path), "size_bytes": std::fs::metadata(&base_path).unwrap().len() },
            SFT: { "sha256": sha256_file(&sft_path), "size_bytes": std::fs::metadata(&sft_path).unwrap().len() },
        },
        "tokenizer_sha256": tok.sha256(),
    });
    println!("LIVE identity {identity}");
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let none = HashMap::new;
        let go = |id: &str, model: &str, messages: Vec<(&str, String)>, max: u32| request(id, model, messages, max);

        // --- the text paths still refuse both real exports
        for (model, path) in [(BASE, &base_path), (SFT, &sft_path)] {
            let mut file = std::fs::File::open(path).expect("model");
            let refused = crate::gguf_guard::refuse_juniper_on_text_path(&mut file);
            evidence.record("text_path_guard_refuses_real_gguf", Some(model), refused.is_err(), json!({ "error": refused.err() }));
        }

        // --- startup failures leave nothing running
        let wrong = std::env::var("JUNIPER1_WRONG_RUNTIME").ok();
        if let Some(wrong) = &wrong {
            set_env("JUNIPER_LLAMA_SERVER_JUNIPER1", wrong);
            let outcome = run(&handle, state, go("wrong-runtime", BASE, vec![("user", "Hi".into())], 16), none(), Cancellation::default()).await;
            set_env("JUNIPER_LLAMA_SERVER_JUNIPER1", &server);
            evidence.record("startup_refuses_wrong_runtime_build", Some(BASE),
                outcome.error.as_deref() == Some("RUNTIME_IDENTITY_MISMATCH") && server_children().is_empty() && outcome.clean(),
                json!({ "runtime": wrong, "outcome": outcome.summary() }));
        } else {
            evidence.record("startup_refuses_wrong_runtime_build", Some(BASE), false, json!({ "skipped": "JUNIPER1_WRONG_RUNTIME not set" }));
        }
        std::fs::rename(&tokenizer_path, tokenizer_path.with_extension("hidden")).expect("hide tokenizer");
        let outcome = run(&handle, state, go("no-tokenizer", BASE, vec![("user", "Hi".into())], 16), none(), Cancellation::default()).await;
        std::fs::rename(tokenizer_path.with_extension("hidden"), &tokenizer_path).expect("restore tokenizer");
        evidence.record("startup_refuses_missing_tokenizer", Some(BASE),
            outcome.error.as_deref() == Some("JUNIPER1_TOKENIZER_UNAVAILABLE") && server_children().is_empty(), outcome.summary());
        let mut tampered = std::fs::read(&tokenizer_path).expect("tokenizer");
        let original = tampered.clone();
        tampered.push(b'\n');
        std::fs::write(&tokenizer_path, &tampered).expect("tamper");
        let outcome = run(&handle, state, go("bad-tokenizer", BASE, vec![("user", "Hi".into())], 16), none(), Cancellation::default()).await;
        std::fs::write(&tokenizer_path, &original).expect("restore");
        evidence.record("startup_refuses_tampered_tokenizer", Some(BASE),
            outcome.error.as_deref() == Some("JUNIPER1_TOKENIZER_MISMATCH") && server_children().is_empty(), outcome.summary());
        let mut bytes = std::fs::read(&base_path).expect("model");
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xff;
        std::fs::remove_file(&base_path).expect("unlink");
        std::fs::write(&base_path, &bytes).expect("tampered model");
        drop(bytes);
        let outcome = run(&handle, state, go("bad-model", BASE, vec![("user", "Hi".into())], 16), none(), Cancellation::default()).await;
        link(&base_gguf, &base_path);
        evidence.record("startup_refuses_model_hash_mismatch", Some(BASE),
            outcome.error.as_deref() == Some("LOCAL_MODEL_NOT_READY") && server_children().is_empty(), outcome.summary());
        let fake = models.join("fake-llama-server.sh");
        std::fs::write(&fake, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'version: 0.5.0-dev (build 11270, commit 748d4225b)'; exit 0; fi\nexit 3\n").expect("fake");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        set_env("JUNIPER_LLAMA_SERVER_JUNIPER1", &fake.to_string_lossy());
        let outcome = run(&handle, state, go("crashing-runtime", BASE, vec![("user", "Hi".into())], 16), none(), Cancellation::default()).await;
        set_env("JUNIPER_LLAMA_SERVER_JUNIPER1", &server);
        evidence.record("startup_failure_when_the_runtime_exits", Some(BASE),
            outcome.error.as_deref() == Some("LOCAL_RUNTIME_FAILED") && server_children().is_empty(),
            json!({ "runtime": "script that reports b11270 and exits 3 (fixture, not llama.cpp)", "outcome": outcome.summary() }));

        // --- base model: cold start, provenance, self-test, protocol behavior
        let cold = run(&handle, state, go("base-cold", BASE, vec![("user", "What is the capital of France?".into())], 32), none(), Cancellation::default()).await;
        let provenance = &cold.provenance;
        let base_ok = cold.activities.contains(&"loading-model".to_owned())
            && server_children().len() == 1
            && cold.clean()
            && matches!(cold.error.as_deref(), None | Some("MODEL_OUTPUT_INVALID" | "GENERATION_TRUNCATED" | "EMPTY_ANSWER"));
        let provenance_ok = cold.error.is_some()
            || (provenance["backend"] == "juniper1-token-ids.v1"
                && provenance["runtime"]["runtimeBuild"] == "b11270-748d4225b"
                && provenance["runtime"]["artifactSha256"] == profile.artifacts[0].sha256
                && provenance["runtime"]["tokenizerSha256"] == profile.tokenizer.sha256);
        evidence.record("cold_start_and_turn_through_app_path", Some(BASE), base_ok && provenance_ok,
            json!({ "outcome": cold.summary(), "provenance": provenance, "note": "the base model is not instruction-tuned; a rejected (unshown) turn is the expected safe outcome" }));
        let resident = state.local_runtime.token_ids_for_tests().expect("resident runtime");
        let self_test = resident.client.self_test(&tok, &Cancellation::default()).await;
        evidence.record("runtime_self_test", Some(BASE), self_test.is_ok(),
            json!({ "report": self_test.as_ref().map(|r| r.iter().map(|(id, v)| format!("{id}={v}")).collect::<Vec<_>>()).map_err(Clone::clone) }));

        let cancellation = Cancellation::default();
        let canceller = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            canceller.cancel();
        });
        let cancelled = run(&handle, state, go("base-cancel", BASE, vec![("user", "Write a very long story about a lighthouse keeper.".into())], 512), none(), cancellation).await;
        let after = run(&handle, state, go("base-after-cancel", BASE, vec![("user", "Hello.".into())], 8), none(), Cancellation::default()).await;
        evidence.record("cancellation_mid_generation", Some(BASE),
            cancelled.error.as_deref() == Some("REQUEST_CANCELLED") && cancelled.clean() && cancelled.seconds < 3.0
                && !after.activities.iter().any(|a| a.contains("model")) && state.local_runtime.status().state == "ready",
            json!({ "cancelled": cancelled.summary(), "next_request": after.summary() }));

        let (endpoint, key, pid) = state.local_runtime.resident_for_tests().expect("resident");
        let port: u16 = endpoint.rsplit(':').next().unwrap().parse().unwrap();
        let impatient = TokenIdClient::new(&endpoint, profile.tokenizer.vocab_size, Some(key.clone()), Duration::from_millis(50), Some((pid, port))).expect("client");
        let mut prompt = protocol::serialize(&tok, &[Message::new(Role::User, "Tell me a long story.", None, 1).unwrap()], true).unwrap();
        let timeout = impatient.complete(&prompt, 400, &Sampling::greedy(), &[], &Cancellation::default()).await;
        let healthy_after = resident.client.complete(&prompt, 4, &Sampling::greedy(), &[], &Cancellation::default()).await;
        evidence.record("request_timeout", Some(BASE),
            timeout.as_ref().err().is_some_and(|e| e.starts_with("LOCAL_RUNTIME_TIMEOUT")) && healthy_after.is_ok(),
            json!({ "error": timeout.err(), "next_request_ok": healthy_after.is_ok(), "client": "App TokenIdClient with a 50 ms timeout against the resident server" }));

        let mut banned = 0;
        let mut sampled = 0;
        for seed in 0..24 {
            let completion = resident.client.complete(&prompt, 48, &Sampling { temperature: 1.5, top_k: 0, top_p: 1.0, seed }, &[], &Cancellation::default()).await.expect("sample");
            sampled += completion.ids.len();
            banned += completion.ids.iter().filter(|id| protocol::forbidden_in_generation().contains(id)).count();
        }
        evidence.record("banned_ids_never_sampled", Some(BASE), banned == 0 && sampled > 0,
            json!({ "seeds": 24, "tokens": sampled, "banned_sampled": banned, "sampling": "temperature 1.5, top_k 0, top_p 1" }));

        let smile = [64 + 0xF0, 64 + 0x9F, 64 + 0x99, 64 + 0x82];
        let mut got = Vec::new();
        prompt = script(&tok, &[Ok(tk::USER), Err("smile"), Ok(tk::END), Ok(tk::ASSISTANT), Ok(tk::FINAL)]);
        for id in smile {
            let mut sequence = prompt.clone();
            sequence.extend(&got);
            got.extend(resident.client.complete(&sequence, 1, &Sampling::greedy(), &[(id, 100.0)], &Cancellation::default()).await.expect("byte").ids);
        }
        evidence.record("byte_fallback_ids_returned_intact", Some(BASE), got == smile,
            json!({ "forced": smile, "returned": got, "decoded": tok.decode(&got).unwrap_or_default() }));

        let mut cases = Vec::new();
        for question in ["Count to five.", "Write the words in reverse order: red blue green yellow", "What is 12 + 7?"] {
            let prompt = protocol::serialize(&tok, &[Message::new(Role::User, question, None, 1).unwrap()], true).unwrap();
            let first = resident.client.complete(&prompt, 24, &Sampling::greedy(), &[], &Cancellation::default()).await.expect("first").ids;
            let k = ((first.len().max(2) - 1) / 2).max(1);
            let mut continued_prompt = prompt.clone();
            continued_prompt.extend(&first[..k]);
            let continued = resident.client.complete(&continued_prompt, (first.len() - k) as u32, &Sampling::greedy(), &[], &Cancellation::default()).await.expect("continued");
            cases.push(json!({ "generated": first.len(), "split_at": k, "cached_equals_full": continued.ids == first[k..], "tokens_cached": continued.tokens_cached }));
        }
        evidence.record("kv_cache_continuation", Some(BASE), cases.iter().all(|c| c["cached_equals_full"] == true), json!({ "cases": cases }));
        parity(&mut evidence, state, BASE, &profile.artifacts[0].sha256, &handle).await;
        evidence.record("only_completion_endpoint_called", Some(BASE), resident.client.paths_called().iter().all(|p| p == "/completion"),
            json!({ "paths": resident.client.paths_called().into_iter().collect::<std::collections::BTreeSet<_>>() }));

        // --- stale-callback watch on a finished request's topic
        let stale = Arc::new(Mutex::new(0usize));
        let counter = stale.clone();
        let stale_listener = handle.listen("juniper://chat/base-cold", move |_| *counter.lock().unwrap() += 1);

        // --- switch to the SFT candidate
        let old_pid = pid;
        let first = run(&handle, state, go("sft-1", SFT, vec![("user", "What is the capital of France?".into())], 64), none(), Cancellation::default()).await;
        let switched = first.activities.contains(&"loading-model".to_owned()) && !alive(old_pid) && server_children().len() == 1
            && state.local_runtime.status().artifact_id.as_deref() == Some(SFT);
        evidence.record("model_switch_base_to_candidate", Some(SFT), switched,
            json!({ "previous_pid_alive": alive(old_pid), "servers": server_children().len(), "status": state.local_runtime.status().artifact_id }));
        let provenance = &first.provenance;
        let provenance_ok = provenance["backend"] == "juniper1-token-ids.v1"
            && provenance["constitution"] == "juniper-constitution.v2"
            && provenance["toolProtocol"] == "juniper-tool-protocol-v1"
            && provenance["runtime"]["runtimeBuild"] == "b11270-748d4225b"
            && provenance["runtime"]["artifactId"] == SFT
            && provenance["runtime"]["artifactSha256"] == profile.artifacts[1].sha256
            && provenance["runtime"]["modelRevision"] == profile.artifacts[1].source_checkpoint
            && provenance["runtime"]["tokenizerSha256"] == profile.tokenizer.sha256
            && provenance["runtime"]["protocol"] == "juniper-chat-v1"
            && provenance["runtime"]["contextSize"] == 2048
            && provenance["runtime"]["templateSha256"].is_null();
        evidence.record("single_turn_answer_with_provenance", Some(SFT), first.clean() && first.error.is_none() && provenance_ok,
            json!({ "outcome": first.summary(), "provenance": provenance }));
        let repeat = run(&handle, state, go("sft-1-repeat", SFT, vec![("user", "What is the capital of France?".into())], 64), none(), Cancellation::default()).await;
        evidence.record("repeated_identical_request_is_deterministic", Some(SFT),
            repeat.text == first.text && repeat.error == first.error && repeat.clean(),
            json!({ "first": first.text, "repeat": repeat.text, "first_seconds": first.seconds, "repeat_seconds": repeat.seconds }));

        let mut history: Vec<(&str, String)> = Vec::new();
        let mut turns = Vec::new();
        for (index, question) in ["My name is Ada. What is 2 + 3?", "What did I say my name was?", "Thank you. Name one primary color."].iter().enumerate() {
            history.push(("user", (*question).to_owned()));
            let outcome = run(&handle, state, go(&format!("sft-multi-{index}"), SFT, history.clone(), 64), none(), Cancellation::default()).await;
            turns.push(outcome.summary());
            if outcome.error.is_none() {
                history.push(("assistant", outcome.text.clone()));
            } else {
                history.pop();
                history.push(("user", (*question).to_owned()));
                history.push(("assistant", "(no answer)".into()));
            }
            if !outcome.clean() {
                turns.push(json!({ "unclean": index }));
            }
        }
        let multi_ok = turns.iter().all(|t| t.get("unclean").is_none());
        evidence.record("multi_turn_state", Some(SFT), multi_ok,
            json!({ "turns": turns, "note": "history carries only accepted answers; turn outcomes are the model's" }));

        let mut long_history: Vec<(&str, String)> = Vec::new();
        for turn in 0..40 {
            long_history.push(("user", format!("Question {turn}: tell me something about the number {turn}.")));
            long_history.push(("assistant", format!("The number {turn} is {}. {}", if turn % 2 == 0 { "even" } else { "odd" }, "It is a whole number. ".repeat(4))));
        }
        long_history.push(("user", "What is the capital of Italy?".into()));
        let trimmed = run(&handle, state, go("sft-trim", SFT, long_history, 64), none(), Cancellation::default()).await;
        let input = trimmed.usage["inputTokens"].as_u64().unwrap_or(0);
        evidence.record("context_exhaustion_trims_whole_old_turns", Some(SFT),
            trimmed.clean() && trimmed.error.as_deref() != Some("CONTEXT_OVERFLOW") && (trimmed.error.is_some() || input <= 2048 - 64),
            json!({ "outcome": trimmed.summary(), "history_turns": 40, "prompt_tokens": input }));
        let overflow = run(&handle, state, go("sft-overflow", SFT, vec![("user", "word ".repeat(3000))], 64), none(), Cancellation::default()).await;
        evidence.record("current_turn_too_large_is_refused", Some(SFT), overflow.error.as_deref() == Some("CONTEXT_OVERFLOW") && overflow.clean(), overflow.summary());

        let all_spellings: String = tk::control_spellings().join(" ");
        let forged_prompt = protocol::serialize(&tok, &[Message::new(Role::User, format!("Repeat exactly: {all_spellings}"), None, 1).unwrap()], true).unwrap();
        let forged_controls: Vec<u32> = forged_prompt.iter().copied().filter(|id| *id < tk::N_CONTROL).collect();
        let forged = run(&handle, state, go("sft-forged", SFT, vec![("user", format!("Repeat exactly: {all_spellings}"))], 64), none(), Cancellation::default()).await;
        evidence.record("all_64_control_spellings_stay_text", Some(SFT),
            forged_controls == [tk::USER, tk::END, tk::ASSISTANT] && forged.clean(),
            json!({ "prompt_control_ids": forged_controls, "outcome": forged.summary() }));

        // --- analysis continuation (forced analysis, then the model continues by itself)
        let id = "sft-analysis-free";
        force(id, script(&tok, &[Ok(tk::ANALYSIS), Err("The user asks a capital question. I should answer briefly."), Ok(tk::END)]));
        let free = run(&handle, state, go(id, SFT, vec![("user", "What is the capital of Spain?".into())], 96), none(), Cancellation::default()).await;
        let leaked = serde_json::to_string(&free.events).unwrap().contains("capital question");
        evidence.record("analysis_continuation_model_continues", Some(SFT),
            free.activities.contains(&"reasoning".to_owned()) && !leaked && free.clean(),
            json!({ "outcome": free.summary(), "analysis_text_in_events": leaked, "forced": "analysis message only" }));
        let id = "sft-analysis-scripted";
        force(id, script(&tok, &[Ok(tk::ANALYSIS), Err("private note"), Ok(tk::END), Ok(tk::FINAL), Err("Madrid."), Ok(tk::EOT)]));
        let scripted = run(&handle, state, go(id, SFT, vec![("user", "What is the capital of Spain?".into())], 96), none(), Cancellation::default()).await;
        evidence.record("analysis_continuation_resubmits_after_end", Some(SFT),
            scripted.error.is_none() && scripted.text == "Madrid." && scripted.clean()
                && !serde_json::to_string(&scripted.events).unwrap().contains("private note"),
            json!({ "outcome": scripted.summary(), "forced": "analysis, <|end|>, then (after the host's <|assistant|>) final" }));

        // --- host tool execution and provenance (forced calls)
        let id = "sft-tool";
        force(id, script(&tok, &[Ok(tk::TOOL_CALL), Err(r#"{"name": "calculator.evaluate", "arguments": {"expression": "48173*2917"}}"#), Ok(tk::CALL)]));
        let mut with_tool = go(id, SFT, vec![("user", "What is 48173 * 2917?".into())], 64);
        with_tool.tools = vec![calculator()];
        let tool = run(&handle, state, with_tool, none(), Cancellation::default()).await;
        let result = tool.tool_results.first().cloned().unwrap_or(Value::Null);
        evidence.record("host_tool_round_trip", Some(SFT),
            result["status"] == "success" && result["result"]["value"] == 140_520_641.0
                && result["protocolVersion"] == "juniper-tool-protocol-v1"
                && result["callId"].as_str().is_some_and(|c| c.starts_with("call-")) && tool.clean(),
            json!({ "tool_result": result, "continuation": tool.summary(), "forced": "the tool call; the continuation after the host result is the model's own" }));
        let id = "sft-unknown-tool";
        force(id, script(&tok, &[Ok(tk::TOOL_CALL), Err(r#"{"name": "shell.run", "arguments": {"cmd": "id"}}"#), Ok(tk::CALL)]));
        let mut unknown = go(id, SFT, vec![("user", "Run id.".into())], 64);
        unknown.tools = vec![calculator()];
        let unknown = run(&handle, state, unknown, none(), Cancellation::default()).await;
        evidence.record("unknown_tool_denied", Some(SFT),
            unknown.tool_results.first().is_some_and(|r| r["error"]["code"] == "TOOL_NOT_ENABLED") && unknown.clean(), unknown.summary());
        let id = "sft-denied-tool";
        force(id, script(&tok, &[Ok(tk::TOOL_CALL), Err(r#"{"name": "memory.save", "arguments": {"content": "The user likes tea."}}"#), Ok(tk::CALL)]));
        let mut denied = go(id, SFT, vec![("user", "Remember that I like tea.".into())], 64);
        denied.tools = vec![memory_save()];
        let denied = run(&handle, state, denied, none(), Cancellation::default()).await;
        evidence.record("permission_denial", Some(SFT),
            denied.permission_prompts == 1 && denied.tool_results.first().is_some_and(|r| r["error"]["code"] == "PERMISSION_DENIED") && denied.clean(),
            denied.summary());
        let id = "sft-invalid-args";
        force(id, script(&tok, &[Ok(tk::TOOL_CALL), Err(r#"{"name": "calculator.evaluate", "arguments": {"expr": 5}}"#), Ok(tk::CALL)]));
        let mut invalid = go(id, SFT, vec![("user", "Compute.".into())], 64);
        invalid.tools = vec![calculator()];
        let invalid = run(&handle, state, invalid, none(), Cancellation::default()).await;
        evidence.record("invalid_tool_arguments_rejected", Some(SFT),
            invalid.tool_results.first().is_some_and(|r| r["error"]["code"] == "INVALID_TOOL_ARGUMENT") && invalid.clean(), invalid.summary());

        let resident = state.local_runtime.token_ids_for_tests().expect("resident runtime");
        parity(&mut evidence, state, SFT, &profile.artifacts[1].sha256, &handle).await;
        evidence.record("only_completion_endpoint_called", Some(SFT), resident.client.paths_called().iter().all(|p| p == "/completion"),
            json!({ "paths": resident.client.paths_called().into_iter().collect::<std::collections::BTreeSet<_>>() }));

        // --- crash and restart
        let (_, _, pid) = state.local_runtime.resident_for_tests().expect("resident");
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        tokio::time::sleep(Duration::from_millis(500)).await;
        let recovered = run(&handle, state, go("sft-recovered", SFT, vec![("user", "Say hello.".into())], 32), none(), Cancellation::default()).await;
        evidence.record("crash_and_restart", Some(SFT),
            recovered.activities.contains(&"restarting-model".to_owned()) && recovered.clean() && !alive(pid) && server_children().len() == 1,
            json!({ "outcome": recovered.summary(), "killed_pid_alive": alive(pid) }));

        // --- switch back; no stale events on earlier topics; one server
        let (_, _, sft_pid) = state.local_runtime.resident_for_tests().expect("resident");
        let back = run(&handle, state, go("base-again", BASE, vec![("user", "Hello.".into())], 8), none(), Cancellation::default()).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        handle.unlisten(stale_listener);
        let stale_events = *stale.lock().unwrap();
        evidence.record("switch_back_without_stale_callbacks_or_leaks", Some(BASE),
            back.activities.contains(&"loading-model".to_owned()) && !alive(sft_pid) && server_children().len() == 1 && stale_events == 0 && back.clean(),
            json!({ "events_on_finished_topic": stale_events, "servers": server_children().len(), "previous_pid_alive": alive(sft_pid) }));

        let unloaded = state.local_runtime.unload().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        evidence.record("unload_leaves_no_process", None, unloaded.is_ok() && server_children().is_empty() && state.local_runtime.status().state == "idle",
            json!({ "servers": server_children().len() }));
    });
    let passed = evidence
        .checks
        .iter()
        .filter(|c| c["passed"] == true)
        .count();
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| {
            info.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_owned())
        });
    let record = json!({
        "schema": "juniper-app.juniper1-token-id-live.v1",
        "app_revision": std::env::var("JUNIPER1_EVIDENCE_REVISION").ok(),
        "host": { "cpu": cpu, "threads": std::thread::available_parallelism().map(|n| n.get()).ok(), "accelerator": "none (CUDA_VISIBLE_DEVICES cleared, -ngl 0)" },
        "identity": identity,
        "profile": { "id": profile.id, "protocol": profile.protocol, "runtime": profile.runtime, "ctx_size": profile.server.ctx_size },
        "summary": { "checks": evidence.checks.len(), "passed": passed },
        "checks": evidence.checks,
    });
    std::fs::write(&out, serde_json::to_string_pretty(&record).expect("json")).expect("evidence");
    println!(
        "LIVE summary {passed}/{} -> {}",
        record["summary"]["checks"],
        out.display()
    );
    assert_eq!(
        Some(passed as u64),
        record["summary"]["checks"].as_u64(),
        "failed checks are listed in the evidence record"
    );
}
