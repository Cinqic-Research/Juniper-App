//! Juniper's resident desktop `llama-server`.
//!
//! One server runs at a time, owned by Juniper, bound to loopback, protected
//! by a per-launch API key, and kept loaded across turns: a 12 GB model that
//! takes minutes to load cannot be started per request. A model with a
//! qualified backend profile is started only with that profile's exact flags,
//! template, and runtime build, and its identity is checked after load.
//! Juniper 1 is started only through its token-ID profile (`crate::juniper1`)
//! and is then served by the host tokenizer and protocol, never by
//! llama-server's text endpoints.

use crate::backend::{self, Backend, BackendProfile, RequestPolicy};
use crate::catalog::{self, CatalogArtifact, CatalogEntry};
use crate::commands::{AppState, Cancellation, record_runtime_log};
use crate::domain::{ChatRequest, ChatStreamEvent, RuntimeError, RuntimeIdentity};
use crate::juniper1::{self, chat::TokenIdRuntime};
use crate::managed_models;
use crate::providers;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{Duration, Instant, sleep, timeout};

const GENERIC_STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
/// Silence allowed on a Juniper-owned server's stream without a qualified
/// profile. Fixed here rather than taken from the provider loop, whose test
/// builds shorten it for fake servers.
const GENERIC_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const HEALTH_INTERVAL: Duration = Duration::from_millis(250);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const WARM_UP_TIMEOUT: Duration = Duration::from_secs(300);
const GENERIC_CACHE_RAM_MIB: &str = "1024";
/// The context a generic model is served with: its own length, capped so
/// the KV cache stays predictable on modest hardware.
const GENERIC_CONTEXT_CAP: u64 = 16_384;

fn generic_context(entry: &CatalogEntry) -> u32 {
    entry.context_length.clamp(2_048, GENERIC_CONTEXT_CAP) as u32
}
const STDERR_TAIL_LINES: usize = 48;
const STDERR_LINE_CHARS: usize = 400;

#[derive(Default)]
pub struct LocalRuntime {
    /// Serializes start, restart, switch, and unload decisions.
    lifecycle: tokio::sync::Mutex<()>,
    resident: Mutex<Option<Resident>>,
    /// A server between spawn and readiness, held here so application exit
    /// can stop it; loading a large model takes minutes.
    starting: Mutex<Option<tokio::process::Child>>,
    loading: Mutex<Option<String>>,
}

struct Resident {
    artifact_id: String,
    child: tokio::process::Child,
    route: Route,
    leases: Arc<AtomicUsize>,
}

impl Resident {
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Everything a request needs to talk to the resident server.
#[derive(Clone)]
struct Route {
    endpoint: String,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    port: u16,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    owner_pid: Option<u32>,
    api_key: String,
    identity: RuntimeIdentity,
    profile: Option<BackendProfile>,
    /// Set for a Juniper 1 server: requests go through the token-ID backend.
    token_ids: Option<Arc<TokenIdRuntime>>,
    lineage: String,
}

/// What `acquire` starts for a request.
enum Launch {
    Text(Box<TextLaunch>),
    TokenIds,
}

struct TextLaunch {
    entry: CatalogEntry,
    artifact: CatalogArtifact,
    profile: Option<BackendProfile>,
}

/// Marks the resident server as in use; switching models or unloading waits
/// for in-flight generations instead of killing them.
pub(crate) struct Lease(Arc<AtomicUsize>);

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalRuntimeStatus {
    /// `idle`, `loading`, `ready`, or `busy`.
    pub state: String,
    pub artifact_id: Option<String>,
    pub identity: Option<RuntimeIdentity>,
}

impl LocalRuntime {
    pub fn status(&self) -> LocalRuntimeStatus {
        if let Some(artifact_id) = self.loading.lock().ok().and_then(|loading| loading.clone()) {
            return LocalRuntimeStatus {
                state: "loading".into(),
                artifact_id: Some(artifact_id),
                identity: None,
            };
        }
        let Ok(mut resident) = self.resident.lock() else {
            return idle_status();
        };
        match resident
            .as_mut()
            .and_then(|server| server.alive().then_some(server))
        {
            Some(server) => LocalRuntimeStatus {
                state: if server.leases.load(Ordering::Acquire) > 0 {
                    "busy"
                } else {
                    "ready"
                }
                .into(),
                artifact_id: Some(server.artifact_id.clone()),
                identity: Some(server.route.identity.clone()),
            },
            None => idle_status(),
        }
    }

    /// Stops the resident server unless a generation is using it.
    pub async fn unload(&self) -> Result<bool, String> {
        let _lifecycle = self.lifecycle.lock().await;
        let server = {
            let mut resident = self.resident.lock().map_err(|_| state_error())?;
            if resident
                .as_ref()
                .is_some_and(|server| server.leases.load(Ordering::Acquire) > 0)
            {
                return Err(
                    "LOCAL_RUNTIME_BUSY: Stop the current generation before unloading the model."
                        .into(),
                );
            }
            resident.take()
        };
        match server {
            Some(mut server) => {
                let _ = server.child.kill().await;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Signals the resident server to stop. Synchronous so it can run from the
    /// exit event after the async runtime stops being polled; Tauri exits with
    /// `std::process::exit`, which runs no destructors.
    pub fn terminate_all(&self) -> usize {
        let mut stopped = 0;
        if let Some(mut server) = self.resident.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = server.child.start_kill();
            stopped += 1;
        }
        if let Some(mut child) = self.starting.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = child.start_kill();
            stopped += 1;
        }
        stopped
    }

    async fn acquire<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        request: &ChatRequest,
        cancellation: &Cancellation,
        state: &AppState,
    ) -> Result<(Lease, Route), String> {
        let catalog_id = request
            .model
            .catalog_id
            .clone()
            .unwrap_or_else(|| request.model.model_id.clone());
        // Juniper 1 artifacts are named only by the token-ID profile, never by
        // the public catalog, and run only through that backend.
        let (artifact_id, launch) = match juniper1::profile::find(&catalog_id) {
            Some((_, artifact)) => (artifact.id, Launch::TokenIds),
            None => {
                let entry = catalog::find(&catalog_id)?;
                // The model profile names the artifact the user installed;
                // without one, the first installed artifact of the entry.
                let artifact = match request.model.artifact_id.as_deref() {
                    Some(id) => entry.artifacts.iter().find(|artifact| artifact.id == id).cloned().ok_or_else(|| {
                        "LOCAL_MODEL_NOT_READY: This model's download is no longer in the catalog.".to_owned()
                    })?,
                    None => managed_models::path_for_catalog(app, &catalog_id, None)
                        .map(|(_, artifact)| artifact)
                        .or_else(|_| {
                            entry.artifacts.first().cloned().ok_or_else(|| {
                                "LOCAL_MODEL_NOT_READY: This catalog entry has no artifact.".to_owned()
                            })
                        })?,
                };
                if !catalog::supports_current_device(&artifact) {
                    return Err("LOCAL_MODEL_UNSUPPORTED: This model artifact does not support this device.".into());
                }
                let profile = qualified_profile(&artifact)?;
                (
                    artifact.id.clone(),
                    Launch::Text(Box::new(TextLaunch {
                        entry,
                        artifact,
                        profile,
                    })),
                )
            }
        };

        let _lifecycle = self.lifecycle.lock().await;
        let reusable = {
            let mut resident = self.resident.lock().map_err(|_| state_error())?;
            resident
                .as_mut()
                .filter(|server| server.artifact_id == artifact_id)
                .and_then(|server| server.alive().then_some(server))
                .map(|server| {
                    server.leases.fetch_add(1, Ordering::AcqRel);
                    (Lease(server.leases.clone()), server.route.clone())
                })
        };
        if let Some((lease, route)) = reusable {
            // A busy server can be slow to answer; one missed check is not a crash.
            for _ in 0..3 {
                if healthy(&route).await {
                    return Ok((lease, route));
                }
                sleep(Duration::from_secs(1)).await;
            }
        }
        let (previous, restarting) = {
            let mut resident = self.resident.lock().map_err(|_| state_error())?;
            let in_use_by_another_chat = resident.as_mut().is_some_and(|server| {
                server.artifact_id == artifact_id
                    && server.alive()
                    && server.leases.load(Ordering::Acquire) > 0
            });
            if in_use_by_another_chat {
                return Err("LOCAL_RUNTIME_BUSY: The local model is not responding while another chat is using it. Wait for that reply or stop it, then try again.".into());
            }
            match resident.as_ref() {
                Some(server) if server.artifact_id == artifact_id => {
                    record_runtime_log(
                        state,
                        "local_runtime.lost",
                        Some("LOCAL_RUNTIME_LOST"),
                        Some("juniper-local"),
                        Some(&catalog_id),
                    );
                    (resident.take(), true)
                }
                Some(server) if server.leases.load(Ordering::Acquire) > 0 => {
                    return Err("LOCAL_RUNTIME_BUSY: Another chat is still using a different local model. Wait for it to finish or stop it, then try again.".into());
                }
                _ => (resident.take(), false),
            }
        };
        if let Some(mut server) = previous {
            let _ = server.child.kill().await;
        }

        emit_activity(
            app,
            &request.request_id,
            if restarting {
                "restarting-model"
            } else {
                "loading-model"
            },
        );
        if let Ok(mut loading) = self.loading.lock() {
            *loading = Some(artifact_id.clone());
        }
        record_runtime_log(
            state,
            "local_runtime.starting",
            None,
            Some("juniper-local"),
            Some(&catalog_id),
        );
        let started = match launch {
            Launch::Text(text) => {
                start(
                    app,
                    request,
                    &text.entry,
                    &text.artifact,
                    text.profile,
                    cancellation,
                    &self.starting,
                )
                .await
            }
            Launch::TokenIds => {
                start_token_ids(app, &artifact_id, cancellation, &self.starting).await
            }
        };
        if let Ok(mut loading) = self.loading.lock() {
            *loading = None;
        }
        let (child, route) = started.inspect_err(|error| {
            record_runtime_log(
                state,
                "local_runtime.failed",
                error.split(':').next(),
                Some("juniper-local"),
                Some(&catalog_id),
            );
        })?;
        record_runtime_log(
            state,
            "local_runtime.ready",
            None,
            Some("juniper-local"),
            Some(&catalog_id),
        );
        let leases = Arc::new(AtomicUsize::new(1));
        let lease = Lease(leases.clone());
        *self.resident.lock().map_err(|_| state_error())? = Some(Resident {
            artifact_id,
            child,
            route: route.clone(),
            leases,
        });
        Ok((lease, route))
    }
}

#[cfg(all(test, unix, not(target_os = "android")))]
impl LocalRuntime {
    /// The resident server's endpoint, key, and process ID, for hardware tests.
    pub(crate) fn resident_for_tests(&self) -> Option<(String, String, u32)> {
        let resident = self.resident.lock().ok()?;
        let server = resident.as_ref()?;
        Some((
            server.route.endpoint.clone(),
            server.route.api_key.clone(),
            server.child.id()?,
        ))
    }
}

#[cfg(all(test, target_os = "linux"))]
impl LocalRuntime {
    /// The resident Juniper 1 runtime, for hardware tests that drive its client directly.
    pub(crate) fn token_ids_for_tests(&self) -> Option<Arc<TokenIdRuntime>> {
        let resident = self.resident.lock().ok()?;
        resident.as_ref()?.route.token_ids.clone()
    }
}

fn idle_status() -> LocalRuntimeStatus {
    LocalRuntimeStatus {
        state: "idle".into(),
        artifact_id: None,
        identity: None,
    }
}

fn state_error() -> String {
    "LOCAL_RUNTIME_FAILED: Runtime process state unavailable.".into()
}

/// The backend profile a catalog artifact names, checked against the artifact
/// so the profile cannot be applied to a different file.
fn qualified_profile(artifact: &CatalogArtifact) -> Result<Option<BackendProfile>, String> {
    let Some(name) = artifact.backend_profile.as_deref() else {
        return Ok(None);
    };
    let profile = backend::profile(name)?;
    if profile.artifact.id != artifact.id
        || artifact.sha256.as_deref() != Some(profile.artifact.sha256.as_str())
        || artifact.size_bytes != profile.artifact.size_bytes
    {
        return Err("BACKEND_PROFILE_MISMATCH: The model artifact does not match its qualified backend profile.".into());
    }
    Ok(Some(profile))
}

/// Starts Juniper's private, loopback-only llama-server for one generation
/// and keeps it resident for the following ones.
pub async fn stream_chat<R: Runtime>(
    app: AppHandle<R>,
    request: ChatRequest,
    cancellation: Cancellation,
    state: &AppState,
) -> Result<(), String> {
    providers::validate_local_request(&request)?;
    let (lease, route) = state
        .local_runtime
        .acquire(&app, &request, &cancellation, state)
        .await?;
    if let Some(runtime) = &route.token_ids {
        juniper1::chat::stream_chat(
            &app,
            &request,
            &cancellation,
            state,
            runtime,
            route.identity.clone(),
        )
        .await;
        drop(lease);
        return Ok(());
    }
    let policy = text_policy(&route);
    let mut normalized = request;
    normalized.provider.kind = "openai-compatible".into();
    normalized.provider.base_url = route.endpoint.clone();
    providers::stream(
        normalized,
        app.clone(),
        cancellation,
        state,
        policy,
        Some(route.identity.clone()),
    )
    .await;
    drop(lease);
    Ok(())
}

/// The request policy for a text-mode route to the resident server.
fn text_policy(route: &Route) -> RequestPolicy {
    RequestPolicy {
        idle_timeout: route
            .profile
            .as_ref()
            .map_or(GENERIC_IDLE_TIMEOUT, |profile| {
                Duration::from_secs(profile.lifecycle.first_token_timeout_seconds)
            }),
        // The served context is fixed at launch, so every prompt is measured
        // against it before generation.
        context_window: route.identity.context_size,
        backend: route.profile.clone().map_or(Backend::Generic, |profile| {
            Backend::GptOss(Box::new(profile))
        }),
        loopback_key: Some(route.api_key.clone()),
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        loopback_owner: route.owner_pid.map(|pid| (pid, route.port)),
        lineage: Some(route.lineage.clone()),
    }
}

/// Starts or reuses the resident server for one agent model turn. The lease
/// is held only for that turn, so chats and agent tasks share one resident
/// model and the server is never started twice. Juniper 1's token-ID backend
/// is not qualified for agent work and is refused.
pub(crate) async fn agent_route<R: Runtime>(
    app: &AppHandle<R>,
    request: &ChatRequest,
    cancellation: &Cancellation,
    state: &AppState,
) -> Result<(Lease, ChatRequest, RequestPolicy), String> {
    providers::validate_local_request(request)?;
    if juniper1::profile::find(
        request
            .model
            .catalog_id
            .as_deref()
            .unwrap_or(&request.model.model_id),
    )
    .is_some()
    {
        return Err("AGENT_MODEL_NOT_QUALIFIED: Juniper 1 is not qualified for agent tasks. Choose another model.".into());
    }
    let (lease, route) = state
        .local_runtime
        .acquire(app, request, cancellation, state)
        .await?;
    let policy = text_policy(&route);
    let mut normalized = request.clone();
    normalized.provider.kind = "openai-compatible".into();
    normalized.provider.base_url = route.endpoint.clone();
    Ok((lease, normalized, policy))
}

// Starting a server needs the request (for activity events), the catalog
// entry and artifact, the qualified profile, cancellation, and the slot that
// keeps the child reachable from exit cleanup.
#[allow(clippy::too_many_arguments)]
async fn start<R: Runtime>(
    app: &AppHandle<R>,
    request: &ChatRequest,
    entry: &CatalogEntry,
    artifact: &CatalogArtifact,
    profile: Option<BackendProfile>,
    cancellation: &Cancellation,
    starting: &Mutex<Option<tokio::process::Child>>,
) -> Result<(tokio::process::Child, Route), String> {
    let (model_path, _) = managed_models::path_for_catalog(app, &entry.id, Some(&artifact.id))?;
    // This launcher serves text-mode chat; a Juniper 1 GGUF needs the token-ID backend (A-02).
    let mut model_file = std::fs::File::open(&model_path)
        .map_err(|_| "The managed model file is unavailable.".to_owned())?;
    crate::gguf_guard::refuse_juniper_on_text_path(&mut model_file)?;
    drop(model_file);
    let executable = match &profile {
        Some(_) => accelerated_executable(app)?,
        None => runtime_executable(app)?,
    };
    let runtime_build = runtime_build(&executable).await;
    if let Some(profile) = &profile
        && !runtime_build
            .as_deref()
            .is_some_and(|build| build_matches(build, &profile.runtime.commit))
    {
        return Err(format!(
            "RUNTIME_IDENTITY_MISMATCH: This model is qualified only on llama.cpp {} ({}). The available runtime is {}.",
            profile.runtime.tag,
            &profile.runtime.commit[..9],
            runtime_build.as_deref().unwrap_or("unidentified")
        ));
    }
    if let Some(profile) = &profile
        && profile.runtime.accelerator == "cuda"
        && !runtime_has_cuda_device(&executable).await
    {
        return Err(
            "LOCAL_RUNTIME_GPU_UNAVAILABLE: The selected llama.cpp build does not report a usable CUDA device.".into(),
        );
    }
    let runtime_dir = runtime_directory(app)?;
    let key = KeyFile::create(&runtime_dir)?;
    let port = reserve_port()?;
    let mut args: Vec<String> = vec![
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "--model".into(),
        model_path.to_string_lossy().into_owned(),
        "--alias".into(),
        entry.id.clone(),
        "--api-key-file".into(),
        key.path.to_string_lossy().into_owned(),
        "--no-webui".into(),
        "--no-slots".into(),
        "--offline".into(),
    ];
    let mut command = tokio::process::Command::new(&executable);
    match &profile {
        Some(profile) => {
            let template = runtime_dir.join(&profile.template.file);
            std::fs::write(&template, backend::GPT_OSS_TEMPLATE).map_err(|_| {
                "LOCAL_RUNTIME_FAILED: Juniper could not stage the model's chat template."
                    .to_owned()
            })?;
            args.extend(profile.server_args());
            args.extend([
                "--chat-template-file".into(),
                template.to_string_lossy().into_owned(),
            ]);
            if profile.runtime.accelerator == "cuda" {
                command.env("CUDA_VISIBLE_DEVICES", "0");
            }
        }
        None => args.extend([
            "--cache-ram".into(),
            GENERIC_CACHE_RAM_MIB.into(),
            "--ctx-size".into(),
            generic_context(entry).to_string(),
        ]),
    }
    let (child_pid, diagnostics) = spawn(command, &args, starting)?;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let owner_pid = child_pid;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let pid = owner_pid;
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let _ = child_pid;
    let route = Route {
        endpoint: format!("http://127.0.0.1:{port}"),
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        port,
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        owner_pid,
        api_key: key.secret.clone(),
        identity: RuntimeIdentity {
            artifact_id: artifact.id.clone(),
            artifact_sha256: artifact.sha256.clone().unwrap_or_default(),
            model_repository: profile
                .as_ref()
                .map(|profile| profile.model.repository.clone()),
            model_revision: profile
                .as_ref()
                .map(|profile| profile.model.revision.clone()),
            runtime_build,
            template_sha256: profile
                .as_ref()
                .map(|profile| profile.template.sha256.clone()),
            context_size: Some(
                profile
                    .as_ref()
                    .map_or_else(|| generic_context(entry), |profile| profile.server.ctx_size),
            ),
            tokenizer_sha256: None,
            protocol: None,
        },
        token_ids: None,
        lineage: match &profile {
            Some(profile) => format!(
                "{} by {} ({} license), unmodified weights ({} quantization) served on this device by {} {}",
                profile.model.name,
                profile.model.developer,
                profile.model.license,
                profile.artifact.quantization,
                profile.runtime.engine,
                profile.runtime.tag
            ),
            None => format!("{}. {}", entry.display_name, entry.attribution),
        },
        profile,
    };
    let startup_timeout = route
        .profile
        .as_ref()
        .map_or(GENERIC_STARTUP_TIMEOUT, |profile| {
            Duration::from_secs(profile.lifecycle.startup_timeout_seconds)
        });
    let model_path_text = model_path.to_string_lossy().into_owned();
    let ready = async {
        // llama-server listens before it loads the model, and only after
        // parsing its arguments, so a listener that is provably the child's
        // means the key file has been read.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            wait_until(starting, startup_timeout, cancellation, || async {
                pid.is_some_and(|pid| listener_belongs_to(pid, port))
            })
            .await?;
            drop(key);
        }
        wait_until(starting, startup_timeout, cancellation, || healthy(&route)).await?;
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        drop(key);
        let props = fetch_props(&route).await?;
        check_ownership(&props, &model_path_text)?;
        if let Some(profile) = &route.profile {
            check_props(&props, profile)?;
            if profile.lifecycle.warm_up {
                emit_activity(app, &request.request_id, "warming-up");
                warm_up(&route, cancellation).await?;
            }
        }
        Ok::<(), String>(())
    }
    .await;
    let child = starting.lock().ok().and_then(|mut slot| slot.take());
    match (ready, child) {
        (Ok(()), Some(child)) => Ok((child, route)),
        (Ok(()), None) => {
            Err("LOCAL_RUNTIME_FAILED: The local runtime was stopped while it was starting.".into())
        }
        (Err(error), child) => {
            if let Some(mut child) = child {
                let _ = child.kill().await;
            }
            Err(classify_failure(error, &diagnostics))
        }
    }
}

/// Spawns the server and parks it in `starting` so exit cleanup can reach it
/// while it loads. Returns its PID and the stderr tail used to classify a
/// failed start.
fn spawn(
    mut command: tokio::process::Command,
    args: &[String],
    starting: &Mutex<Option<tokio::process::Child>>,
) -> Result<(Option<u32>, StderrTail), String> {
    // If Juniper itself is killed, no exit handler runs; the kernel stops the
    // server instead of leaving a multi-gigabyte process behind.
    #[cfg(target_os = "linux")]
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = crate::child::spawn_owned(&mut command).map_err(|_| {
        "LOCAL_RUNTIME_UNAVAILABLE: Juniper's local runtime could not be started.".to_owned()
    })?;
    let diagnostics = capture_stderr(&mut child);
    let pid = child.id();
    *starting.lock().map_err(|_| state_error())? = Some(child);
    Ok((pid, diagnostics))
}

/// Starts a Juniper 1 server for the token-ID backend. Model file, GGUF
/// metadata, tokenizer, and runtime build are verified before launch; the
/// served build, model path, and context, and the token-ID self-test, after.
async fn start_token_ids<R: Runtime>(
    app: &AppHandle<R>,
    artifact_id: &str,
    cancellation: &Cancellation,
    starting: &Mutex<Option<tokio::process::Child>>,
) -> Result<(tokio::process::Child, Route), String> {
    ensure_token_id_platform()?;
    let verified =
        juniper1::verify_installed(&managed_models::models_directory(app)?, artifact_id)?;
    let profile = &verified.profile;
    let executable = token_id_executable(app)?;
    let runtime_build = runtime_build(&executable).await;
    if !runtime_build
        .as_deref()
        .is_some_and(|build| build_matches(build, &profile.runtime.commit))
    {
        return Err(format!(
            "RUNTIME_IDENTITY_MISMATCH: Juniper 1 is qualified only on llama.cpp {} ({}). The available runtime is {}.",
            profile.runtime.tag,
            &profile.runtime.commit[..9],
            runtime_build.as_deref().unwrap_or("unidentified")
        ));
    }
    let runtime_dir = runtime_directory(app)?;
    let key = KeyFile::create(&runtime_dir)?;
    let port = reserve_port()?;
    let model_path_text = verified.model_path.to_string_lossy().into_owned();
    let server = &profile.server;
    let args: Vec<String> = [
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "--model",
        &model_path_text,
        "--alias",
        &verified.artifact.id,
        "--api-key-file",
        &key.path.to_string_lossy(),
        "--no-webui",
        "--no-slots",
        "--offline",
        "--no-mmproj",
        "--no-context-shift",
        "--fit",
        "off",
        "-c",
        &server.ctx_size.to_string(),
        "-np",
        &server.parallel.to_string(),
        "-ngl",
        &server.n_gpu_layers.to_string(),
        "-b",
        &server.batch_size.to_string(),
        "--cache-ram",
        &server.cache_ram_mib.to_string(),
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut command = tokio::process::Command::new(&executable);
    if profile.runtime.accelerator == "cpu" {
        // The qualified configuration is CPU-only; a CUDA build must not
        // claim a GPU it will not use.
        command.env("CUDA_VISIBLE_DEVICES", "");
    }
    let (child_pid, diagnostics) = spawn(command, &args, starting)?;
    let endpoint = format!("http://127.0.0.1:{port}");
    let client = juniper1::client::TokenIdClient::new(
        &endpoint,
        verified.tokenizer.vocab_size(),
        Some(key.secret.clone()),
        Duration::from_secs(profile.lifecycle.request_timeout_seconds),
        child_pid.map(|pid| (pid, port)),
    )?;
    let identity = RuntimeIdentity {
        artifact_id: verified.artifact.id.clone(),
        artifact_sha256: verified.artifact.sha256.clone(),
        model_repository: None,
        model_revision: Some(verified.artifact.source_checkpoint.clone()),
        runtime_build,
        template_sha256: None,
        context_size: Some(server.ctx_size),
        tokenizer_sha256: Some(verified.tokenizer.sha256().to_owned()),
        protocol: Some(profile.protocol.clone()),
    };
    let lineage = verified.artifact.lineage(profile);
    let runtime = Arc::new(TokenIdRuntime {
        profile: verified.profile.clone(),
        artifact: verified.artifact.clone(),
        tokenizer: verified.tokenizer.clone(),
        client,
    });
    let route = Route {
        endpoint,
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        port,
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        owner_pid: child_pid,
        api_key: key.secret.clone(),
        identity,
        profile: None,
        token_ids: Some(runtime.clone()),
        lineage,
    };
    let startup_timeout = Duration::from_secs(profile.lifecycle.startup_timeout_seconds);
    // Bound the complete startup, including props and every self-test request;
    // separate health polling limits cannot bound a stalled self-test.
    let ready = tokio::select! {
        result = timeout(startup_timeout, async {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            wait_until(starting, startup_timeout, cancellation, || async {
                child_pid.is_some_and(|pid| listener_belongs_to(pid, port))
            })
            .await?;
            drop(key);
        }
        wait_until(starting, startup_timeout, cancellation, || healthy(&route)).await?;
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        drop(key);
        let props = fetch_props(&route).await?;
        check_ownership(&props, &model_path_text)?;
        check_token_id_props(&props, profile)?;
        runtime
            .client
            .self_test(&runtime.tokenizer, cancellation)
            .await?;
        Ok::<(), String>(())
        }) => result.unwrap_or_else(|_| Err("LOCAL_RUNTIME_TIMEOUT: The local model took too long to start.".into())),
        _ = cancellation.wait() => Err("REQUEST_CANCELLED: Generation cancelled.".into()),
    };
    let child = starting.lock().ok().and_then(|mut slot| slot.take());
    match (ready, child) {
        (Ok(()), Some(child)) => Ok((child, route)),
        (Ok(()), None) => {
            Err("LOCAL_RUNTIME_FAILED: The local runtime was stopped while it was starting.".into())
        }
        (Err(error), child) => {
            if let Some(mut child) = child {
                let _ = child.kill().await;
            }
            Err(classify_failure(error, &diagnostics))
        }
    }
}

/// Native Juniper 1 activation requires proof of listener ownership before
/// credentials are sent. Other desktop platforms need their own ownership
/// implementation and live qualification before this route can activate.
fn ensure_token_id_platform() -> Result<(), String> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        Err("JUNIPER1_PLATFORM_NOT_QUALIFIED: Native Juniper 1 activation is qualified only on Linux with verified runtime listener ownership.".into())
    }
}

/// The served build and context must be the qualified ones.
fn check_token_id_props(
    props: &Value,
    profile: &juniper1::profile::TokenIdProfile,
) -> Result<(), String> {
    if !build_matches(
        props["build_info"].as_str().unwrap_or_default(),
        &profile.runtime.commit,
    ) {
        return Err(
            "RUNTIME_IDENTITY_MISMATCH: The loaded runtime is not the qualified build.".into(),
        );
    }
    if props["default_generation_settings"]["n_ctx"].as_u64()
        != Some(u64::from(profile.server.ctx_size))
    {
        return Err(
            "RUNTIME_IDENTITY_MISMATCH: The loaded context size is not the qualified one.".into(),
        );
    }
    Ok(())
}

/// The loopback API key, written owner-only and removed once the server has
/// read it, so it never appears in a process listing.
struct KeyFile {
    path: PathBuf,
    secret: String,
}

impl KeyFile {
    fn create(directory: &Path) -> Result<Self, String> {
        use std::io::Write;
        let secret = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let path = directory.join(format!("server-{}.key", uuid::Uuid::new_v4().simple()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(&path)
            .and_then(|mut file| file.write_all(secret.as_bytes()).map(|_| file))
            .map_err(|_| {
                "LOCAL_RUNTIME_FAILED: Juniper could not protect the local runtime.".to_owned()
            })?;
        let _ = file.flush();
        Ok(Self { path, secret })
    }
}

impl Drop for KeyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn runtime_directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| {
            "LOCAL_RUNTIME_FAILED: Juniper could not locate its data directory.".to_owned()
        })?
        .join("runtime");
    std::fs::create_dir_all(&directory).map_err(|_| {
        "LOCAL_RUNTIME_FAILED: Juniper could not prepare its runtime directory.".to_owned()
    })?;
    Ok(directory)
}

/// Keeps the last few stderr lines in memory to classify a failed start.
/// They are never logged or shown: llama-server output can describe the model
/// path and hardware, and in verbose builds, prompt text.
type StderrTail = Arc<Mutex<VecDeque<String>>>;

fn capture_stderr(child: &mut tokio::process::Child) -> StderrTail {
    let tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
    if let Some(stderr) = child.stderr.take() {
        let sink = tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut tail) = sink.lock() {
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line.chars().take(STDERR_LINE_CHARS).collect());
                }
            }
        });
    }
    tail
}

fn classify_failure(error: String, diagnostics: &Mutex<VecDeque<String>>) -> String {
    if !error.starts_with("LOCAL_RUNTIME_FAILED") && !error.starts_with("LOCAL_RUNTIME_TIMEOUT") {
        return error;
    }
    let lines = diagnostics
        .lock()
        .map(|lines| {
            lines
                .iter()
                .map(|line| line.to_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mentions = |needles: &[&str]| {
        lines
            .iter()
            .any(|line| needles.iter().any(|needle| line.contains(needle)))
    };
    if mentions(&[
        "out of memory",
        "cudamalloc failed",
        "failed to allocate",
        "unable to allocate",
    ]) {
        return "LOCAL_RUNTIME_OUT_OF_MEMORY: The model does not fit in the available GPU or system memory with its qualified settings. Close other memory- or GPU-heavy applications and try again.".into();
    }
    if mentions(&[
        "no cuda-capable device",
        "cuda error",
        "ggml_cuda_init: failed",
    ]) {
        return "LOCAL_RUNTIME_GPU_UNAVAILABLE: The model's qualified runtime needs an NVIDIA GPU that Juniper could not use.".into();
    }
    error
}

fn build_matches(build: &str, commit: &str) -> bool {
    build
        .rsplit('-')
        .next()
        .is_some_and(|short| short.len() >= 7 && commit.starts_with(short))
}

/// `b<build>-<commit>` from `llama-server --version`, which prints
/// `version: 0.5.0-dev (build 11270, commit 748d4225b)`.
async fn runtime_build(executable: &Path) -> Option<String> {
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = crate::child::spawn_owned(&mut command).ok()?;
    let output = timeout(PROBE_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_version(&text)
}

async fn runtime_has_cuda_device(executable: &Path) -> bool {
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("--list-devices")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let Ok(child) = crate::child::spawn_owned(&mut command) else {
        return false;
    };
    let Ok(output) = timeout(PROBE_TIMEOUT, child.wait_with_output()).await else {
        return false;
    };
    let Ok(output) = output else { return false };
    if !output.status.success() {
        return false;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    has_cuda_device(&text)
}

fn has_cuda_device(text: &str) -> bool {
    text.lines().any(|line| {
        line.trim_start()
            .strip_prefix("CUDA")
            .is_some_and(|tail| tail.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
    })
}

fn parse_version(text: &str) -> Option<String> {
    let build = text.split("build ").nth(1)?.split(',').next()?.trim();
    let commit = text.split("commit ").nth(1)?.split(')').next()?.trim();
    (build.chars().all(|character| character.is_ascii_digit())
        && commit
            .chars()
            .all(|character| character.is_ascii_hexdigit())
        && !build.is_empty()
        && commit.len() >= 7)
        .then(|| format!("b{build}-{commit}"))
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(PROBE_TIMEOUT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "LOCAL_RUNTIME_FAILED: The runtime client could not be initialized.".into())
}

async fn healthy(route: &Route) -> bool {
    let Ok(client) = client() else { return false };
    timeout(
        Duration::from_secs(2),
        client.get(format!("{}/health", route.endpoint)).send(),
    )
    .await
    .is_ok_and(|response| response.is_ok_and(|response| response.status().is_success()))
}

/// Polls `check` while the starting server is alive, within `limit`.
async fn wait_until<F, Fut>(
    starting: &Mutex<Option<tokio::process::Child>>,
    limit: Duration,
    cancellation: &Cancellation,
    mut check: F,
) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let started = Instant::now();
    loop {
        if cancellation.is_cancelled() {
            return Err("REQUEST_CANCELLED: Generation cancelled.".into());
        }
        let running = starting
            .lock()
            .ok()
            .and_then(|mut slot| {
                slot.as_mut()
                    .map(|child| matches!(child.try_wait(), Ok(None)))
            })
            .unwrap_or(false);
        if !running {
            return Err(
                "LOCAL_RUNTIME_FAILED: Juniper's local runtime stopped while loading the model."
                    .into(),
            );
        }
        if check().await {
            return Ok(());
        }
        if started.elapsed() >= limit {
            return Err("LOCAL_RUNTIME_TIMEOUT: The local model took too long to start.".into());
        }
        tokio::select! {
            _ = sleep(HEALTH_INTERVAL) => {},
            _ = cancellation.wait() => return Err("REQUEST_CANCELLED: Generation cancelled.".into()),
        }
    }
}

/// Whether the loopback listener on `port` is a socket held by process `pid`.
/// The port was free when Juniper chose it, but any local process, including
/// another user's, could bind it first; the key is not sent until this holds.
#[cfg(target_os = "linux")]
pub(crate) fn listener_belongs_to(pid: u32, port: u16) -> bool {
    // /proc/net/tcp prints the address as the raw in-memory u32.
    let local = format!(
        "{:08X}:{port:04X}",
        u32::from_ne_bytes(std::net::Ipv4Addr::LOCALHOST.octets())
    );
    let Ok(table) = std::fs::read_to_string("/proc/net/tcp") else {
        return false;
    };
    let sockets: Vec<String> = table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // State 0A is LISTEN; field 9 is the socket inode.
            (fields.get(1) == Some(&local.as_str()) && fields.get(3) == Some(&"0A"))
                .then(|| fields.get(9).map(|inode| format!("socket:[{inode}]")))
                .flatten()
        })
        .collect();
    if sockets.is_empty() {
        return false;
    }
    let Ok(descriptors) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return false;
    };
    descriptors.flatten().any(|entry| {
        std::fs::read_link(entry.path()).is_ok_and(|target| {
            sockets
                .iter()
                .any(|socket| target.as_os_str() == socket.as_str())
        })
    })
}

/// Match an exact IPv4 loopback listener to its owning process before
/// disclosing the runtime's per-launch key or any conversation content.
#[cfg(target_os = "windows")]
pub(crate) fn listener_belongs_to(pid: u32, port: u16) -> bool {
    use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP_STATE_LISTEN, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;
    let mut bytes = 0u32;
    // DWORD storage provides the alignment required by the native table.
    // A changing socket table can require a larger buffer on the second call.
    let first = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut bytes,
            0,
            u32::from(AF_INET),
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if first != ERROR_INSUFFICIENT_BUFFER {
        return false;
    }
    for _ in 0..3 {
        if !(4..=16 * 1024 * 1024).contains(&bytes) {
            return false;
        }
        let mut table = vec![0u32; (bytes as usize).div_ceil(4)];
        let result = unsafe {
            GetExtendedTcpTable(
                table.as_mut_ptr().cast(),
                &mut bytes,
                0,
                u32::from(AF_INET),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if result == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if result != 0 {
            return false;
        }
        let count = table[0] as usize;
        let offset = std::mem::offset_of!(MIB_TCPTABLE_OWNER_PID, table);
        let row_size = std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
        if offset + count.saturating_mul(row_size) > table.len() * 4 {
            return false;
        }
        for index in 0..count {
            // Bounds and DWORD alignment are checked above; copy each row so
            // no native pointer or borrowed view outlives the table storage.
            let row = unsafe {
                std::ptr::read_unaligned(
                    table
                        .as_ptr()
                        .cast::<u8>()
                        .add(offset + index * row_size)
                        .cast::<MIB_TCPROW_OWNER_PID>(),
                )
            };
            if row.dwState == MIB_TCP_STATE_LISTEN as u32
                && row.dwLocalAddr == u32::from_ne_bytes(std::net::Ipv4Addr::LOCALHOST.octets())
                && u16::from_be(row.dwLocalPort as u16) == port
                && row.dwOwningPid == pid
            {
                return true;
            }
        }
        return false;
    }
    false
}

/// `/props` requires the per-launch key, so only the server Juniper started
/// (or something that already holds the key) can answer it.
async fn fetch_props(route: &Route) -> Result<Value, String> {
    let unidentified =
        || "RUNTIME_IDENTITY_MISMATCH: The local runtime did not report its identity.".to_owned();
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    ensure_route_listener_owner(route)?;
    let response = client()?
        .get(format!("{}/props", route.endpoint))
        .bearer_auth(&route.api_key)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|_| unidentified())?;
    if !response.status().is_success() {
        return Err(unidentified());
    }
    response.json().await.map_err(|_| unidentified())
}

/// The server that answers must be serving the file Juniper asked it to load.
/// On Linux and Windows `listener_belongs_to` has already proven the listener
/// belongs to the child before its key is disclosed. Other desktop platforms
/// do not yet provide this listener-ownership qualification.
fn check_ownership(props: &Value, model_path: &str) -> Result<(), String> {
    if props["model_path"].as_str() == Some(model_path) {
        Ok(())
    } else {
        Err(
            "RUNTIME_IDENTITY_MISMATCH: Another process answered on Juniper's local runtime port."
                .into(),
        )
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn ensure_route_listener_owner(route: &Route) -> Result<(), String> {
    if !route
        .owner_pid
        .is_some_and(|pid| listener_belongs_to(pid, route.port))
    {
        return Err(
            "LOCAL_RUNTIME_UNAVAILABLE: The resident server no longer owns its local port.".into(),
        );
    }
    Ok(())
}

/// Confirms the loaded server is the qualified one: build, served template
/// (as llama.cpp rewrote it), and context size.
fn check_props(props: &Value, profile: &BackendProfile) -> Result<(), String> {
    let build = props["build_info"].as_str().unwrap_or_default();
    if !build_matches(build, &profile.runtime.commit) {
        return Err(
            "RUNTIME_IDENTITY_MISMATCH: The loaded runtime is not the qualified build.".into(),
        );
    }
    let template = props["chat_template"].as_str().unwrap_or_default();
    if backend::sha256_hex(template.as_bytes()) != profile.template.served_sha256 {
        return Err("TEMPLATE_IDENTITY_MISMATCH: The loaded chat template is not the qualified Harmony template.".into());
    }
    if props["default_generation_settings"]["n_ctx"].as_u64()
        != Some(u64::from(profile.server.ctx_size))
    {
        return Err(
            "RUNTIME_IDENTITY_MISMATCH: The loaded context size is not the qualified one.".into(),
        );
    }
    Ok(())
}

/// One throwaway request so the first real answer does not pay for faulting
/// CPU-side experts into memory.
async fn warm_up(route: &Route, cancellation: &Cancellation) -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    ensure_route_listener_owner(route)?;
    let call = client()?
        .post(format!("{}/v1/chat/completions", route.endpoint))
        .bearer_auth(&route.api_key)
        .timeout(WARM_UP_TIMEOUT)
        .json(&json!({
            "messages": [{ "role": "user", "content": "Hello." }],
            "max_tokens": 16,
            "reasoning_effort": "low",
            "stream": false
        }))
        .send();
    let response = tokio::select! {
        response = call => response.map_err(|_| "LOCAL_RUNTIME_FAILED: The model did not answer its warm-up request.".to_owned())?,
        _ = cancellation.wait() => return Err("REQUEST_CANCELLED: Generation cancelled.".into()),
    };
    if !response.status().is_success() {
        return Err("LOCAL_RUNTIME_FAILED: The model did not answer its warm-up request.".into());
    }
    Ok(())
}

fn emit_activity<R: Runtime>(app: &AppHandle<R>, request_id: &str, activity: &str) {
    let mut event = ChatStreamEvent::for_request(request_id);
    event.activity = Some(activity.into());
    let _ = app.emit(&format!("juniper://chat/{request_id}"), event);
}

pub fn emit_error<R: Runtime>(app: &AppHandle<R>, request_id: &str, error: &str) {
    let mut pieces = error.splitn(2, ':');
    let code = pieces.next().unwrap_or("LOCAL_RUNTIME_FAILED");
    let message = pieces.next().unwrap_or(error).trim();
    let mut event = ChatStreamEvent::for_request(request_id);
    event.done = Some(true);
    event.error = Some(RuntimeError {
        code: code.into(),
        message: message.into(),
    });
    let _ = app.emit(&format!("juniper://chat/{request_id}"), event);
}

fn executable_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_owned()
    }
}

fn resolve_executable<R: Runtime>(
    app: &AppHandle<R>,
    override_variable: &str,
    name: &str,
) -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var(override_variable) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
    }
    let resource = app.path().resource_dir().map_err(|_| {
        "LOCAL_RUNTIME_UNAVAILABLE: Juniper could not locate its runtime resources.".to_owned()
    })?;
    [
        resource.join("runtime").join(executable_name(name)),
        resource.join("binaries").join(executable_name(name)),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or_else(|| {
        "LOCAL_RUNTIME_UNAVAILABLE: Juniper's bundled local runtime is not available on this build."
            .into()
    })
}

pub(crate) fn runtime_executable<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    resolve_executable(app, "JUNIPER_LLAMA_SERVER", "llama-server")
}

/// The llama.cpp build Juniper 1 is qualified on (b11270). A dedicated
/// override comes first; otherwise the bundled CUDA runtime, which is that
/// build. Either way its reported version must match before it starts.
fn token_id_executable<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("JUNIPER_LLAMA_SERVER_JUNIPER1") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
    }
    accelerated_executable(app).map_err(|_| {
        "LOCAL_RUNTIME_UNAVAILABLE: Juniper 1 needs llama.cpp b11270, which is not installed in this build.".into()
    })
}

/// The CUDA build qualified models need. It is never substituted with the
/// CPU runtime: a different build is a different, unqualified configuration.
fn accelerated_executable<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    resolve_executable(app, "JUNIPER_LLAMA_SERVER_CUDA", "llama-server-cuda").map_err(|_| {
        "LOCAL_RUNTIME_UNAVAILABLE: This model needs Juniper's CUDA runtime, which is not installed in this build.".into()
    })
}

fn reserve_port() -> Result<u16, String> {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| {
            "LOCAL_RUNTIME_PORT_ERROR: Juniper could not reserve a private local port.".to_owned()
        })
        .and_then(|listener| {
            listener
                .local_addr()
                .map(|address| address.port())
                .map_err(|_| {
                    "LOCAL_RUNTIME_PORT_ERROR: Juniper could not inspect the private local port."
                        .into()
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_activation_requires_a_qualified_listener_ownership_platform() {
        assert_eq!(
            ensure_token_id_platform().is_ok(),
            cfg!(target_os = "linux")
        );
    }

    #[test]
    fn runtime_identity_redirects_are_not_followed() {
        use std::io::{Read, Write};
        let listener =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("listener");
        let port = listener.local_addr().expect("address").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let captured = requests.clone();
        let serving = std::thread::spawn(move || {
            listener.set_nonblocking(true).expect("nonblocking");
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Winsock streams accepted from a non-blocking
                        // listener are non-blocking too; the read below
                        // relies on the timeout instead.
                        stream.set_nonblocking(false).expect("blocking stream");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .expect("read timeout");
                        let mut request = [0u8; 4096];
                        let count = stream.read(&mut request).expect("request");
                        captured.fetch_add(1, Ordering::AcqRel);
                        assert!(
                            std::str::from_utf8(&request[..count])
                                .expect("HTTP")
                                .starts_with("GET /props ")
                        );
                        // Same origin: reqwest would retain Authorization if it
                        // followed this redirect. Never send it to another path.
                        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /unexpected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").expect("reply");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            }
        });
        let route = Route {
            endpoint: format!("http://127.0.0.1:{port}"),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            port,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            owner_pid: Some(std::process::id()),
            api_key: "synthetic-test-key".into(),
            identity: RuntimeIdentity::default(),
            profile: None,
            token_ids: None,
            lineage: String::new(),
        };
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        assert!(
            runtime
                .block_on(fetch_props(&route))
                .unwrap_err()
                .starts_with("RUNTIME_IDENTITY_MISMATCH")
        );
        serving.join().expect("server");
        assert_eq!(requests.load(Ordering::Acquire), 1);
    }

    fn profile() -> BackendProfile {
        backend::profile("gpt-oss-20b-mxfp4-flowbox.v1").expect("bundled profile")
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn only_the_process_holding_the_listener_owns_the_port() {
        let listener =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("listener");
        let port = listener.local_addr().expect("address").port();
        assert!(listener_belongs_to(std::process::id(), port));
        // PID 1 holds no socket of this test's.
        assert!(!listener_belongs_to(1, port));
        drop(listener);
        assert!(!listener_belongs_to(std::process::id(), port));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn a_port_squatter_never_receives_runtime_credentials_or_warm_up() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let route = Route {
            endpoint: format!("http://127.0.0.1:{port}"),
            port,
            owner_pid: Some(0),
            api_key: "must-not-be-sent".into(),
            identity: RuntimeIdentity::default(),
            profile: None,
            token_ids: None,
            lineage: String::new(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            assert!(
                fetch_props(&route)
                    .await
                    .unwrap_err()
                    .starts_with("LOCAL_RUNTIME_UNAVAILABLE")
            );
            assert!(
                warm_up(&route, &Cancellation::default())
                    .await
                    .unwrap_err()
                    .starts_with("LOCAL_RUNTIME_UNAVAILABLE")
            );
        });
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "no authenticated connection may reach a listener owned by another process"
        );
    }

    #[test]
    fn version_output_is_parsed_into_a_build_identity() {
        assert_eq!(
            parse_version(
                "version: 0.5.0-dev (build 11270, commit 748d4225b)\nbuilt with GNU 13.3.0"
            ),
            Some("b11270-748d4225b".into())
        );
        assert_eq!(parse_version("llama-server dev"), None);
        assert_eq!(parse_version("version: x (build 1, commit zz)"), None);
    }

    #[test]
    fn only_the_qualified_runtime_build_matches() {
        let commit = &profile().runtime.commit;
        assert!(build_matches("b11270-748d4225b", commit));
        assert!(!build_matches("b10900-e107984bc", commit));
        assert!(!build_matches("b11270-748d", commit));
        assert!(!build_matches("", commit));
    }

    #[test]
    fn a_cuda_profile_requires_a_reported_cuda_device() {
        assert!(has_cuda_device(
            "Available devices:\n  CUDA0: NVIDIA GeForce RTX 2060 (5737 MiB)\n"
        ));
        assert!(!has_cuda_device("Available devices:\n  CPU: AMD Ryzen\n"));
        assert!(!has_cuda_device("Available devices:\n  CUDA: no device\n"));
    }

    #[test]
    fn served_identity_must_match_build_template_and_context() {
        let profile = profile();
        let served = |template: &str, build: &str, context: u64| {
            json!({
                "build_info": build,
                "chat_template": template,
                "default_generation_settings": { "n_ctx": context }
            })
        };
        // The served template is llama.cpp's rewrite of the bundled file, so
        // the bundled text itself must not match the served hash.
        assert!(
            check_props(
                &served(backend::GPT_OSS_TEMPLATE, "b11270-748d4225b", 16_384),
                &profile
            )
            .is_err()
        );
        assert!(
            check_props(&served("x", "b11270-748d4225b", 16_384), &profile)
                .unwrap_err()
                .starts_with("TEMPLATE_IDENTITY_MISMATCH")
        );
        assert!(
            check_props(&served("x", "b10900-e107984bc", 16_384), &profile)
                .unwrap_err()
                .starts_with("RUNTIME_IDENTITY_MISMATCH")
        );
    }

    #[test]
    fn only_the_server_juniper_started_is_trusted() {
        let props = json!({ "model_path": "/data/models/a.gguf" });
        assert!(check_ownership(&props, "/data/models/a.gguf").is_ok());
        assert!(
            check_ownership(&props, "/data/models/b.gguf")
                .unwrap_err()
                .starts_with("RUNTIME_IDENTITY_MISMATCH")
        );
        assert!(check_ownership(&json!({}), "/data/models/a.gguf").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn exit_cleanup_reaches_a_server_that_is_still_loading() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime should start");
        runtime.block_on(async {
            let local = LocalRuntime::default();
            let child = tokio::process::Command::new("sleep")
                .arg("60")
                .kill_on_drop(true)
                .spawn()
                .expect("fixture child should start");
            let pid = child.id().expect("pid");
            *local.starting.lock().expect("lock") = Some(child);
            assert_eq!(local.terminate_all(), 1);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
                && std::time::Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(unsafe { libc::kill(pid as libc::pid_t, 0) != 0 });
        });
    }

    #[test]
    fn a_profile_is_never_applied_to_a_different_artifact() {
        let entry = catalog::find("gpt-oss-20b").expect("catalog entry");
        let mut artifact = entry.artifacts[0].clone();
        assert!(qualified_profile(&artifact).expect("profile").is_some());
        artifact.sha256 = Some("0".repeat(64));
        assert!(
            qualified_profile(&artifact)
                .unwrap_err()
                .starts_with("BACKEND_PROFILE_MISMATCH")
        );
        artifact.backend_profile = None;
        assert!(qualified_profile(&artifact).expect("generic").is_none());
    }

    #[test]
    fn startup_failures_are_classified_without_exposing_output() {
        let lines = Mutex::new(VecDeque::from([
            "load_tensors: offloading 24 layers".to_owned(),
            "ggml_backend_cuda_buffer_type_alloc_buffer: allocating 4.1 GiB on device 0: cudaMalloc failed: out of memory".to_owned(),
        ]));
        let error = classify_failure(
            "LOCAL_RUNTIME_FAILED: Juniper's local runtime stopped while loading the model.".into(),
            &lines,
        );
        assert!(error.starts_with("LOCAL_RUNTIME_OUT_OF_MEMORY"));
        assert!(!error.contains("cudaMalloc"));
        let cancelled = classify_failure("REQUEST_CANCELLED: Generation cancelled.".into(), &lines);
        assert!(cancelled.starts_with("REQUEST_CANCELLED"));
    }

    #[test]
    fn the_key_file_is_owner_only_and_removed() {
        let directory = std::env::temp_dir().join(format!("juniper-key-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).expect("directory");
        let key = KeyFile::create(&directory).expect("key");
        assert_eq!(
            std::fs::read_to_string(&key.path).expect("read"),
            key.secret
        );
        assert_eq!(key.secret.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key.path)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let path = key.path.clone();
        drop(key);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn exit_cleanup_stops_the_resident_server() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime should start");
        runtime.block_on(async {
            let local = LocalRuntime::default();
            let child = tokio::process::Command::new("sleep")
                .arg("60")
                .kill_on_drop(true)
                .spawn()
                .expect("fixture child should start");
            let pid = child.id().expect("running child should have a pid");
            *local.resident.lock().expect("lock") = Some(Resident {
                artifact_id: "fixture".into(),
                child,
                route: Route {
                    endpoint: "http://127.0.0.1:9".into(),
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    port: 9,
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    owner_pid: Some(pid),
                    api_key: "key".into(),
                    identity: RuntimeIdentity::default(),
                    profile: None,
                    token_ids: None,
                    lineage: String::new(),
                },
                leases: Arc::new(AtomicUsize::new(0)),
            });
            assert_eq!(local.status().state, "ready");
            assert_eq!(local.terminate_all(), 1);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
                && std::time::Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(unsafe { libc::kill(pid as libc::pid_t, 0) != 0 });
            assert_eq!(local.terminate_all(), 0);
            assert_eq!(local.status().state, "idle");
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_busy_server_is_not_unloaded() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime should start");
        runtime.block_on(async {
            let local = LocalRuntime::default();
            let child = tokio::process::Command::new("sleep")
                .arg("60")
                .kill_on_drop(true)
                .spawn()
                .expect("fixture child should start");
            let leases = Arc::new(AtomicUsize::new(1));
            *local.resident.lock().expect("lock") = Some(Resident {
                artifact_id: "fixture".into(),
                child,
                route: Route {
                    endpoint: "http://127.0.0.1:9".into(),
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    port: 9,
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    owner_pid: None,
                    api_key: "key".into(),
                    identity: RuntimeIdentity::default(),
                    profile: None,
                    token_ids: None,
                    lineage: String::new(),
                },
                leases: leases.clone(),
            });
            assert_eq!(local.status().state, "busy");
            assert!(
                local
                    .unload()
                    .await
                    .unwrap_err()
                    .starts_with("LOCAL_RUNTIME_BUSY")
            );
            drop(Lease(leases));
            assert!(local.unload().await.expect("unload"));
            assert_eq!(local.status().state, "idle");
        });
    }
}
