//! Live catalog verification (ignored; needs the network, a llama-server
//! build, and disk space). For each selected artifact it downloads through
//! Juniper's own transfer code from the real source, verifies every file's
//! SHA-256, starts llama-server with Juniper's generic arguments, runs one
//! chat request and one tool-call request, records the evidence, and
//! deletes the files.
//!
//!   JUNIPER_LLAMA_SERVER=/usr/lib/Juniper/runtime/llama-server \
//!   JUNIPER_LIVE_MODELS_DIR=/path/with/space \
//!   JUNIPER_LIVE_EVIDENCE=/path/evidence.jsonl \
//!   [JUNIPER_LIVE_ARTIFACTS=id,id] [JUNIPER_LIVE_RESUME=artifact-id] \
//!   cargo test --lib live_catalog -- --ignored --nocapture

use crate::catalog;
use crate::commands::Cancellation;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set")))
}

fn append(evidence: &Path, record: &Value) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(evidence)
        .expect("evidence file");
    writeln!(file, "{record}").expect("evidence write");
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn wait_healthy(client: &reqwest::Client, base: &str, limit: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if client
            .get(format!("{base}/health"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn chat(client: &reqwest::Client, base: &str, body: Value) -> Result<Value, String> {
    let response = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let value: Value = response.json().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {value}"));
    }
    Ok(value)
}

#[test]
#[ignore = "downloads real models; run deliberately with the documented environment"]
fn live_catalog() {
    let server = env_path("JUNIPER_LLAMA_SERVER");
    let models = env_path("JUNIPER_LIVE_MODELS_DIR");
    let evidence = env_path("JUNIPER_LIVE_EVIDENCE");
    std::fs::create_dir_all(&models).expect("models directory");
    let selected: Option<Vec<String>> = std::env::var("JUNIPER_LIVE_ARTIFACTS")
        .ok()
        .map(|value| value.split(',').map(str::to_owned).collect());
    let resume_target = std::env::var("JUNIPER_LIVE_RESUME").ok();
    let build = std::process::Command::new(&server)
        .arg("--version")
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stderr)
                .trim()
                .replace('\n', " | ")
        })
        .unwrap_or_default();
    let app = tauri::test::mock_app();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(900))
        .build()
        .unwrap();
    for entry in catalog::entries().expect("catalog") {
        for artifact in &entry.artifacts {
            if crate::managed_models::is_import_only(artifact)
                || selected
                    .as_ref()
                    .is_some_and(|ids| !ids.contains(&artifact.id))
            {
                continue;
            }
            let mut record = json!({
                "artifactId": artifact.id,
                "catalogId": entry.id,
                "sourceRepository": entry.source_repository,
                "sourceRevision": artifact.source_revision,
                "files": artifact.files.iter().map(|file| json!({"path": file.path, "sizeBytes": file.size_bytes, "sha256": file.sha256})).collect::<Vec<_>>(),
                "sizeBytes": artifact.size_bytes,
                "runtime": build,
                "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
                "startedAt": chrono::Utc::now().to_rfc3339(),
            });
            let started = Instant::now();
            // Optionally prove a real resume: stop part-way, then continue.
            if resume_target.as_deref() == Some(artifact.id.as_str()) {
                let cancellation = Cancellation::default();
                let cancel = cancellation.clone();
                let handle = app.handle().clone();
                let directory = models.clone();
                let target = artifact.clone();
                let interrupted = runtime.block_on(async move {
                    let stopper = tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(20)).await;
                        cancel.cancel();
                    });
                    let result = crate::managed_models::download_artifact(
                        &handle,
                        &directory,
                        &target,
                        "live",
                        cancellation,
                    )
                    .await;
                    stopper.abort();
                    result
                });
                let partial_bytes: u64 = std::fs::read_dir(&models)
                    .map(|entries| {
                        entries
                            .flatten()
                            .filter(|item| {
                                item.file_name().to_string_lossy().contains(&artifact.id)
                                    && item.file_name().to_string_lossy().ends_with(".part")
                            })
                            .filter_map(|item| item.metadata().ok())
                            .map(|metadata| metadata.len())
                            .sum()
                    })
                    .unwrap_or(0);
                record["resume"] = json!({ "interruptedWith": interrupted.err(), "partialBytesBeforeResume": partial_bytes });
            }
            let download = runtime.block_on(crate::managed_models::download_artifact(
                app.handle(),
                &models,
                artifact,
                "live",
                Cancellation::default(),
            ));
            record["downloadSeconds"] = json!(started.elapsed().as_secs());
            if let Err(error) = download {
                record["download"] = json!({ "ok": false, "error": error });
                append(&evidence, &record);
                let _ = remove_artifact(&models, &artifact.id);
                continue;
            }
            record["download"] = json!({ "ok": true, "verified": "every file's size and SHA-256 matched before publish" });
            let primary = crate::managed_models::primary_file(artifact)
                .expect("primary")
                .path
                .clone();
            let model_path = if artifact.files.len() == 1 {
                models.join(format!("{}.gguf", artifact.id))
            } else {
                models.join(&artifact.id).join(&primary)
            };
            let port = free_port();
            let context = entry.context_length.clamp(2_048, 16_384);
            let load_started = Instant::now();
            let mut child = std::process::Command::new(&server)
                .args([
                    "--host",
                    "127.0.0.1",
                    "--port",
                    &port.to_string(),
                    "--model",
                    &model_path.to_string_lossy(),
                    "--no-webui",
                    "--offline",
                    "--cache-ram",
                    "1024",
                    "--ctx-size",
                    &context.to_string(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("llama-server starts");
            let base = format!("http://127.0.0.1:{port}");
            let healthy = runtime.block_on(wait_healthy(&client, &base, Duration::from_secs(600)));
            record["loadSeconds"] = json!(load_started.elapsed().as_secs());
            record["contextSize"] = json!(context);
            if !healthy {
                record["load"] = json!({ "ok": false });
            } else {
                record["load"] = json!({ "ok": true });
                let chat_started = Instant::now();
                let answer = runtime.block_on(chat(
                    &client,
                    &base,
                    json!({
                        "messages": [{ "role": "user", "content": "In one short sentence, what is the capital of France?" }],
                        "max_tokens": 512,
                        "temperature": 0
                    }),
                ));
                record["chat"] = match answer {
                    Ok(value) => {
                        let content = value["choices"][0]["message"]["content"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned();
                        json!({
                            "ok": content.to_lowercase().contains("paris"),
                            "finishReason": value["choices"][0]["finish_reason"],
                            "content": content.chars().take(240).collect::<String>(),
                            "completionTokens": value["usage"]["completion_tokens"],
                            "seconds": chat_started.elapsed().as_secs_f32(),
                        })
                    }
                    Err(error) => json!({ "ok": false, "error": error }),
                };
                let tool = runtime.block_on(chat(
                    &client,
                    &base,
                    json!({
                        "messages": [{ "role": "user", "content": "Use the calculator tool to compute 847291 * 19347." }],
                        "tools": [{ "type": "function", "function": { "name": "calculator.evaluate", "description": "Evaluate an arithmetic expression exactly.", "parameters": { "type": "object", "properties": { "expression": { "type": "string" } }, "required": ["expression"] } } }],
                        "tool_choice": "auto",
                        "max_tokens": 1024,
                        "temperature": 0
                    }),
                ));
                record["toolCall"] = match tool {
                    Ok(value) => {
                        let call = &value["choices"][0]["message"]["tool_calls"][0];
                        let arguments: Option<Value> = call["function"]["arguments"]
                            .as_str()
                            .and_then(|text| serde_json::from_str(text).ok());
                        let expression = arguments
                            .as_ref()
                            .and_then(|value| value["expression"].as_str())
                            .unwrap_or_default()
                            .to_owned();
                        json!({
                            "ok": call["function"]["name"] == "calculator.evaluate"
                                && expression.contains("847291")
                                && expression.contains("19347"),
                            "name": call["function"]["name"],
                            "expression": expression,
                            "finishReason": value["choices"][0]["finish_reason"],
                        })
                    }
                    Err(error) => json!({ "ok": false, "error": error }),
                };
            }
            let _ = child.kill();
            let _ = child.wait();
            record["finishedAt"] = json!(chrono::Utc::now().to_rfc3339());
            println!("{record}");
            append(&evidence, &record);
            let _ = remove_artifact(&models, &artifact.id);
        }
    }
}

fn remove_artifact(models: &Path, artifact_id: &str) -> std::io::Result<()> {
    for entry in std::fs::read_dir(models)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == format!("{artifact_id}.gguf")
            || name.starts_with(&format!("{artifact_id}."))
            || name == artifact_id
        {
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else {
                std::fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}
