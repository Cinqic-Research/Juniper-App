use crate::catalog::{self, CatalogArtifact, CatalogArtifactFile, CatalogEntry};
use crate::commands::{AppState, Cancellation};
use crate::device;
use futures_util::StreamExt;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, HeaderValue, RANGE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::time::Duration;

/// Free space kept beyond a download so the system is never filled.
const STORAGE_HEADROOM_BYTES: u64 = 1024 * 1024 * 1024;
/// A stalled transfer fails after this long without a byte; there is no
/// limit on total time, so large files on slow links still complete.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Downloads that may transfer at once; more wait in line.
const PARALLEL_DOWNLOADS: usize = 2;
const JOURNAL_VERSION: u32 = 1;
const READY_MARKER: &str = ".juniper-ready.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedModel {
    pub catalog_id: String,
    pub artifact_id: String,
    /// Compatibility field retained in the serialized API for pre-v2 clients.
    pub variant_id: String,
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub verified: bool,
    /// `ready`, `partial` (resumable), or `corrupt`.
    pub state: String,
    /// Bytes already on disk for a partial download.
    pub completed_bytes: u64,
    pub file_count: usize,
}

pub fn models_directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("models");
    fs::create_dir_all(&directory).map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
    Ok(directory)
}

// ------------------------------------------------------------------ locks

fn operations() -> &'static Mutex<HashSet<String>> {
    static OPERATIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    OPERATIONS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Exclusive use of one artifact's files: a download, import, and removal
/// of the same artifact can never interleave.
#[derive(Debug)]
struct OperationLock(String);

impl OperationLock {
    fn acquire(artifact_id: &str) -> Result<Self, String> {
        let mut active = operations()
            .lock()
            .map_err(|_| "MODEL_BUSY: Model state is unavailable.".to_owned())?;
        if !active.insert(artifact_id.to_owned()) {
            return Err(
                "MODEL_BUSY: This model is already being downloaded, imported, or removed.".into(),
            );
        }
        Ok(Self(artifact_id.to_owned()))
    }
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        if let Ok(mut active) = operations().lock() {
            active.remove(&self.0);
        }
    }
}

fn download_slots() -> &'static tokio::sync::Semaphore {
    static SLOTS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| tokio::sync::Semaphore::new(PARALLEL_DOWNLOADS))
}

// ------------------------------------------------------------------ layout

/// A single-file artifact keeps its pre-v2 name (`<artifact>.gguf`) so
/// existing installs survive; a multi-file bundle lives in its own folder
/// and is published by renaming a complete staging folder into place.
enum Layout {
    Single { file: PathBuf, partial: PathBuf },
    Bundle { folder: PathBuf, staging: PathBuf },
}

fn layout(directory: &Path, artifact: &CatalogArtifact) -> Result<Layout, String> {
    if artifact.files.len() == 1 {
        Ok(Layout::Single {
            file: safe_path(directory, &format!("{}.gguf", artifact.id))?,
            partial: safe_path(directory, &format!("{}.gguf.part", artifact.id))?,
        })
    } else {
        Ok(Layout::Bundle {
            folder: safe_path(directory, &artifact.id)?,
            staging: safe_path(directory, &format!("{}.partial", artifact.id))?,
        })
    }
}

fn journal_path(directory: &Path, artifact: &CatalogArtifact) -> Result<PathBuf, String> {
    safe_path(directory, &format!("{}.journal.json", artifact.id))
}

/// What a partial download is bound to. A resume continues only when the
/// journal names this exact artifact and file manifest.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Journal {
    version: u32,
    artifact_id: String,
    source_revision: String,
    files: Vec<JournalFile>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct JournalFile {
    path: String,
    size_bytes: u64,
    sha256: String,
}

fn journal_for(artifact: &CatalogArtifact) -> Journal {
    Journal {
        version: JOURNAL_VERSION,
        artifact_id: artifact.id.clone(),
        source_revision: artifact.source_revision.clone(),
        files: artifact
            .files
            .iter()
            .map(|file| JournalFile {
                path: file.path.clone(),
                size_bytes: file.size_bytes,
                sha256: file.sha256.clone(),
            })
            .collect(),
    }
}

fn read_journal(path: &Path) -> Option<Journal> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let staging = path.with_extension("tmp");
    {
        let mut file =
            File::create(&staging).map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
    }
    fs::rename(&staging, path).map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))
}

/// The file llama.cpp is given: the only file, or the first shard of a
/// split GGUF, from which it finds the others by name.
pub fn primary_file(artifact: &CatalogArtifact) -> Option<&CatalogArtifactFile> {
    artifact
        .files
        .iter()
        .find(|file| file.path.contains("-00001-of-"))
        .or_else(|| artifact.files.first())
}

fn bytes_on_disk(path: &Path) -> u64 {
    fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn bundle_ready(folder: &Path, artifact: &CatalogArtifact) -> Result<bool, String> {
    let marker = folder.join(READY_MARKER);
    if read_journal(&marker) != Some(journal_for(artifact)) {
        return Ok(false);
    }
    for file in &artifact.files {
        if !verify_against(
            &folder.join(&file.path),
            file.size_bytes,
            Some(&file.sha256),
            true,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn artifact_ready(directory: &Path, artifact: &CatalogArtifact) -> Result<bool, String> {
    match layout(directory, artifact)? {
        Layout::Single { file, .. } => Ok(file.is_file() && verify_file(&file, artifact)?),
        Layout::Bundle { folder, .. } => Ok(folder.is_dir() && bundle_ready(&folder, artifact)?),
    }
}

// ------------------------------------------------------------------ inventory

pub fn list<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<ManagedModel>, String> {
    let directory = models_directory(app)?;
    let mut found = Vec::new();
    for entry in catalog::entries()? {
        for artifact in &entry.artifacts {
            if let Some(model) = inspect_artifact(&directory, &entry, artifact)? {
                found.push(model);
            }
        }
    }
    Ok(found)
}

fn inspect_artifact(
    directory: &Path,
    entry: &CatalogEntry,
    artifact: &CatalogArtifact,
) -> Result<Option<ManagedModel>, String> {
    let primary = primary_file(artifact)
        .map(|file| file.path.clone())
        .unwrap_or_default();
    let record = |path: &Path, state: &str, completed: u64, verified: bool| ManagedModel {
        catalog_id: entry.id.clone(),
        artifact_id: artifact.id.clone(),
        variant_id: artifact.id.clone(),
        file_name: primary.clone(),
        path: path.to_string_lossy().into_owned(),
        size_bytes: artifact.size_bytes,
        sha256: artifact.sha256.clone().unwrap_or_default(),
        verified,
        state: state.into(),
        completed_bytes: completed,
        file_count: artifact.files.len(),
    };
    match layout(directory, artifact)? {
        Layout::Single { file, partial } => {
            if is_symlink(&partial)? {
                fs::remove_file(&partial)
                    .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
            }
            if file.is_file() {
                let verified = verify_file(&file, artifact)?;
                return Ok(Some(record(
                    &file,
                    if verified { "ready" } else { "corrupt" },
                    artifact.size_bytes,
                    verified,
                )));
            }
            if partial.is_file() {
                return Ok(Some(record(
                    &partial,
                    "partial",
                    bytes_on_disk(&partial),
                    false,
                )));
            }
        }
        Layout::Bundle { folder, staging } => {
            if folder.is_dir() {
                let verified = bundle_ready(&folder, artifact)?;
                return Ok(Some(record(
                    &folder,
                    if verified { "ready" } else { "corrupt" },
                    artifact.size_bytes,
                    verified,
                )));
            }
            if staging.is_dir() {
                let completed = artifact
                    .files
                    .iter()
                    .map(|file| {
                        let done = staging.join(&file.path);
                        if done.is_file() {
                            bytes_on_disk(&done)
                        } else {
                            bytes_on_disk(&staging.join(format!("{}.part", file.path)))
                        }
                    })
                    .sum();
                return Ok(Some(record(&staging, "partial", completed, false)));
            }
        }
    }
    Ok(None)
}

fn select_artifact<'a>(
    entry: &'a CatalogEntry,
    artifact_id: Option<&str>,
) -> Result<&'a CatalogArtifact, String> {
    match artifact_id {
        Some(id) => entry
            .artifacts
            .iter()
            .find(|artifact| artifact.id == id)
            .ok_or_else(|| {
                "MODEL_ARTIFACT_UNKNOWN: That download is not part of this model.".into()
            }),
        None => entry
            .artifacts
            .first()
            .ok_or_else(|| "The catalog entry has no artifact.".into()),
    }
}

/// Removes one artifact (or every artifact of the model) with its partial
/// files and journal. User files are never touched: imports were copied or
/// hard-linked into Juniper's folder.
pub fn remove<R: Runtime>(
    app: &AppHandle<R>,
    catalog_id: &str,
    artifact_id: Option<&str>,
) -> Result<(), String> {
    let directory = models_directory(app)?;
    let entry = catalog::find(catalog_id)?;
    let artifacts: Vec<&CatalogArtifact> = match artifact_id {
        Some(id) => vec![select_artifact(&entry, Some(id))?],
        None => entry.artifacts.iter().collect(),
    };
    for artifact in artifacts {
        let _lock = OperationLock::acquire(&artifact.id)?;
        let journal = journal_path(&directory, artifact)?;
        match layout(&directory, artifact)? {
            Layout::Single { file, partial } => {
                for path in [record_path(&file), file, partial, journal] {
                    if fs::symlink_metadata(&path).is_ok() {
                        fs::remove_file(path)
                            .map_err(|error| format!("MODEL_REMOVE_ERROR: {error}"))?;
                    }
                }
            }
            Layout::Bundle { folder, staging } => {
                for path in [folder, staging] {
                    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir()) {
                        fs::remove_dir_all(path)
                            .map_err(|error| format!("MODEL_REMOVE_ERROR: {error}"))?;
                    }
                }
                if journal.exists() {
                    fs::remove_file(journal)
                        .map_err(|error| format!("MODEL_REMOVE_ERROR: {error}"))?;
                }
            }
        }
    }
    Ok(())
}

/// The verified file the runtime loads for a model: the chosen artifact, or
/// the first installed one.
pub fn path_for_catalog<R: Runtime>(
    app: &AppHandle<R>,
    catalog_id: &str,
    artifact_id: Option<&str>,
) -> Result<(PathBuf, CatalogArtifact), String> {
    let directory = models_directory(app)?;
    let entry = catalog::find(catalog_id)?;
    let candidates: Vec<&CatalogArtifact> = match artifact_id {
        Some(id) => vec![select_artifact(&entry, Some(id))?],
        None => entry.artifacts.iter().collect(),
    };
    if candidates.is_empty()
        || candidates
            .iter()
            .all(|artifact| !catalog::supports_current_device(artifact))
    {
        return Err(
            "LOCAL_MODEL_UNSUPPORTED: This model artifact does not support this device.".into(),
        );
    }
    for artifact in candidates {
        if !catalog::supports_current_device(artifact) {
            continue;
        }
        if !artifact_ready(&directory, artifact)? {
            continue;
        }
        let primary =
            primary_file(artifact).ok_or("LOCAL_MODEL_NOT_READY: The artifact has no files.")?;
        let path = match layout(&directory, artifact)? {
            Layout::Single { file, .. } => file,
            Layout::Bundle { folder, .. } => folder.join(&primary.path),
        };
        return Ok((path, artifact.clone()));
    }
    Err("LOCAL_MODEL_NOT_READY: Download and verify this model before using it.".into())
}

// ------------------------------------------------------------------ download

fn storage_check(directory: &Path, remaining: u64) -> Result<(), String> {
    let required = remaining.saturating_add(STORAGE_HEADROOM_BYTES);
    if device::collect(directory)
        .free_storage_bytes
        .is_some_and(|free| free < required)
    {
        return Err(format!(
            "INSUFFICIENT_STORAGE: This download needs {} free (including 1 GB of headroom).",
            format_bytes(required)
        ));
    }
    Ok(())
}

fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(!cfg!(test))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(IDLE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent("Juniper/0.3 local model market")
        .build()
        .map_err(|_| {
            "MODEL_DOWNLOAD_ERROR: Juniper could not initialize its download client.".to_owned()
        })
}

/// Parses `bytes start-end/total`.
fn content_range(value: &str) -> Option<(u64, u64, u64)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

/// Progress across a whole artifact, for the progress events.
struct Progress<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    request_id: &'a str,
    total: u64,
    before: u64,
    last_emit: std::time::Instant,
}

impl<R: Runtime> Progress<'_, R> {
    fn report(&mut self, file_completed: u64, force: bool) {
        if force || self.last_emit.elapsed() >= Duration::from_millis(250) {
            self.last_emit = std::time::Instant::now();
            emit_progress(
                self.app,
                self.request_id,
                "downloading",
                self.before + file_completed,
                self.total,
                None,
            );
        }
    }
}

/// Downloads one file to `partial`, resuming from what is already there
/// when the server honours the exact range asked for. Every response is
/// checked against the manifest's size before a byte is kept.
async fn download_file<R: Runtime>(
    client: &reqwest::Client,
    file: &CatalogArtifactFile,
    partial: &Path,
    cancellation: &Cancellation,
    progress: &mut Progress<'_, R>,
) -> Result<(), String> {
    let url = file
        .url
        .as_deref()
        .ok_or_else(|| "MODEL_DOWNLOAD_ERROR: This file has no download source.".to_owned())?;
    // A partial file that is a link, or a hard link shared with another
    // file, is never written into: a restart would truncate the other file.
    if is_symlink(partial)? || shared_inode(partial) {
        fs::remove_file(partial).map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
    }
    let mut offset = bytes_on_disk(partial);
    if offset > file.size_bytes {
        fs::remove_file(partial).map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
        offset = 0;
    }
    for attempt in 0..2 {
        let mut request = client.get(url);
        if offset > 0 {
            request = request.header(
                RANGE,
                HeaderValue::from_str(&format!("bytes={offset}-"))
                    .map_err(|_| "MODEL_DOWNLOAD_ERROR: Invalid resume range.".to_owned())?,
            );
        }
        let response = tokio::select! {
            biased;
            _ = cancellation.wait() => return Err("MODEL_DOWNLOAD_CANCELLED: Download paused; you can resume it later.".into()),
            response = request.send() => response.map_err(|_| "MODEL_DOWNLOAD_ERROR: The model source did not respond. Check your connection.".to_owned())?,
        };
        let status = response.status();
        let resumed = match status {
            reqwest::StatusCode::PARTIAL_CONTENT if offset > 0 => {
                let range = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(content_range);
                if range != Some((offset, file.size_bytes - 1, file.size_bytes)) {
                    // A different range or total means a different file: start over.
                    fs::remove_file(partial)
                        .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
                    offset = 0;
                    if attempt == 0 {
                        continue;
                    }
                    return Err(
                        "MODEL_DOWNLOAD_ERROR: The source returned an unexpected byte range."
                            .into(),
                    );
                }
                true
            }
            reqwest::StatusCode::RANGE_NOT_SATISFIABLE if offset == file.size_bytes => {
                return Ok(());
            }
            reqwest::StatusCode::RANGE_NOT_SATISFIABLE => {
                fs::remove_file(partial)
                    .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
                offset = 0;
                continue;
            }
            status if status.is_success() => false,
            status => {
                return Err(format!(
                    "MODEL_DOWNLOAD_ERROR: The model source returned {status}."
                ));
            }
        };
        if !resumed {
            // The server sent the whole file (or this is a fresh start).
            offset = 0;
        }
        let expected = file.size_bytes - offset;
        if let Some(length) = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            && length != expected
        {
            return Err("MODEL_DOWNLOAD_ERROR: The source announced a different file size than the catalog.".into());
        }
        let mut output = if resumed {
            OpenOptions::new().append(true).open(partial)
        } else {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(partial)
        }
        .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
        let mut completed = offset;
        progress.report(completed, true);
        let mut stream = response.bytes_stream();
        loop {
            let next = tokio::select! {
                biased;
                _ = cancellation.wait() => {
                    let _ = output.flush();
                    return Err("MODEL_DOWNLOAD_CANCELLED: Download paused; you can resume it later.".into());
                }
                next = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => next.map_err(|_| "MODEL_DOWNLOAD_ERROR: The download stalled. It will resume from where it stopped.".to_owned())?,
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|_| "MODEL_DOWNLOAD_ERROR: The connection was interrupted. It will resume from where it stopped.".to_owned())?;
            completed = completed
                .checked_add(chunk.len() as u64)
                .filter(|value| *value <= file.size_bytes)
                .ok_or_else(|| {
                    "MODEL_DOWNLOAD_ERROR: The source returned more bytes than expected.".to_owned()
                })?;
            output
                .write_all(&chunk)
                .map_err(|error| match error.raw_os_error() {
                    #[cfg(unix)]
                    Some(code) if code == libc::ENOSPC => "INSUFFICIENT_STORAGE: The disk filled up during the download. Free space and resume.".to_owned(),
                    _ => format!("MODEL_STORAGE_ERROR: {error}"),
                })?;
            progress.report(completed, false);
        }
        output
            .flush()
            .and_then(|_| output.sync_all())
            .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
        if completed != file.size_bytes {
            return Err("MODEL_DOWNLOAD_ERROR: The download ended early. It will resume from where it stopped.".into());
        }
        return Ok(());
    }
    Err("MODEL_DOWNLOAD_ERROR: The source did not return the requested bytes.".into())
}

fn verify_partial(partial: &Path, file: &CatalogArtifactFile) -> Result<bool, String> {
    let mut hasher = Sha256::new();
    hash_existing(partial, &mut hasher)?;
    Ok(bytes_on_disk(partial) == file.size_bytes
        && format!("{:x}", hasher.finalize()) == file.sha256)
}

/// Downloads a catalog artifact. Each file is checked against its pinned
/// size and SHA-256 before the artifact is published; a bundle becomes
/// visible only when every file has verified.
pub async fn download<R: Runtime>(
    app: AppHandle<R>,
    state: &AppState,
    catalog_id: &str,
    artifact_id: Option<&str>,
    request_id: &str,
    cancellation: Cancellation,
) -> Result<(), String> {
    let entry = catalog::find(catalog_id)?;
    let artifact = select_artifact(&entry, artifact_id)?.clone();
    if !catalog::supports_current_device(&artifact) {
        return Err(
            "MODEL_ARTIFACT_UNSUPPORTED: This artifact does not support this device.".into(),
        );
    }
    if is_import_only(&artifact) {
        return Err(
            "MODEL_IMPORT_ONLY: Juniper has no download source for this model. Import a copy of the verified file instead.".into(),
        );
    }
    let directory = models_directory(&app)?;
    download_artifact(&app, &directory, &artifact, request_id, cancellation).await?;
    crate::commands::record_runtime_log(
        state,
        "model.ready",
        None,
        Some("juniper-local"),
        Some(catalog_id),
    );
    Ok(())
}

/// The transfer itself, for one artifact into `directory`.
pub(crate) async fn download_artifact<R: Runtime>(
    app: &AppHandle<R>,
    directory: &Path,
    artifact: &CatalogArtifact,
    request_id: &str,
    cancellation: Cancellation,
) -> Result<(), String> {
    let app = app.clone();
    let directory = directory.to_path_buf();
    let artifact = artifact.clone();
    let _lock = OperationLock::acquire(&artifact.id)?;
    if artifact_ready(&directory, &artifact)? {
        emit_progress(
            &app,
            request_id,
            "ready",
            artifact.size_bytes,
            artifact.size_bytes,
            None,
        );
        return Ok(());
    }
    emit_progress(&app, request_id, "queued", 0, artifact.size_bytes, None);
    let _slot = tokio::select! {
        biased;
        _ = cancellation.wait() => return Err("MODEL_DOWNLOAD_CANCELLED: Download cancelled before it started.".into()),
        slot = download_slots().acquire() => slot.map_err(|_| "MODEL_DOWNLOAD_ERROR: The download queue is unavailable.".to_owned())?,
    };
    let journal_file = journal_path(&directory, &artifact)?;
    let journal = journal_for(&artifact);
    let layout = layout(&directory, &artifact)?;
    // A journal for different bytes (a changed catalog entry) invalidates
    // the partial files rather than resuming into them.
    if read_journal(&journal_file).is_some_and(|existing| existing != journal) {
        match &layout {
            Layout::Single { partial, .. } => {
                let _ = fs::remove_file(partial);
            }
            Layout::Bundle { staging, .. } => {
                let _ = fs::remove_dir_all(staging);
            }
        }
    }
    let staging_folder = match &layout {
        Layout::Single { .. } => directory.clone(),
        Layout::Bundle { staging, .. } => {
            if fs::symlink_metadata(staging).is_ok_and(|metadata| !metadata.is_dir()) {
                fs::remove_file(staging)
                    .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
            }
            fs::create_dir_all(staging).map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
            staging.clone()
        }
    };
    let partial_for = |file: &CatalogArtifactFile| -> PathBuf {
        match &layout {
            Layout::Single { partial, .. } => partial.clone(),
            Layout::Bundle { staging, .. } => staging.join(format!("{}.part", file.path)),
        }
    };
    let done_for = |file: &CatalogArtifactFile| -> PathBuf {
        match &layout {
            Layout::Single { partial, .. } => partial.clone(),
            Layout::Bundle { staging, .. } => staging.join(&file.path),
        }
    };
    let on_disk: u64 = artifact
        .files
        .iter()
        .map(|file| bytes_on_disk(&done_for(file)).max(bytes_on_disk(&partial_for(file))))
        .sum();
    storage_check(&staging_folder, artifact.size_bytes.saturating_sub(on_disk))?;
    write_atomically(
        &journal_file,
        serde_json::to_string(&journal)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    )?;
    let client = download_client()?;
    let mut progress = Progress {
        app: &app,
        request_id,
        total: artifact.size_bytes,
        before: 0,
        last_emit: std::time::Instant::now(),
    };
    for file in &artifact.files {
        let done = done_for(file);
        let partial = partial_for(file);
        let already = matches!(layout, Layout::Bundle { .. })
            && done.is_file()
            && verify_against(&done, file.size_bytes, Some(&file.sha256), false)?;
        if !already {
            let result = download_file(&client, file, &partial, &cancellation, &mut progress).await;
            if let Err(error) = result {
                let paused = error.starts_with("MODEL_DOWNLOAD_CANCELLED");
                emit_progress(
                    &app,
                    request_id,
                    if paused { "paused" } else { "failed" },
                    progress.before + bytes_on_disk(&partial),
                    artifact.size_bytes,
                    Some((
                        error.split(':').next().unwrap_or("MODEL_DOWNLOAD_ERROR"),
                        error
                            .split_once(": ")
                            .map_or(error.as_str(), |(_, message)| message),
                    )),
                );
                return Err(error);
            }
            emit_progress(
                &app,
                request_id,
                "verifying",
                progress.before + file.size_bytes,
                artifact.size_bytes,
                None,
            );
            let path = partial.clone();
            let expected = file.clone();
            let verified = tokio::task::spawn_blocking(move || verify_partial(&path, &expected))
                .await
                .map_err(|_| "MODEL_DOWNLOAD_ERROR: Verification was interrupted.".to_owned())??;
            if !verified {
                let _ = fs::remove_file(&partial);
                emit_progress(
                    &app,
                    request_id,
                    "failed",
                    progress.before,
                    artifact.size_bytes,
                    Some((
                        "MODEL_CHECKSUM_MISMATCH",
                        "The model failed integrity verification and was removed.",
                    )),
                );
                return Err("MODEL_CHECKSUM_MISMATCH: The model failed integrity verification and was removed.".into());
            }
            if partial != done {
                fs::rename(&partial, &done)
                    .map_err(|error| format!("MODEL_INSTALL_ERROR: {error}"))?;
            }
        }
        progress.before += file.size_bytes;
    }
    match &layout {
        Layout::Single { file, partial } => {
            fs::rename(partial, file).map_err(|error| format!("MODEL_INSTALL_ERROR: {error}"))?;
        }
        Layout::Bundle { folder, staging } => {
            write_atomically(
                &staging.join(READY_MARKER),
                serde_json::to_string(&journal)
                    .map_err(|error| error.to_string())?
                    .as_bytes(),
            )?;
            if folder.exists() {
                fs::remove_dir_all(folder)
                    .map_err(|error| format!("MODEL_INSTALL_ERROR: {error}"))?;
            }
            // One rename publishes the whole bundle.
            fs::rename(staging, folder).map_err(|error| format!("MODEL_INSTALL_ERROR: {error}"))?;
        }
    }
    let _ = fs::remove_file(&journal_file);
    emit_progress(
        &app,
        request_id,
        "ready",
        artifact.size_bytes,
        artifact.size_bytes,
        None,
    );
    Ok(())
}

fn hash_existing(path: &Path, hasher: &mut Sha256) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let bytes = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }
    Ok(())
}

/// An artifact with no download source is installed only by importing a
/// file the user already has.
pub fn is_import_only(artifact: &CatalogArtifact) -> bool {
    artifact.source_url.is_none() && artifact.files.iter().all(|file| file.url.is_none())
}

/// A successful verification may be cached for this process while file size,
/// modification time, and change time remain stable. The cache is deliberately
/// memory-only: a writable sidecar cannot establish the identity of model
/// bytes across process restarts.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerificationRecord {
    sha256: String,
    size_bytes: u64,
    modified_nanos: u128,
    #[serde(default)]
    changed_nanos: Option<i128>,
}

#[cfg(unix)]
fn changed_nanos(metadata: &fs::Metadata) -> Option<i128> {
    Some(i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec()))
}

// The portable metadata API exposes a user-settable modification time on
// Windows. Without a change-time signal, the verification cache is disabled.
#[cfg(not(unix))]
fn changed_nanos(_metadata: &fs::Metadata) -> Option<i128> {
    None
}

/// Filesystems with coarse change-time ticks can report the same ctime for a
/// rapid same-size edit and mtime restore. Probe the model's own filesystem
/// before trusting the verification cache; failure or coarse timestamps mean
/// the model is rehashed on each use.
#[cfg(unix)]
fn ctime_is_precise_enough(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let probe = parent.join(format!(
        ".juniper-ctime-probe-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let mut created = false;
    let result = (|| -> Option<bool> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&probe)
            .ok()?;
        created = true;
        file.write_all(&[0]).ok()?;
        let mut previous = changed_nanos(&file.metadata().ok()?)?;
        let started = std::time::Instant::now();
        for value in 1u8..=16 {
            file.set_len(0).ok()?;
            file.seek(SeekFrom::Start(0)).ok()?;
            file.write_all(&[value]).ok()?;
            file.flush().ok()?;
            let current = changed_nanos(&file.metadata().ok()?)?;
            if current <= previous {
                return Some(false);
            }
            previous = current;
        }
        Some(started.elapsed() <= std::time::Duration::from_secs(1))
    })()
    .unwrap_or(false);
    if created {
        let _ = fs::remove_file(probe);
    }
    result
}

#[cfg(not(unix))]
fn ctime_is_precise_enough(_path: &Path) -> bool {
    false
}

fn record_path(path: &Path) -> PathBuf {
    path.with_extension("gguf.verified")
}

fn verified_cache() -> &'static Mutex<HashMap<PathBuf, VerificationRecord>> {
    static VERIFIED: OnceLock<Mutex<HashMap<PathBuf, VerificationRecord>>> = OnceLock::new();
    VERIFIED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn current_record(path: &Path, sha256: Option<&str>) -> Option<VerificationRecord> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    Some(VerificationRecord {
        sha256: sha256?.to_owned(),
        size_bytes: metadata.len(),
        modified_nanos: metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos(),
        changed_nanos: changed_nanos(&metadata),
    })
}

/// Whether the regular file at `path` has exactly this size and SHA-256.
/// Qualified native artifacts are always hashed: a neighboring verification
/// record is writable data, so it cannot establish their pinned identity.
#[cfg(not(target_os = "android"))]
pub(crate) fn verify_expected(path: &Path, size_bytes: u64, sha256: &str) -> Result<bool, String> {
    verify_against(path, size_bytes, Some(sha256), false)
}

fn verify_against(
    path: &Path,
    size_bytes: u64,
    sha256: Option<&str>,
    use_cache: bool,
) -> Result<bool, String> {
    // Legacy versions wrote user-writable receipts. They never authorize a
    // cache hit, and are discarded even when this inexpensive preflight
    // rejects the file before hashing it.
    let _ = fs::remove_file(record_path(path));
    let link_metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
        return Ok(false);
    }
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() != size_bytes {
        return Ok(false);
    }
    let before = current_record(path, sha256);
    if use_cache
        && before
            .as_ref()
            .is_some_and(|record| record.changed_nanos.is_some())
        && ctime_is_precise_enough(path)
        && verified_cache()
            .lock()
            .ok()
            .and_then(|cache| cache.get(path).cloned())
            == before
    {
        return Ok(true);
    }
    let mut hasher = Sha256::new();
    hash_existing(path, &mut hasher)?;
    let verified = sha256 == Some(format!("{:x}", hasher.finalize()).as_str());
    // A file that changed while it was being hashed is not the file that was
    // hashed; it is neither trusted now nor recorded.
    let unchanged = before.is_some() && before == current_record(path, sha256);
    if use_cache && verified && unchanged && ctime_is_precise_enough(path) {
        if let (Some(record), Ok(mut cache)) = (before, verified_cache().lock()) {
            cache.insert(path.to_owned(), record);
        }
    } else {
        if let Ok(mut cache) = verified_cache().lock() {
            cache.remove(path);
        }
    }
    Ok(verified && unchanged)
}

fn verify_file(path: &Path, artifact: &CatalogArtifact) -> Result<bool, String> {
    let Some(file) = artifact.files.first().filter(|_| artifact.files.len() == 1) else {
        return Ok(false);
    };
    verify_against(path, file.size_bytes, Some(&file.sha256), true)
}

/// Installs a single-file catalog artifact from a file the user selected.
/// On the same filesystem the file is hard-linked rather than copied, so a
/// 12 GB model is not duplicated; either way the installed bytes are hashed
/// against the catalog before they become usable.
pub async fn import<R: Runtime>(
    app: AppHandle<R>,
    state: &AppState,
    catalog_id: &str,
    artifact_id: Option<&str>,
    source: &Path,
    request_id: &str,
    cancellation: Cancellation,
) -> Result<(), String> {
    let entry = catalog::find(catalog_id)?;
    let artifact = select_artifact(&entry, artifact_id)?.clone();
    if !catalog::supports_current_device(&artifact) {
        return Err(
            "MODEL_ARTIFACT_UNSUPPORTED: This artifact does not support this device.".into(),
        );
    }
    if artifact.files.len() != 1 {
        return Err("MODEL_IMPORT_ERROR: This download has several files; import supports single-file models.".into());
    }
    let _lock = OperationLock::acquire(&artifact.id)?;
    let source_metadata = fs::symlink_metadata(source)
        .map_err(|_| "MODEL_IMPORT_ERROR: The selected file is no longer available.".to_owned())?;
    if !source_metadata.file_type().is_file() {
        return Err("MODEL_IMPORT_ERROR: Select a regular file, not a link.".into());
    }
    if source_metadata.len() != artifact.size_bytes {
        return Err(format!(
            "MODEL_CHECKSUM_MISMATCH: The selected file is {}, but this model's verified file is {}.",
            format_bytes(source_metadata.len()),
            format_bytes(artifact.size_bytes)
        ));
    }
    let directory = models_directory(&app)?;
    let Layout::Single {
        file: final_file, ..
    } = layout(&directory, &artifact)?
    else {
        unreachable!("single-file artifacts use the single layout");
    };
    // Imports stage under their own name: a hard link to the user's file
    // must never be picked up as a download's partial file.
    let partial_file = safe_path(&directory, &format!("{}.gguf.import", artifact.id))?;
    if final_file.is_file() && verify_file(&final_file, &artifact)? {
        emit_progress(
            &app,
            request_id,
            "ready",
            artifact.size_bytes,
            artifact.size_bytes,
            None,
        );
        return Ok(());
    }
    if fs::symlink_metadata(&partial_file).is_ok() {
        fs::remove_file(&partial_file).map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
    }
    let linked = fs::hard_link(source, &partial_file).is_ok();
    if !linked {
        storage_check(&directory, artifact.size_bytes)?;
    }
    let expected_sha = artifact.files[0].sha256.clone();
    let before = current_record(&partial_file, Some(&expected_sha));
    let task_source = source.to_owned();
    let task_partial = partial_file.clone();
    let task_app = app.clone();
    let task_request = request_id.to_owned();
    let total = artifact.size_bytes;
    let task_cancellation = cancellation.clone();
    let digest = tokio::task::spawn_blocking(move || {
        hash_into_place(
            &task_source,
            &task_partial,
            linked,
            total,
            &task_cancellation,
            |completed| {
                emit_progress(
                    &task_app,
                    &task_request,
                    if linked { "verifying" } else { "copying" },
                    completed,
                    total,
                    None,
                )
            },
        )
    })
    .await
    .map_err(|_| "MODEL_IMPORT_ERROR: The import was interrupted.".to_owned())?;
    let digest = match digest {
        Ok(digest) => digest,
        Err(error) => {
            let _ = fs::remove_file(&partial_file);
            return Err(error);
        }
    };
    if digest != expected_sha {
        let _ = fs::remove_file(&partial_file);
        emit_progress(
            &app,
            request_id,
            "failed",
            total,
            total,
            Some((
                "MODEL_CHECKSUM_MISMATCH",
                "The selected file is not the verified model file.",
            )),
        );
        return Err(
            "MODEL_CHECKSUM_MISMATCH: The selected file is not the verified model file.".into(),
        );
    }
    // A hard link shares its bytes with the user's file, which can still be
    // changed under its original name; record only what was actually hashed.
    if linked && before != current_record(&partial_file, Some(&expected_sha)) {
        let _ = fs::remove_file(&partial_file);
        return Err("MODEL_CHECKSUM_MISMATCH: The selected file changed during import.".into());
    }
    fs::rename(&partial_file, &final_file)
        .map_err(|error| format!("MODEL_INSTALL_ERROR: {error}"))?;
    emit_progress(&app, request_id, "ready", total, total, None);
    crate::commands::record_runtime_log(
        state,
        "model.imported",
        None,
        Some("juniper-local"),
        Some(catalog_id),
    );
    Ok(())
}

/// Hashes the bytes that end up at `destination`: the linked file itself, or
/// each chunk as it is copied, so what is verified is what is installed.
fn hash_into_place(
    source: &Path,
    destination: &Path,
    linked: bool,
    total: u64,
    cancellation: &Cancellation,
    progress: impl Fn(u64),
) -> Result<String, String> {
    let mut reader = File::open(if linked { destination } else { source })
        .map_err(|_| "MODEL_IMPORT_ERROR: The selected file could not be read.".to_owned())?;
    let mut writer = if linked {
        None
    } else {
        Some(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)
                .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?,
        )
    };
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 4 * 1024 * 1024];
    let mut completed = 0u64;
    let mut reported = 0u64;
    loop {
        if cancellation.is_cancelled() {
            return Err("MODEL_IMPORT_CANCELLED: Import cancelled.".into());
        }
        let bytes = reader
            .read(&mut buffer)
            .map_err(|_| "MODEL_IMPORT_ERROR: The selected file could not be read.".to_owned())?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
        if let Some(writer) = writer.as_mut() {
            writer
                .write_all(&buffer[..bytes])
                .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
        }
        completed += bytes as u64;
        if completed > total {
            return Err("MODEL_CHECKSUM_MISMATCH: The selected file changed during import.".into());
        }
        if completed - reported >= 64 * 1024 * 1024 || completed == total {
            reported = completed;
            progress(completed);
        }
    }
    if let Some(mut writer) = writer {
        writer
            .flush()
            .and_then(|_| writer.sync_all())
            .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
    }
    if completed != total {
        return Err("MODEL_CHECKSUM_MISMATCH: The selected file changed during import.".into());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(unix)]
fn shared_inode(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.nlink() > 1)
}

#[cfg(not(unix))]
fn shared_inode(_path: &Path) -> bool {
    false
}

fn is_symlink(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn safe_path(directory: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || name.starts_with('.')
        || !name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        return Err("MODEL_PATH_ERROR: The model filename is unsafe.".into());
    }
    Ok(directory.join(name))
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}

fn emit_progress<R: Runtime>(
    app: &AppHandle<R>,
    request_id: &str,
    status: &str,
    completed: u64,
    total: u64,
    error: Option<(&str, &str)>,
) {
    let _ = app.emit(
        &format!("juniper://model-download/{request_id}"),
        crate::domain::ModelPullProgress {
            request_id: request_id.into(),
            status: status.into(),
            digest: None,
            completed_bytes: Some(completed),
            total_bytes: Some(total),
            done: Some(matches!(status, "ready" | "failed")),
            error: error.map(|(code, message)| crate::domain::RuntimeError {
                code: code.into(),
                message: message.into(),
            }),
        },
    );
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn artifact_for_test(size_bytes: u64, sha256: &str) -> CatalogArtifact {
        CatalogArtifact {
            id: "test-model-q4".into(),
            runtime_id: "llama.cpp".into(),
            format: "GGUF".into(),
            platforms: vec!["linux".into()],
            architectures: vec!["x86_64".into()],
            quantization: Some("Q4_K_M".into()),
            size_bytes,
            sha256: Some(sha256.into()),
            backend_profile: None,
            capabilities: None,
            source_url: Some("https://huggingface.co/test/model".into()),
            source_revision: "main".into(),
            files: vec![crate::catalog::CatalogArtifactFile {
                path: "test-model.gguf".into(),
                size_bytes,
                sha256: sha256.into(),
                url: Some("https://huggingface.co/test/model/resolve/main/test-model.gguf".into()),
            }],
            minimum_runtime_version: None,
            maturity: "stable".into(),
            qualification: "qualified".into(),
        }
    }

    #[test]
    fn ordinary_catalog_verification_ignores_a_forged_sidecar() {
        let root =
            std::env::temp_dir().join(format!("juniper-record-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("test directory should be created");
        let path = root.join("model.gguf");
        fs::write(&path, b"model").expect("model written");
        let expected = format!("{:x}", Sha256::digest(b"RIGHT_MODEL"));
        let artifact = artifact_for_test(11, &expected);
        let record = current_record(&path, Some(&expected)).expect("record fixture");
        fs::write(record_path(&path), serde_json::to_vec(&record).unwrap())
            .expect("forged writable receipt");
        assert!(!verify_file(&path, &artifact).expect("ordinary catalog verification"));
        assert!(!record_path(&path).exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn qualified_native_hashes_ignore_forged_verification_records() {
        let root =
            std::env::temp_dir().join(format!("juniper-forged-record-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("directory");
        let path = root.join("model.gguf");
        fs::write(&path, b"WRONG_MODEL").expect("model");
        let expected = format!("{:x}", Sha256::digest(b"RIGHT_MODEL"));
        // A distributor can write a receipt with the expected digest and the
        // wrong file's real timestamps. Those timestamps do not prove hashing.
        let forged = current_record(&path, Some(&expected)).expect("record fixture");
        fs::write(record_path(&path), serde_json::to_vec(&forged).unwrap())
            .expect("forged writable receipt");
        assert!(!verify_expected(&path, 11, &expected).expect("verify"));
        fs::write(&path, b"RIGHT_MODEL").expect("correct model");
        assert!(verify_expected(&path, 11, &expected).expect("verify correct bytes"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn verification_rejects_symlinks() {
        let root =
            std::env::temp_dir().join(format!("juniper-model-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("test directory should be created");
        let target = root.join("outside.bin");
        let link = root.join("model.gguf");
        fs::write(&target, b"model").expect("test target should be written");
        std::os::unix::fs::symlink(&target, &link).expect("test symlink should be created");
        let digest = format!("{:x}", Sha256::digest(b"model"));
        assert!(
            !verify_file(&link, &artifact_for_test(5, &digest)).expect("verification should run")
        );
        fs::remove_file(&link).expect("test symlink should be removable");
        fs::remove_file(&target).expect("test target should be removable");
        fs::remove_dir(&root).expect("test directory should be removable");
    }
}

/// Transfer behaviour against a scripted HTTP server. These are fixture
/// tests (no real model source): each case controls exactly what the server
/// sends so resume, range, size, digest, bundle, and lock rules are exercised.
#[cfg(test)]
mod transfer_tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::sync::Arc;

    #[derive(Clone, Copy)]
    enum Behaviour {
        /// Honour `Range` with a correct `Content-Range`.
        Honest,
        /// Ignore `Range` and send the whole file with 200.
        IgnoreRange,
        /// Answer a range with a mismatched `Content-Range`.
        WrongRange,
        /// Close the connection after this many bytes.
        Truncate(usize),
        /// Send extra bytes after the file.
        Oversize,
    }

    /// Serves `files` (name → bytes) and records each request's Range header.
    fn server(
        files: Vec<(&'static str, Vec<u8>)>,
        behaviour: Behaviour,
    ) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let seen = ranges.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap_or(0);
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .trim_start_matches('/')
                    .to_owned();
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = value.trim().trim_end_matches('-').parse::<usize>().ok();
                    }
                }
                seen.lock()
                    .unwrap()
                    .push(range.map(|start| start.to_string()));
                let Some((_, body)) = files.iter().find(|(name, _)| *name == path) else {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    continue;
                };
                let total = body.len();
                let (status, start, header) = match (range, behaviour) {
                    (Some(start), Behaviour::Honest | Behaviour::Truncate(_)) => (
                        "206 Partial Content",
                        start,
                        format!("Content-Range: bytes {start}-{}/{total}\r\n", total - 1),
                    ),
                    (Some(start), Behaviour::WrongRange) => (
                        "206 Partial Content",
                        start,
                        format!("Content-Range: bytes 0-{}/{}\r\n", total - 1, total + 7),
                    ),
                    _ => ("200 OK", 0, String::new()),
                };
                let mut payload = body[start..].to_vec();
                if let Behaviour::Truncate(limit) = behaviour {
                    let head = format!(
                        "HTTP/1.1 {status}\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&payload[..limit.min(payload.len())]);
                    continue;
                }
                if let Behaviour::Oversize = behaviour {
                    payload.extend_from_slice(b"EXTRA");
                }
                let head = format!(
                    "HTTP/1.1 {status}\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&payload);
            }
        });
        (format!("http://{address}"), ranges)
    }

    fn artifact(id: &str, base: &str, files: &[(&str, &[u8])]) -> CatalogArtifact {
        let files: Vec<CatalogArtifactFile> = files
            .iter()
            .map(|(name, bytes)| CatalogArtifactFile {
                path: (*name).into(),
                size_bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
                url: Some(format!("{base}/{name}")),
            })
            .collect();
        CatalogArtifact {
            id: id.into(),
            runtime_id: "llama.cpp".into(),
            format: "GGUF".into(),
            platforms: vec!["linux".into()],
            architectures: vec!["x86_64".into()],
            quantization: Some("Q4_K_M".into()),
            size_bytes: files.iter().map(|file| file.size_bytes).sum(),
            sha256: (files.len() == 1).then(|| files[0].sha256.clone()),
            backend_profile: None,
            capabilities: None,
            source_url: None,
            source_revision: "fixture".into(),
            files,
            minimum_runtime_version: None,
            maturity: "beta".into(),
            qualification: "package-only".into(),
        }
    }

    fn folder() -> PathBuf {
        let path = std::env::temp_dir().join(format!("juniper-transfer-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn run(directory: &Path, artifact: &CatalogArtifact) -> Result<(), String> {
        let app = tauri::test::mock_app();
        tauri::async_runtime::block_on(download_artifact(
            app.handle(),
            directory,
            artifact,
            "test",
            Cancellation::default(),
        ))
    }

    fn model_bytes(length: usize, seed: u8) -> Vec<u8> {
        (0..length)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    #[test]
    fn a_partial_download_resumes_with_an_exact_range_and_verifies() {
        let bytes = model_bytes(300_000, 1);
        let (base, ranges) = server(vec![("model.gguf", bytes.clone())], Behaviour::Honest);
        let directory = folder();
        let artifact = artifact("fixture-a", &base, &[("model.gguf", &bytes)]);
        fs::write(directory.join("fixture-a.gguf.part"), &bytes[..120_000]).unwrap();
        fs::write(
            directory.join("fixture-a.journal.json"),
            serde_json::to_string(&journal_for(&artifact)).unwrap(),
        )
        .unwrap();
        run(&directory, &artifact).expect("resumed download");
        assert_eq!(ranges.lock().unwrap()[0].as_deref(), Some("120000"));
        assert_eq!(fs::read(directory.join("fixture-a.gguf")).unwrap(), bytes);
        assert!(!directory.join("fixture-a.gguf.part").exists());
        assert!(!directory.join("fixture-a.journal.json").exists());
        assert!(artifact_ready(&directory, &artifact).unwrap());
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn an_ignored_or_mismatched_range_restarts_from_the_beginning() {
        for behaviour in [Behaviour::IgnoreRange, Behaviour::WrongRange] {
            let bytes = model_bytes(200_000, 2);
            let (base, _) = server(vec![("model.gguf", bytes.clone())], behaviour);
            let directory = folder();
            let artifact = artifact("fixture-b", &base, &[("model.gguf", &bytes)]);
            // A stale partial with different bytes must not survive.
            fs::write(directory.join("fixture-b.gguf.part"), vec![0xAA; 50_000]).unwrap();
            run(&directory, &artifact).expect("download restarts cleanly");
            assert_eq!(fs::read(directory.join("fixture-b.gguf")).unwrap(), bytes);
            fs::remove_dir_all(directory).ok();
        }
    }

    #[test]
    fn corrupt_oversized_and_truncated_responses_never_become_ready() {
        let bytes = model_bytes(100_000, 3);
        let directory = folder();
        // Corrupt: the manifest digest is for other bytes.
        let (base, _) = server(vec![("model.gguf", bytes.clone())], Behaviour::Honest);
        let mut corrupt = artifact("fixture-c", &base, &[("model.gguf", &bytes)]);
        corrupt.files[0].sha256 = format!("{:x}", Sha256::digest(b"something else"));
        assert!(
            run(&directory, &corrupt)
                .unwrap_err()
                .starts_with("MODEL_CHECKSUM_MISMATCH")
        );
        assert!(!directory.join("fixture-c.gguf").exists());
        assert!(
            !directory.join("fixture-c.gguf.part").exists(),
            "a corrupt download is removed"
        );
        // Oversized: more bytes than the manifest.
        let (base, _) = server(vec![("model.gguf", bytes.clone())], Behaviour::Oversize);
        let oversized = artifact("fixture-d", &base, &[("model.gguf", &bytes)]);
        assert!(run(&directory, &oversized).is_err());
        assert!(!directory.join("fixture-d.gguf").exists());
        // Truncated: the partial stays for a later resume, never as ready.
        let (base, _) = server(
            vec![("model.gguf", bytes.clone())],
            Behaviour::Truncate(40_000),
        );
        let truncated = artifact("fixture-e", &base, &[("model.gguf", &bytes)]);
        assert!(run(&directory, &truncated).is_err());
        assert!(!directory.join("fixture-e.gguf").exists());
        assert!(bytes_on_disk(&directory.join("fixture-e.gguf.part")) > 0);
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn a_bundle_is_published_only_when_every_file_verifies() {
        let first = model_bytes(150_000, 4);
        let second = model_bytes(90_000, 5);
        let directory = folder();
        // The second shard is truncated: nothing is published.
        let (base, _) = server(
            vec![
                ("m-00001-of-00002.gguf", first.clone()),
                ("m-00002-of-00002.gguf", second.clone()),
            ],
            Behaviour::Truncate(70_000),
        );
        let bundle = artifact(
            "fixture-f",
            &base,
            &[
                ("m-00001-of-00002.gguf", &first),
                ("m-00002-of-00002.gguf", &second),
            ],
        );
        assert!(run(&directory, &bundle).is_err());
        assert!(
            !directory.join("fixture-f").exists(),
            "an incomplete bundle is never in place"
        );
        assert!(directory.join("fixture-f.partial").is_dir());
        assert!(!artifact_ready(&directory, &bundle).unwrap());
        // Resumed against an honest server, it completes and appears at once.
        let (base, _) = server(
            vec![
                ("m-00001-of-00002.gguf", first.clone()),
                ("m-00002-of-00002.gguf", second.clone()),
            ],
            Behaviour::Honest,
        );
        let bundle = artifact(
            "fixture-f",
            &base,
            &[
                ("m-00001-of-00002.gguf", &first),
                ("m-00002-of-00002.gguf", &second),
            ],
        );
        run(&directory, &bundle).expect("bundle completes");
        assert!(directory.join("fixture-f").is_dir());
        assert!(!directory.join("fixture-f.partial").exists());
        assert!(artifact_ready(&directory, &bundle).unwrap());
        assert_eq!(primary_file(&bundle).unwrap().path, "m-00001-of-00002.gguf");
        // Tampering with one shard makes the bundle not ready.
        fs::write(
            directory.join("fixture-f/m-00002-of-00002.gguf"),
            vec![0u8; second.len()],
        )
        .unwrap();
        assert!(!artifact_ready(&directory, &bundle).unwrap());
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn conflicting_operations_are_refused_and_changed_manifests_discard_partials() {
        let held = OperationLock::acquire("fixture-g").unwrap();
        assert!(
            OperationLock::acquire("fixture-g")
                .unwrap_err()
                .starts_with("MODEL_BUSY")
        );
        drop(held);
        assert!(OperationLock::acquire("fixture-g").is_ok());

        let bytes = model_bytes(80_000, 6);
        let (base, ranges) = server(vec![("model.gguf", bytes.clone())], Behaviour::Honest);
        let directory = folder();
        let artifact = artifact("fixture-h", &base, &[("model.gguf", &bytes)]);
        let mut stale = journal_for(&artifact);
        stale.source_revision = "an-older-revision".into();
        fs::write(
            directory.join("fixture-h.journal.json"),
            serde_json::to_string(&stale).unwrap(),
        )
        .unwrap();
        fs::write(directory.join("fixture-h.gguf.part"), &bytes[..30_000]).unwrap();
        run(&directory, &artifact).expect("download");
        assert_eq!(
            ranges.lock().unwrap()[0],
            None,
            "a partial from another revision is not resumed"
        );
        fs::remove_dir_all(directory).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_linked_partial_file_is_replaced_not_written_into() {
        let bytes = model_bytes(60_000, 7);
        let (base, ranges) = server(vec![("model.gguf", bytes.clone())], Behaviour::IgnoreRange);
        let directory = folder();
        let users_file = directory.join("users-own.gguf");
        fs::write(&users_file, b"the user's own data").unwrap();
        fs::hard_link(&users_file, directory.join("fixture-i.gguf.part")).unwrap();
        let artifact = artifact("fixture-i", &base, &[("model.gguf", &bytes)]);
        run(&directory, &artifact).expect("download");
        assert_eq!(fs::read(&users_file).unwrap(), b"the user's own data");
        assert_eq!(
            ranges.lock().unwrap()[0],
            None,
            "the shared file was not resumed"
        );
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn content_ranges_are_parsed_strictly() {
        assert_eq!(content_range("bytes 10-99/100"), Some((10, 99, 100)));
        assert_eq!(content_range("bytes */100"), None);
        assert_eq!(content_range("items 1-2/3"), None);
    }
}
