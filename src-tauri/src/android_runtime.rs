use crate::commands::{AppState, Cancellation, record_runtime_log};
use crate::domain::{ChatRequest, ChatStreamEvent, RuntimeError, Usage};
use crate::managed_models;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::time::{Duration, Instant, sleep};

const CONTEXT_SIZE: u32 = 2048;
const LOAD_TIMEOUT: Duration = Duration::from_secs(120);
const GENERATION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadModelArgs {
    path: String,
    context_size: u32,
    threads: u32,
    expected_model_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestArgs {
    request_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateArgs {
    request_id: String,
    messages_json: String,
    max_output: u32,
    temperature: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Accepted {
    #[allow(dead_code)]
    accepted: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeStatus {
    pub(crate) state: String,
    pub(crate) loaded_path: Option<String>,
    pub(crate) runtime_available: Option<bool>,
    pub(crate) abi: Option<String>,
    pub(crate) total_memory_bytes: Option<u64>,
    pub(crate) available_memory_bytes: Option<u64>,
    pub(crate) low_memory: Option<bool>,
    pub(crate) memory_pressure: Option<String>,
    pub(crate) failure_code: Option<String>,
    pub(crate) failure_message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeEvent {
    available: bool,
    kind: Option<String>,
    request_id: Option<String>,
    text: Option<String>,
    code: Option<String>,
    message: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    duration_ms: Option<u64>,
}

async fn call<R, T, P>(app: &AppHandle<R>, command: &'static str, payload: P) -> Result<T, String>
where
    R: Runtime,
    T: DeserializeOwned + Send + 'static,
    P: Serialize + Send + 'static,
{
    let app = app.clone();
    tokio::task::spawn_blocking(move || juniper_local_runtime::invoke(&app, command, payload))
        .await
        .map_err(|_| {
            "LOCAL_RUNTIME_PLUGIN_ERROR: Native runtime call was interrupted.".to_owned()
        })?
}

fn request_topic(request_id: &str) -> String {
    format!("juniper://chat/{request_id}")
}

fn emit<R: Runtime>(app: &AppHandle<R>, event: ChatStreamEvent) {
    let _ = app.emit(&request_topic(&event.request_id), event);
}

pub fn emit_error<R: Runtime>(app: &AppHandle<R>, request_id: &str, error: &str) {
    let mut pieces = error.splitn(2, ':');
    let code = pieces.next().unwrap_or("LOCAL_RUNTIME_FAILED");
    let message = pieces.next().unwrap_or(error).trim();
    emit(
        app,
        ChatStreamEvent {
            request_id: request_id.into(),
            delta: None,
            activity: None,
            tool_calls: None,
            tool_results: None,
            done: Some(true),
            usage: None,
            error: Some(RuntimeError {
                code: code.into(),
                message: message.into(),
            }),
            permission_request: None,
            provenance: None,
        },
    );
}

fn native_error(status: &NativeStatus) -> String {
    if let Some(code) = status
        .failure_code
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        let message = status
            .failure_message
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("The native local runtime rejected the model.");
        return format!("{code}: {message}");
    }
    match status.state.as_str() {
        "unavailable" => {
            "LOCAL_RUNTIME_UNAVAILABLE: The Android native runtime is unavailable for this ABI."
                .into()
        }
        "failed" => {
            "LOCAL_MODEL_LOAD_FAILED: The managed GGUF could not be loaded by the native engine."
                .into()
        }
        _ => "LOCAL_RUNTIME_FAILED: The native local runtime entered an unexpected state.".into(),
    }
}

pub async fn runtime_status<R: Runtime>(app: &AppHandle<R>) -> Result<NativeStatus, String> {
    call(
        app,
        "pollStatus",
        RequestArgs {
            request_id: String::new(),
        },
    )
    .await
}

async fn wait_until_ready<R: Runtime>(
    app: &AppHandle<R>,
    model_path: &PathBuf,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let model_path = model_path.to_string_lossy().into_owned();
    let initial: NativeStatus = call(
        app,
        "pollStatus",
        RequestArgs {
            request_id: String::new(),
        },
    )
    .await?;
    if initial.state == "busy" {
        return Err("LOCAL_RUNTIME_BUSY: Another local generation is already running.".into());
    }
    if !(initial.state == "ready" && initial.loaded_path.as_deref() == Some(model_path.as_str())) {
        call::<_, Accepted, _>(
            app,
            "loadModel",
            LoadModelArgs {
                path: model_path.clone(),
                context_size: CONTEXT_SIZE,
                threads: 4,
                expected_model_bytes: std::fs::metadata(&model_path)
                    .map_err(|_| "LOCAL_MODEL_NOT_READY: The managed model is unavailable.")?
                    .len(),
            },
        )
        .await?;
    }

    let deadline = Instant::now() + LOAD_TIMEOUT;
    loop {
        if cancellation.is_cancelled() {
            let _ = call::<_, Accepted, _>(
                app,
                "unload",
                RequestArgs {
                    request_id: String::new(),
                },
            )
            .await;
            return Err("REQUEST_CANCELLED: Generation cancelled.".into());
        }
        let status: NativeStatus = call(
            app,
            "pollStatus",
            RequestArgs {
                request_id: String::new(),
            },
        )
        .await?;
        match status.state.as_str() {
            "ready" if status.loaded_path.as_deref() == Some(model_path.as_str()) => return Ok(()),
            "failed" => return Err(native_error(&status)),
            "busy" => {
                return Err(
                    "LOCAL_RUNTIME_BUSY: Another local generation is already running.".into(),
                );
            }
            _ if Instant::now() >= deadline => {
                return Err(
                    "LOCAL_MODEL_LOAD_TIMEOUT: The managed model took too long to load.".into(),
                );
            }
            _ => {}
        }
        tokio::select! {
            _ = sleep(POLL_INTERVAL) => {},
            _ = cancellation.wait() => {},
        }
    }
}

pub async fn stream_chat<R: Runtime>(
    app: AppHandle<R>,
    request: ChatRequest,
    cancellation: Cancellation,
    state: &AppState,
) -> Result<(), String> {
    let catalog_id = request
        .model
        .catalog_id
        .clone()
        .unwrap_or_else(|| request.model.model_id.clone());
    let model_path = managed_models::path_for_catalog(&app, &catalog_id)?;
    record_runtime_log(
        state,
        "local_runtime.native_starting",
        None,
        Some("juniper-local"),
        Some(&catalog_id),
    );

    wait_until_ready(&app, &model_path, &cancellation).await?;
    if cancellation.is_cancelled() {
        return Err("REQUEST_CANCELLED: Generation cancelled.".into());
    }

    let messages = crate::providers::composed_messages(&request)?;
    let messages_json = serde_json::to_string(&messages)
        .map_err(|_| "PROMPT_FORMAT_FAILED: The chat messages could not be encoded.".to_owned())?;
    call::<_, Accepted, _>(
        &app,
        "startGenerate",
        GenerateArgs {
            request_id: request.request_id.clone(),
            messages_json,
            max_output: request.generation.max_output.unwrap_or(256).clamp(1, 512),
            temperature: request
                .generation
                .temperature
                .unwrap_or(0.3)
                .clamp(0.0, 2.0),
        },
    )
    .await?;

    let deadline = Instant::now() + GENERATION_TIMEOUT;
    let mut cancel_sent = false;
    loop {
        if cancellation.is_cancelled() && !cancel_sent {
            cancel_sent = true;
            let _ = call::<_, Accepted, _>(
                &app,
                "cancel",
                RequestArgs {
                    request_id: request.request_id.clone(),
                },
            )
            .await;
        }

        if Instant::now() >= deadline {
            let _ = call::<_, Accepted, _>(
                &app,
                "cancel",
                RequestArgs {
                    request_id: request.request_id.clone(),
                },
            )
            .await;
            return Err("LOCAL_RUNTIME_TIMEOUT: The local model did not finish generation.".into());
        }

        let event: NativeEvent = call(
            &app,
            "pollEvent",
            RequestArgs {
                request_id: request.request_id.clone(),
            },
        )
        .await?;
        if event.available {
            match event.kind.as_deref() {
                Some("delta") => {
                    emit(
                        &app,
                        ChatStreamEvent {
                            request_id: event
                                .request_id
                                .clone()
                                .unwrap_or_else(|| request.request_id.clone()),
                            delta: event.text,
                            activity: None,
                            tool_calls: None,
                            tool_results: None,
                            done: Some(false),
                            usage: None,
                            error: None,
                            permission_request: None,
                            provenance: None,
                        },
                    );
                }
                Some("done") => {
                    let error =
                        event
                            .code
                            .filter(|code| !code.is_empty())
                            .map(|code| RuntimeError {
                                code,
                                message: event
                                    .message
                                    .unwrap_or_else(|| "The native runtime failed.".into()),
                            });
                    emit(
                        &app,
                        ChatStreamEvent {
                            request_id: request.request_id.clone(),
                            delta: None,
                            activity: None,
                            tool_calls: None,
                            tool_results: None,
                            done: Some(true),
                            usage: Some(Usage {
                                input_tokens: event.input_tokens,
                                output_tokens: event.output_tokens,
                                total_tokens: event
                                    .input_tokens
                                    .zip(event.output_tokens)
                                    .map(|(input, output)| input + output),
                                duration_ms: event.duration_ms,
                            }),
                            error,
                            permission_request: None,
                            provenance: None,
                        },
                    );
                    return Ok(());
                }
                _ => {}
            }
        }
        sleep(POLL_INTERVAL).await;
    }
}
