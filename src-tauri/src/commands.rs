use crate::domain::{
    Attachment, ChatRequest, DiscoveredModel, GgufSelection, ModelInspection, RuntimeLogEntry,
};
use crate::providers;
use crate::{catalog, device, managed_models};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque, hash_map::Entry},
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::{Notify, oneshot};
use uuid::Uuid;

#[derive(Default)]
pub struct AppState {
    pub cancellations: Mutex<HashMap<String, Cancellation>>,
    pub attachments: Mutex<HashMap<String, PathBuf>>,
    pub gguf_files: Mutex<HashMap<String, PathBuf>>,
    pub picker_in_progress: AtomicBool,
    pub permission_waiters: Mutex<HashMap<String, oneshot::Sender<String>>>,
    pub runtime_logs: Mutex<VecDeque<RuntimeLogEntry>>,
    #[cfg(not(target_os = "android"))]
    pub local_runtimes: crate::local_runtime::RuntimeProcesses,
}

struct PickerGuard<'a>(&'a AtomicBool);

impl<'a> PickerGuard<'a> {
    fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag))
    }
}

impl Drop for PickerGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub const MAX_RUNTIME_LOGS: usize = 200;
const MAX_ATTACHMENT_BYTES: u64 = 1024 * 1024;
const MAX_GGUF_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;
const MAX_APP_DATA_BYTES: usize = 64 * 1024 * 1024;

/// Text types Juniper will read as an attachment. The picker filter and the
/// grant check must agree, so both read this one list.
const SUPPORTED_ATTACHMENT_EXTENSIONS: &[&str] = &[
    "txt", "md", "json", "csv", "toml", "yaml", "yml", "rs", "ts", "tsx", "js", "jsx", "py", "css",
    "html",
];

#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn open_regular_file(path: &std::path::Path, unavailable: &str) -> Result<File, String> {
    let path_metadata = std::fs::symlink_metadata(path).map_err(|_| unavailable.to_owned())?;
    if path_metadata.file_type().is_symlink()
        || is_reparse_point(&path_metadata)
        || !path_metadata.is_file()
    {
        return Err(
            "Symbolic links, reparse points, and non-regular files are not accepted.".into(),
        );
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| unavailable.to_owned())?;
    if !file
        .metadata()
        .map_err(|_| unavailable.to_owned())?
        .is_file()
    {
        return Err("Only regular files are accepted.".into());
    }
    Ok(file)
}

fn bounded_log_label(value: Option<&str>) -> Option<String> {
    let value = value?;
    let bounded: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect();
    (!bounded.is_empty()).then_some(bounded)
}

pub fn record_runtime_log(
    state: &AppState,
    event: &str,
    code: Option<&str>,
    provider_kind: Option<&str>,
    model_id: Option<&str>,
) {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".into());
    if let Ok(mut logs) = state.runtime_logs.lock() {
        logs.push_back(RuntimeLogEntry {
            timestamp,
            event: bounded_log_label(Some(event)).unwrap_or_else(|| "runtime.event".into()),
            code: bounded_log_label(code),
            provider_kind: bounded_log_label(provider_kind),
            model_id: bounded_log_label(model_id),
        });
        while logs.len() > MAX_RUNTIME_LOGS {
            logs.pop_front();
        }
    }
}

/// Positive readiness signal: the webview loaded, React mounted, and stored
/// state was hydrated through IPC. Written to local stderr only.
#[tauri::command]
pub fn frontend_ready() {
    crate::startup::frontend_ready();
}

/// System bar, display cutout, and keyboard insets for the webview, in CSS
/// pixels. Android draws Juniper edge to edge, and older Android System
/// WebView builds do not expose these insets to CSS. Other platforms have no
/// such insets and return `None`.
#[tauri::command]
pub async fn window_insets(app: AppHandle) -> Option<Value> {
    #[cfg(target_os = "android")]
    {
        tokio::task::spawn_blocking(move || {
            juniper_local_runtime::invoke::<_, Value, _>(
                &app,
                "windowInsets",
                serde_json::json!({}),
            )
        })
        .await
        .ok()
        .and_then(Result::ok)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        None
    }
}

/// Bounded summary of an error that unmounted the interface, so a blank
/// window leaves a local diagnostic instead of nothing. Never sent anywhere.
#[tauri::command]
pub fn frontend_fatal(report: String) {
    crate::startup::frontend_fatal(&report);
}

#[tauri::command]
pub fn runtime_logs(state: State<'_, AppState>) -> Vec<RuntimeLogEntry> {
    state
        .runtime_logs
        .lock()
        .map(|logs| logs.iter().cloned().collect())
        .unwrap_or_default()
}

#[derive(Clone, Default)]
pub struct Cancellation {
    flag: Arc<Mutex<bool>>,
    notify: Arc<Notify>,
}

impl Cancellation {
    pub fn cancel(&self) {
        if let Ok(mut flag) = self.flag.lock() {
            *flag = true;
        }
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.lock().map(|flag| *flag).unwrap_or(true)
    }

    pub async fn wait(&self) {
        self.notify.notified().await;
    }
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn begin_cancellable_operation(state: &AppState, request_id: &str) -> Result<Cancellation, String> {
    if !valid_request_id(request_id) {
        return Err("Invalid request identifier.".into());
    }
    let cancellation = Cancellation::default();
    let mut operations = state
        .cancellations
        .lock()
        .map_err(|_| "Cancellation state unavailable.")?;
    match operations.entry(request_id.to_owned()) {
        Entry::Vacant(entry) => {
            entry.insert(cancellation.clone());
            Ok(cancellation)
        }
        Entry::Occupied(_) => Err("A request with this identifier is already active.".into()),
    }
}

#[tauri::command]
// `result` is only mutated by the Linux-only /proc/meminfo branch below, so on
// every other target the binding is not mutable and -D warnings would fail.
#[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
pub fn system_info() -> HashMap<String, String> {
    let mut result = HashMap::from([
        (
            String::from("application"),
            String::from("Juniper 0.3.0-rc.33"),
        ),
        (String::from("os"), std::env::consts::OS.to_owned()),
        (
            String::from("architecture"),
            std::env::consts::ARCH.to_owned(),
        ),
        (String::from("runtime"), String::from("Tauri 2 / Rust")),
        (
            String::from("database"),
            format!("SQLite schema v{}", crate::storage::SCHEMA_VERSION),
        ),
        (String::from("telemetry"), String::from("Off")),
    ]);
    #[cfg(target_os = "linux")]
    if let Ok(contents) = std::fs::read_to_string("/proc/meminfo")
        && let Some(line) = contents.lines().find(|line| line.starts_with("MemTotal:"))
    {
        result.insert("memory".into(), line.replace("MemTotal:", "").trim().into());
    }
    result
}

#[tauri::command]
pub fn model_catalog() -> Result<Vec<catalog::CatalogEntry>, String> {
    catalog::entries()
}

#[tauri::command]
pub async fn device_capabilities(app: AppHandle) -> Result<device::DeviceCapabilities, String> {
    let directory = managed_models::models_directory(&app)?;
    #[cfg(target_os = "android")]
    let mut capabilities = device::collect(&directory);
    #[cfg(not(target_os = "android"))]
    let mut capabilities = device::collect(&directory);
    #[cfg(target_os = "android")]
    {
        let native = crate::android_runtime::runtime_status(&app).await?;
        capabilities.native_runtime_available = native.runtime_available;
        capabilities.native_runtime_state = Some(native.state);
        let native_abi = native.abi.clone();
        capabilities.native_abi = native_abi.clone();
        capabilities.native_total_memory_bytes = native.total_memory_bytes;
        capabilities.native_available_memory_bytes = native.available_memory_bytes;
        capabilities.native_low_memory = native.low_memory;
        if let Some(value) = native.total_memory_bytes {
            capabilities.total_memory_bytes = Some(value);
        }
        if let Some(value) = native.available_memory_bytes {
            capabilities.available_memory_bytes = Some(value);
        }
        if let Some(value) = native.memory_pressure {
            capabilities.memory_pressure = value;
        }
        if let Some(value) = native_abi {
            capabilities.architecture = value.clone();
            capabilities.cpu_architecture = value;
        }
    }
    let packaged_llama = {
        #[cfg(target_os = "android")]
        {
            false
        }
        #[cfg(not(target_os = "android"))]
        {
            std::env::var_os("JUNIPER_LLAMA_SERVER")
                .is_some_and(|path| std::path::Path::new(&path).is_file())
                || app.path().resource_dir().ok().is_some_and(|directory| {
                    ["llama-server", "llama-server.exe"].iter().any(|name| {
                        directory.join("runtime").join(name).is_file()
                            || directory.join("binaries").join(name).is_file()
                    })
                })
        }
    };
    capabilities.runtimes = crate::runtime_registry::for_device(
        std::env::consts::OS,
        &capabilities.architecture,
        packaged_llama,
        capabilities.native_runtime_available.unwrap_or(false),
    );
    Ok(capabilities)
}

#[tauri::command]
pub async fn runtime_registry(
    app: AppHandle,
) -> Result<Vec<crate::runtime_registry::RuntimeDescriptor>, String> {
    Ok(device_capabilities(app).await?.runtimes)
}

#[tauri::command]
pub fn managed_models(app: AppHandle) -> Result<Vec<managed_models::ManagedModel>, String> {
    managed_models::list(&app)
}

#[tauri::command]
pub fn delete_managed_model(app: AppHandle, catalog_id: String) -> Result<(), String> {
    managed_models::remove(&app, &catalog_id)
}

#[tauri::command]
pub async fn download_managed_model(
    app: AppHandle,
    state: State<'_, AppState>,
    catalog_id: String,
    request_id: String,
) -> Result<(), String> {
    let cancellation = begin_cancellable_operation(state.inner(), &request_id)?;
    let result =
        managed_models::download(app, state.inner(), &catalog_id, &request_id, cancellation).await;
    state
        .cancellations
        .lock()
        .map_err(|_| "Cancellation state unavailable.")?
        .remove(&request_id);
    result
}

#[tauri::command]
pub fn cancel_managed_model(state: State<'_, AppState>, request_id: String) -> Result<(), String> {
    cancel_chat(state, request_id)
}

#[tauri::command]
pub fn load_app_data(app: AppHandle, state: State<'_, AppState>) -> Result<Option<Value>, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("juniper.db");
    let data =
        crate::storage::load_app_data(&path).map_err(|error| format!("DATABASE_ERROR: {error}"))?;
    let paths = crate::storage::load_attachment_paths(&path)
        .map_err(|error| format!("DATABASE_ERROR: {error}"))?;
    state
        .attachments
        .lock()
        .map_err(|_| "Attachment state unavailable.")?
        .extend(paths);
    Ok(data)
}

#[tauri::command]
pub fn save_app_data(
    app: AppHandle,
    state: State<'_, AppState>,
    data: Value,
) -> Result<(), String> {
    if !data.is_object()
        || serde_json::to_vec(&data)
            .map_err(|_| "Application data could not be encoded.".to_owned())?
            .len()
            > MAX_APP_DATA_BYTES
    {
        return Err("Application data is outside the allowed size or shape.".into());
    }
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("juniper.db");
    let attachment_paths = state
        .attachments
        .lock()
        .map_err(|_| "Attachment state unavailable.")?
        .clone();
    crate::storage::save_app_data_with_paths(&path, &data, &attachment_paths)
        .map_err(|error| format!("DATABASE_ERROR: {error}"))
}

#[tauri::command]
pub async fn health_check(
    state: State<'_, AppState>,
    kind: String,
    base_url: String,
    api_key_ref: Option<String>,
    device_link_fingerprint: Option<String>,
) -> Result<String, String> {
    let result = providers::health_check_with_pin(
        &kind,
        &base_url,
        api_key_ref.as_deref(),
        device_link_fingerprint.as_deref(),
    )
    .await;
    record_runtime_log(
        state.inner(),
        if result.is_ok() {
            "provider.connected"
        } else {
            "provider.connection_failed"
        },
        None,
        Some(&kind),
        None,
    );
    result
}

#[tauri::command]
pub async fn list_models(
    state: State<'_, AppState>,
    kind: String,
    base_url: String,
    api_key_ref: Option<String>,
    device_link_fingerprint: Option<String>,
) -> Result<Vec<DiscoveredModel>, String> {
    let result = providers::list_models_with_pin(
        &kind,
        &base_url,
        api_key_ref.as_deref(),
        device_link_fingerprint.as_deref(),
    )
    .await;
    record_runtime_log(
        state.inner(),
        if result.is_ok() {
            "provider.models_listed"
        } else {
            "provider.model_list_failed"
        },
        None,
        Some(&kind),
        None,
    );
    result
}

#[tauri::command]
pub async fn inspect_model(
    state: State<'_, AppState>,
    kind: String,
    base_url: String,
    model_id: String,
    api_key_ref: Option<String>,
    device_link_fingerprint: Option<String>,
) -> Result<ModelInspection, String> {
    let result = providers::inspect_model_with_pin(
        &kind,
        &base_url,
        &model_id,
        api_key_ref.as_deref(),
        device_link_fingerprint.as_deref(),
    )
    .await;
    record_runtime_log(
        state.inner(),
        if result.is_ok() {
            "model.inspected"
        } else {
            "model.inspection_failed"
        },
        None,
        Some(&kind),
        Some(&model_id),
    );
    result
}

#[tauri::command]
pub async fn pull_model(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: String,
    base_url: String,
    model_reference: String,
    request_id: String,
    api_key_ref: Option<String>,
) -> Result<(), String> {
    let cancellation = begin_cancellable_operation(state.inner(), &request_id)?;
    record_runtime_log(
        state.inner(),
        "model.pull_started",
        None,
        Some(&kind),
        Some(&model_reference),
    );
    let result = providers::pull_model(
        app,
        &kind,
        &base_url,
        &model_reference,
        &request_id,
        api_key_ref.as_deref(),
        cancellation,
    )
    .await;
    state
        .cancellations
        .lock()
        .map_err(|_| "Cancellation state unavailable.")?
        .remove(&request_id);
    record_runtime_log(
        state.inner(),
        if result.is_ok() {
            "model.pull_completed"
        } else {
            "model.pull_failed"
        },
        None,
        Some(&kind),
        Some(&model_reference),
    );
    result
}

#[tauri::command]
pub fn cancel_model_pull(state: State<'_, AppState>, request_id: String) -> Result<(), String> {
    cancel_chat(state, request_id)
}

#[tauri::command]
pub async fn import_gguf(
    app: AppHandle,
    state: State<'_, AppState>,
    selection_id: String,
    model_name: String,
    request_id: String,
) -> Result<(), String> {
    let model_name = model_name.trim();
    if !valid_model_reference(model_name) {
        return Err("Enter a model name using letters, numbers, '/', ':', '.', '_' or '-'.".into());
    }
    let path = state
        .gguf_files
        .lock()
        .map_err(|_| "GGUF selection state unavailable.")?
        .get(&selection_id)
        .cloned()
        .ok_or_else(|| "This GGUF selection is no longer available.".to_owned())?;
    let mut source = open_regular_file(&path, "The selected GGUF is unavailable.")?;
    let metadata = source
        .metadata()
        .map_err(|_| "The selected GGUF is unavailable.")?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_GGUF_BYTES {
        return Err("The selected GGUF file is no longer a readable regular file.".into());
    }
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("gguf-imports");
    std::fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;
    let import_id = Uuid::new_v4();
    let cached_gguf = cache_dir.join(format!("juniper-{import_id}.gguf"));
    let mut cached_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&cached_gguf)
        .map_err(|error| format!("Could not stage the GGUF import: {error}"))?;
    let staging_result = std::io::copy(&mut source, &mut cached_file)
        .and_then(|_| cached_file.flush())
        .map_err(|error| format!("Could not stage the GGUF import: {error}"));
    drop(cached_file);
    if let Err(error) = staging_result {
        let _ = std::fs::remove_file(&cached_gguf);
        return Err(error);
    }
    let modelfile = cache_dir.join(format!("juniper-{import_id}.Modelfile"));
    let prepare_result = cached_gguf
        .to_str()
        .ok_or_else(|| "The staged GGUF path is invalid.".to_owned())
        .and_then(|cached_path_text| {
            std::fs::write(&modelfile, format!("FROM {cached_path_text}\n"))
                .map_err(|error| format!("Could not prepare the Ollama import: {error}"))
        });
    if let Err(error) = prepare_result {
        let _ = std::fs::remove_file(&modelfile);
        let _ = std::fs::remove_file(&cached_gguf);
        return Err(error);
    }

    let cancellation = match begin_cancellable_operation(state.inner(), &request_id) {
        Ok(cancellation) => cancellation,
        Err(error) => {
            let _ = std::fs::remove_file(&modelfile);
            let _ = std::fs::remove_file(&cached_gguf);
            return Err(error);
        }
    };
    let topic = format!("juniper://gguf-import/{request_id}");
    let result = async {
        let modelfile_text = modelfile
            .to_str()
            .ok_or_else(|| "The temporary Modelfile path is invalid.".to_owned())?;
        let mut child = tokio::process::Command::new("ollama")
            .args(["create", model_name, "-f", modelfile_text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "Ollama could not be started for GGUF import.".to_owned())?;
        let status = tokio::select! {
            status = child.wait() => status.map_err(|error| format!("Ollama import failed: {error}")),
            _ = cancellation.wait() => {
                let _ = child.kill().await;
                emit_gguf_progress(&app, &topic, &request_id, "cancelled", Some("MODEL_IMPORT_CANCELLED"), "GGUF import cancelled.");
                return Err("GGUF import cancelled.".into());
            }
        }?;
        if !status.success() {
            emit_gguf_progress(&app, &topic, &request_id, "error", Some("MODEL_IMPORT_FAILED"), "Ollama could not import the GGUF file.");
            return Err("Ollama could not import the GGUF file.".into());
        }
        emit_gguf_progress(&app, &topic, &request_id, "complete", None, "GGUF import complete.");
        Ok(())
    }
    .await;
    state
        .cancellations
        .lock()
        .map_err(|_| "Cancellation state unavailable.")?
        .remove(&request_id);
    let _ = std::fs::remove_file(modelfile);
    let _ = std::fs::remove_file(cached_gguf);
    result
}

#[tauri::command]
pub fn cancel_gguf_import(state: State<'_, AppState>, request_id: String) -> Result<(), String> {
    cancel_chat(state, request_id)
}

fn valid_model_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '/' | ':' | '.' | '_' | '-')
        })
}

fn emit_gguf_progress(
    app: &AppHandle,
    topic: &str,
    request_id: &str,
    status: &str,
    code: Option<&str>,
    message: &str,
) {
    let _ = app.emit(
        topic,
        crate::domain::ModelPullProgress {
            request_id: request_id.into(),
            status: status.into(),
            digest: None,
            completed_bytes: None,
            total_bytes: None,
            done: Some(true),
            error: code.map(|code| crate::domain::RuntimeError {
                code: code.into(),
                message: message.into(),
            }),
        },
    );
}

#[tauri::command]
pub async fn delete_model(
    state: State<'_, AppState>,
    kind: String,
    base_url: String,
    model_id: String,
    api_key_ref: Option<String>,
) -> Result<(), String> {
    let result = providers::delete_model(&kind, &base_url, &model_id, api_key_ref.as_deref()).await;
    record_runtime_log(
        state.inner(),
        if result.is_ok() {
            "model.deleted"
        } else {
            "model.delete_failed"
        },
        None,
        Some(&kind),
        Some(&model_id),
    );
    result
}

#[tauri::command]
pub async fn running_models(
    kind: String,
    base_url: String,
    api_key_ref: Option<String>,
) -> Result<Vec<Value>, String> {
    providers::running_models(&kind, &base_url, api_key_ref.as_deref()).await
}

#[tauri::command]
pub async fn chat_stream(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ChatRequest,
) -> Result<(), String> {
    let cancellation = begin_cancellable_operation(state.inner(), &request.request_id)?;
    if request.provider.kind == "juniper-local" {
        let event_app = app.clone();
        #[cfg(target_os = "android")]
        let result =
            crate::android_runtime::stream_chat(app, request.clone(), cancellation, state.inner())
                .await;
        #[cfg(not(target_os = "android"))]
        let result =
            crate::local_runtime::stream_chat(app, request.clone(), cancellation, state.inner())
                .await;
        if let Err(error) = result {
            #[cfg(target_os = "android")]
            crate::android_runtime::emit_error(&event_app, &request.request_id, &error);
            #[cfg(not(target_os = "android"))]
            crate::local_runtime::emit_error(&event_app, &request.request_id, &error);
            record_runtime_log(
                state.inner(),
                "local_runtime.error",
                error.split(':').next(),
                Some("juniper-local"),
                Some(&request.model.model_id),
            );
        }
    } else {
        providers::stream(request.clone(), app, cancellation, state.inner()).await;
    }
    state
        .cancellations
        .lock()
        .map_err(|_| "Cancellation state unavailable.")?
        .remove(&request.request_id);
    Ok(())
}

#[tauri::command]
pub fn cancel_chat(state: State<'_, AppState>, request_id: String) -> Result<(), String> {
    if !valid_request_id(&request_id) {
        return Err("Invalid request identifier.".into());
    }
    if let Some(cancellation) = state
        .cancellations
        .lock()
        .map_err(|_| "Cancellation state unavailable.")?
        .get(&request_id)
    {
        cancellation.cancel();
    }
    Ok(())
}

#[tauri::command]
pub fn resolve_permission(
    state: State<'_, AppState>,
    request_id: String,
    call_id: String,
    decision: String,
) -> Result<(), String> {
    if !valid_request_id(&request_id)
        || call_id.is_empty()
        || call_id.len() > 256
        || call_id.chars().any(char::is_control)
    {
        return Err("Invalid permission request identifier.".into());
    }
    if !matches!(
        decision.as_str(),
        "allow-once" | "allow-chat" | "allow-assistant" | "deny"
    ) {
        return Err("Unknown permission decision.".into());
    }
    let key = format!("{request_id}:{call_id}");
    let sender = state
        .permission_waiters
        .lock()
        .map_err(|_| "Permission state unavailable.")?
        .remove(&key)
        .ok_or_else(|| "This permission request is no longer pending.".to_owned())?;
    sender
        .send(decision)
        .map_err(|_| "The permission request is no longer active.".into())
}

fn register_attachment_path(state: &AppState, path: PathBuf) -> Result<Attachment, String> {
    let file = open_regular_file(&path, "The selected file is not available.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "The selected file is not available.")?;
    if !metadata.is_file() {
        return Err("Only files can be attached.".into());
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err("The selected file is too large. Text attachments are limited to 1 MB.".into());
    }
    if !is_supported_attachment_path(&path) {
        return Err("This attachment type is not supported as text in this release.".into());
    }
    let id = Uuid::new_v4().to_string();
    state
        .attachments
        .lock()
        .map_err(|_| "Attachment state unavailable.")?
        .insert(id.clone(), path.clone());
    Ok(Attachment {
        id,
        name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment")
            .into(),
        size_bytes: metadata.len(),
        content_type: "text/plain".into(),
    })
}

fn is_supported_attachment_path(path: &std::path::Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    SUPPORTED_ATTACHMENT_EXTENSIONS.contains(&extension.as_str())
}

#[tauri::command]
pub async fn pick_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Attachment>, String> {
    let Some(_guard) = PickerGuard::acquire(&state.picker_in_progress) else {
        return Ok(None);
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Text files", SUPPORTED_ATTACHMENT_EXTENSIONS)
        .pick_file(move |selected| {
            let _ = tx.send(selected);
        });
    let selected = rx
        .await
        .map_err(|_| "The attachment picker closed unexpectedly.")?;
    let Some(file) = selected else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|_| "The selected file path is not available.")?;
    register_attachment_path(state.inner(), path).map(Some)
}

fn register_gguf(state: &AppState, path: PathBuf) -> Result<GgufSelection, String> {
    let mut file = open_regular_file(&path, "The selected file is not available.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "The selected file is not available.")?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("Only non-empty regular GGUF files can be selected.".into());
    }
    if metadata.len() > MAX_GGUF_BYTES {
        return Err("The selected GGUF file is larger than the supported limit.".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("gguf") {
        return Err("Select a file with the .gguf extension.".into());
    }
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|_| "The selected GGUF file is too short or not readable.")?;
    if &magic != b"GGUF" {
        return Err("The selected file does not have a valid GGUF header.".into());
    }
    let id = Uuid::new_v4().to_string();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("model.gguf")
        .to_owned();
    state
        .gguf_files
        .lock()
        .map_err(|_| "GGUF selection state unavailable.")?
        .insert(id.clone(), path);
    Ok(GgufSelection {
        id,
        name,
        size_bytes: metadata.len(),
    })
}

#[tauri::command]
pub async fn pick_gguf(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<GgufSelection>, String> {
    let Some(_guard) = PickerGuard::acquire(&state.picker_in_progress) else {
        return Ok(None);
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("GGUF models", &["gguf"])
        .pick_file(move |selected| {
            let _ = tx.send(selected);
        });
    let selected = rx
        .await
        .map_err(|_| "The GGUF picker closed unexpectedly.")?;
    let Some(file) = selected else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|_| "The selected GGUF path is not available.")?;
    register_gguf(state.inner(), path).map(Some)
}

#[tauri::command]
pub fn read_attachment(
    state: State<'_, AppState>,
    attachment_id: String,
) -> Result<String, String> {
    read_attachment_grant(state.inner(), &attachment_id)
}

fn read_attachment_grant(state: &AppState, attachment_id: &str) -> Result<String, String> {
    let path = state
        .attachments
        .lock()
        .map_err(|_| "Attachment state unavailable.")?
        .get(attachment_id)
        .cloned()
        .ok_or_else(|| "This attachment is no longer granted.".to_owned())?;
    let file = open_regular_file(&path, "The granted attachment is no longer available.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "The granted attachment is no longer available.")?;
    if !metadata.is_file()
        || metadata.len() > MAX_ATTACHMENT_BYTES
        || !is_supported_attachment_path(&path)
    {
        return Err("The granted attachment is no longer a supported text file.".into());
    }
    let mut contents = String::new();
    file.take(MAX_ATTACHMENT_BYTES + 1)
        .read_to_string(&mut contents)
        .map_err(|_| "The selected file could not be decoded as UTF-8 text.".to_owned())?;
    if contents.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err("The selected file is too large.".into());
    }
    Ok(contents)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn credential_entry(reference: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("com.cinqic.juniper", reference)
        .map_err(|_| "The system credential store rejected this reference.".into())
}

pub(crate) fn valid_credential_reference(reference: &str) -> bool {
    !reference.is_empty()
        && reference.len() <= 160
        && reference.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
}

#[tauri::command]
pub fn secure_set_credential(
    app: AppHandle,
    reference: String,
    secret: String,
) -> Result<(), String> {
    if !valid_credential_reference(&reference) || secret.is_empty() || secret.len() > 8192 {
        return Err("Credential reference or value is outside the allowed limit.".into());
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let _ = app;
        credential_entry(&reference)?
            .set_password(&secret)
            .map_err(|_| "Could not save the credential to the system keychain.".into())
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        #[cfg(target_os = "android")]
        {
            juniper_local_runtime::secure_set_credential(&app, &reference, &secret)
        }
        #[cfg(target_os = "ios")]
        {
            let _ = (app, secret);
            Err("Secure provider credentials on iOS are not available in this build.".into())
        }
    }
}

#[tauri::command]
pub fn secure_delete_credential(app: AppHandle, reference: String) -> Result<(), String> {
    if !valid_credential_reference(&reference) {
        return Err("Credential reference is invalid.".into());
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let _ = app;
        credential_entry(&reference)?
            .delete_credential()
            .map_err(|_| "Could not remove the credential from the system keychain.".into())
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        #[cfg(target_os = "android")]
        {
            juniper_local_runtime::secure_delete_credential(&app, &reference)
        }
        #[cfg(target_os = "ios")]
        {
            let _ = reference;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_picker_guard_allows_only_one_dialog_until_it_closes() {
        let flag = AtomicBool::new(false);
        let first = PickerGuard::acquire(&flag).expect("first picker may open");
        assert!(PickerGuard::acquire(&flag).is_none());
        drop(first);
        assert!(PickerGuard::acquire(&flag).is_some());
    }

    fn temporary_path(extension: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("juniper-command-test-{suffix}.{extension}"))
    }

    #[test]
    fn attachment_picker_grant_is_opaque_and_reads_only_granted_text() {
        let path = temporary_path("txt");
        std::fs::write(&path, "approved attachment content").expect("fixture should write");
        let state = AppState::default();
        let attachment = register_attachment_path(&state, path.clone()).expect("grant should work");
        assert_eq!(attachment.name, path.file_name().unwrap().to_string_lossy());
        assert_eq!(
            read_attachment_grant(&state, &attachment.id).expect("granted content should read"),
            "approved attachment content"
        );
        assert!(read_attachment_grant(&state, "not-granted").is_err());
        std::fs::write(&path, vec![b'x'; 1024 * 1024 + 1]).expect("replacement should write");
        assert!(read_attachment_grant(&state, &attachment.id).is_err());
        std::fs::remove_file(path).expect("temporary attachment should be removable");
    }

    #[test]
    fn attachment_read_revalidates_deleted_and_non_utf8_files() {
        let path = temporary_path("txt");
        let state = AppState::default();
        std::fs::write(&path, b"initially valid").expect("fixture should write");
        let grant = register_attachment_path(&state, path.clone()).expect("grant should work");
        std::fs::write(&path, [0xff, 0xfe]).expect("replacement should write");
        assert!(read_attachment_grant(&state, &grant.id).is_err());
        std::fs::remove_file(&path).expect("fixture should be removable");
        assert!(read_attachment_grant(&state, &grant.id).is_err());
    }

    #[test]
    fn attachment_grant_rejects_unsupported_types_and_oversized_files() {
        let unsupported = temporary_path("bin");
        std::fs::write(&unsupported, b"not text").expect("fixture should write");
        let state = AppState::default();
        let error = register_attachment_path(&state, unsupported.clone())
            .expect_err("unsupported attachment should be rejected");
        assert!(error.contains("not supported"));
        std::fs::remove_file(unsupported).expect("temporary attachment should be removable");

        let oversized = temporary_path("txt");
        std::fs::write(&oversized, vec![b'x'; 1024 * 1024 + 1]).expect("fixture should write");
        let error = register_attachment_path(&state, oversized.clone())
            .expect_err("oversized attachment should be rejected");
        assert!(error.contains("too large"));
        std::fs::remove_file(oversized).expect("temporary attachment should be removable");
    }

    #[cfg(unix)]
    #[test]
    fn attachment_grant_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let target = temporary_path("txt");
        let link = target.with_file_name(format!(
            "{}-link.txt",
            target.file_stem().unwrap_or_default().to_string_lossy()
        ));
        std::fs::write(&target, "linked content").expect("fixture should write");
        symlink(&target, &link).expect("fixture symlink should be created");
        let state = AppState::default();
        assert!(register_attachment_path(&state, link.clone()).is_err());
        let grant = register_attachment_path(&state, target.clone()).expect("target should grant");
        std::fs::remove_file(&target).expect("target should be removable");
        symlink(&link, &target).expect("replacement symlink should be created");
        assert!(read_attachment_grant(&state, &grant.id).is_err());
        std::fs::remove_file(&target).expect("replacement symlink should be removable");
        std::fs::remove_file(link).expect("fixture symlink should be removable");
    }

    #[test]
    fn gguf_picker_requires_extension_and_magic_header() {
        let invalid = temporary_path("gguf");
        std::fs::write(&invalid, b"NOPE").expect("fixture should write");
        let state = AppState::default();
        let error = register_gguf(&state, invalid.clone()).expect_err("invalid GGUF should fail");
        assert!(error.contains("valid GGUF header"));
        std::fs::remove_file(invalid).expect("temporary GGUF should be removable");

        let valid = temporary_path("gguf");
        std::fs::write(&valid, b"GGUFfixture").expect("fixture should write");
        let selection = register_gguf(&state, valid.clone()).expect("valid GGUF should pass");
        assert_eq!(selection.name, valid.file_name().unwrap().to_string_lossy());
        assert!(state.gguf_files.lock().unwrap().contains_key(&selection.id));
        std::fs::remove_file(valid).expect("temporary GGUF should be removable");
    }

    #[test]
    fn imported_model_reference_is_not_a_shell_fragment() {
        assert!(valid_model_reference("future-model:7b"));
        assert!(!valid_model_reference("library/model@sha256:abc"));
        assert!(!valid_model_reference("model; rm -rf /"));
        assert!(!valid_model_reference("model name"));
        assert!(!valid_model_reference(""));
    }

    #[test]
    fn duplicate_and_malformed_request_ids_are_rejected() {
        let state = AppState::default();
        let first = begin_cancellable_operation(&state, "request-1")
            .expect("first request should register");
        assert!(begin_cancellable_operation(&state, "request-1").is_err());
        assert!(begin_cancellable_operation(&state, "bad\nrequest").is_err());
        first.cancel();
    }

    #[test]
    fn credential_references_are_opaque_and_bounded() {
        assert!(valid_credential_reference("credential-1234_abcd.v1"));
        assert!(!valid_credential_reference(""));
        assert!(!valid_credential_reference("credential/../../secret"));
        assert!(!valid_credential_reference("credential\nsecond"));
    }

    #[test]
    fn runtime_logs_are_bounded_and_contain_only_event_metadata() {
        let state = AppState::default();
        for index in 0..(MAX_RUNTIME_LOGS + 3) {
            record_runtime_log(
                &state,
                &format!("event-{index}"),
                Some("TEST_CODE"),
                Some("ollama"),
                Some("future-model"),
            );
        }
        let logs = state.runtime_logs.lock().expect("runtime logs should lock");
        assert_eq!(logs.len(), MAX_RUNTIME_LOGS);
        assert_eq!(
            logs.front().expect("oldest log should remain").event,
            "event-3"
        );
        assert_eq!(
            logs.back().expect("newest log should remain").event,
            "event-202"
        );
        assert_eq!(
            logs.back()
                .expect("newest log should remain")
                .model_id
                .as_deref(),
            Some("future-model")
        );
    }
}
