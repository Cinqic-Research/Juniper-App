use crate::catalog::{self, CatalogArtifact, CatalogEntry};
use crate::commands::{AppState, Cancellation};
use crate::device;
use futures_util::StreamExt;
use reqwest::header::{HeaderValue, RANGE};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::time::Duration;

const STORAGE_HEADROOM_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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
    pub state: String,
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

pub fn list<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<ManagedModel>, String> {
    let directory = models_directory(app)?;
    let entries = catalog::entries()?;
    let inspected = entries
        .iter()
        .map(|entry| inspect_entry(&directory, entry))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(inspected.into_iter().flatten().collect())
}

fn inspect_entry(directory: &Path, entry: &CatalogEntry) -> Result<Option<ManagedModel>, String> {
    let artifact = entry
        .artifacts
        .first()
        .ok_or_else(|| "The catalog entry has no downloadable artifact.".to_owned())?;
    let path = final_path(directory, artifact)?;
    let partial = partial_path(directory, artifact)?;
    if is_symlink(&partial)? {
        fs::remove_file(&partial).map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
    }
    if path.is_file() {
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        let verified = verify_file(&path, artifact)?;
        return Ok(Some(ManagedModel {
            catalog_id: entry.id.clone(),
            artifact_id: artifact.id.clone(),
            variant_id: artifact.id.clone(),
            file_name: artifact
                .files
                .first()
                .map(|file| file.path.clone())
                .unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            size_bytes: metadata.len(),
            sha256: artifact.sha256.clone().unwrap_or_default(),
            verified,
            state: if verified { "ready" } else { "corrupt" }.into(),
        }));
    }
    if partial.is_file() {
        let metadata = fs::metadata(&partial).map_err(|error| error.to_string())?;
        return Ok(Some(ManagedModel {
            catalog_id: entry.id.clone(),
            artifact_id: artifact.id.clone(),
            variant_id: artifact.id.clone(),
            file_name: artifact
                .files
                .first()
                .map(|file| file.path.clone())
                .unwrap_or_default(),
            path: partial.to_string_lossy().into_owned(),
            size_bytes: metadata.len(),
            sha256: artifact.sha256.clone().unwrap_or_default(),
            verified: false,
            state: "partial".into(),
        }));
    }
    Ok(None)
}

pub fn remove<R: Runtime>(app: &AppHandle<R>, catalog_id: &str) -> Result<(), String> {
    let directory = models_directory(app)?;
    let entry = catalog::find(catalog_id)?;
    let artifact = entry
        .artifacts
        .first()
        .ok_or_else(|| "The catalog entry has no downloadable artifact.".to_owned())?;
    let installed = final_path(&directory, artifact)?;
    for path in [
        record_path(&installed),
        installed.clone(),
        partial_path(&directory, artifact)?,
    ] {
        if path.exists() {
            fs::remove_file(path).map_err(|error| format!("MODEL_REMOVE_ERROR: {error}"))?;
        }
    }
    Ok(())
}

pub fn path_for_catalog<R: Runtime>(
    app: &AppHandle<R>,
    catalog_id: &str,
) -> Result<PathBuf, String> {
    let directory = models_directory(app)?;
    let entry = catalog::find(catalog_id)?;
    let artifact = entry
        .artifacts
        .first()
        .ok_or_else(|| "The catalog entry has no downloadable artifact.".to_owned())?;
    let path = final_path(&directory, artifact)?;
    if !path.is_file() || !verify_file(&path, artifact)? {
        return Err(
            "LOCAL_MODEL_NOT_READY: Download and verify this model before using it.".into(),
        );
    }
    Ok(path)
}

pub async fn download<R: Runtime>(
    app: AppHandle<R>,
    state: &AppState,
    catalog_id: &str,
    request_id: &str,
    cancellation: Cancellation,
) -> Result<(), String> {
    let entry = catalog::find(catalog_id)?;
    let artifact = entry
        .artifacts
        .first()
        .ok_or_else(|| "The catalog entry has no downloadable artifact.".to_owned())?;
    if is_import_only(artifact) {
        return Err(
            "MODEL_IMPORT_ONLY: Juniper has no download source for this model. Import a copy of the verified file instead.".into(),
        );
    }
    if artifact.size_bytes > MAX_DOWNLOAD_BYTES {
        return Err(
            "MODEL_TOO_LARGE: This model is larger than Juniper's safe download limit.".into(),
        );
    }
    let directory = models_directory(&app)?;
    let final_file = final_path(&directory, artifact)?;
    let partial_file = partial_path(&directory, artifact)?;
    if is_symlink(&partial_file)? {
        fs::remove_file(&partial_file).map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
    }
    let capabilities = device::collect(&directory);
    let required = artifact
        .size_bytes
        .saturating_add(STORAGE_HEADROOM_BYTES)
        .max(entry.minimum_storage_bytes);
    if capabilities
        .free_storage_bytes
        .is_some_and(|free| free < required)
    {
        return Err(format!(
            "INSUFFICIENT_STORAGE: Free {} before downloading this model.",
            format_bytes(required)
        ));
    }
    if final_file.is_file() && verify_file(&final_file, artifact)? {
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
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .user_agent("Juniper/0.3 local model market")
        .build()
        .map_err(|_| {
            "MODEL_DOWNLOAD_ERROR: Juniper could not initialize its download client.".to_owned()
        })?;
    let mut offset = partial_file
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if offset > artifact.size_bytes {
        fs::remove_file(&partial_file).map_err(|error| error.to_string())?;
        offset = 0;
    }
    let source_url = artifact
        .source_url
        .as_deref()
        .or_else(|| artifact.files.first().and_then(|file| file.url.as_deref()))
        .ok_or_else(|| "MODEL_DOWNLOAD_ERROR: This artifact has no source URL.".to_owned())?;
    let mut request = client.get(source_url);
    if offset > 0 {
        request = request.header(
            RANGE,
            HeaderValue::from_str(&format!("bytes={offset}-"))
                .map_err(|_| "MODEL_DOWNLOAD_ERROR: Invalid resume range.".to_owned())?,
        );
    }
    let response = request
        .send()
        .await
        .map_err(|_| "MODEL_DOWNLOAD_ERROR: The model source did not respond.".to_owned())?;
    let resumed = offset > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() {
        return Err(format!(
            "MODEL_DOWNLOAD_ERROR: The model source returned {}.",
            response.status()
        ));
    }
    if !resumed {
        offset = 0;
    }
    let mut file = if resumed {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&partial_file)
    } else {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&partial_file)
    }
    .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
    let mut hasher = Sha256::new();
    if resumed {
        hash_existing(&partial_file, &mut hasher)?;
    }
    emit_progress(
        &app,
        request_id,
        "downloading",
        offset,
        artifact.size_bytes,
        None,
    );
    let mut completed = offset;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancellation.is_cancelled() {
            emit_progress(
                &app,
                request_id,
                "paused",
                completed,
                artifact.size_bytes,
                Some((
                    "MODEL_DOWNLOAD_CANCELLED",
                    "Download paused; you can resume it later.",
                )),
            );
            return Err(
                "MODEL_DOWNLOAD_CANCELLED: Download paused; you can resume it later.".into(),
            );
        }
        let chunk = chunk
            .map_err(|_| "MODEL_DOWNLOAD_ERROR: The connection was interrupted.".to_owned())?;
        completed = completed.saturating_add(chunk.len() as u64);
        if completed > artifact.size_bytes {
            return Err(
                "MODEL_DOWNLOAD_ERROR: The source returned more bytes than expected.".into(),
            );
        }
        file.write_all(&chunk)
            .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
        hasher.update(&chunk);
        emit_progress(
            &app,
            request_id,
            "downloading",
            completed,
            artifact.size_bytes,
            None,
        );
    }
    file.flush()
        .map_err(|error| format!("MODEL_STORAGE_ERROR: {error}"))?;
    if completed != artifact.size_bytes {
        return Err(
            "MODEL_DOWNLOAD_ERROR: The download ended before the expected file size.".into(),
        );
    }
    let actual = format!("{:x}", hasher.finalize());
    if artifact.sha256.as_deref() != Some(actual.as_str()) {
        let _ = fs::remove_file(&partial_file);
        emit_progress(
            &app,
            request_id,
            "failed",
            completed,
            artifact.size_bytes,
            Some((
                "MODEL_CHECKSUM_MISMATCH",
                "The model failed integrity verification and was removed.",
            )),
        );
        return Err(
            "MODEL_CHECKSUM_MISMATCH: The model failed integrity verification and was removed."
                .into(),
        );
    }
    fs::rename(&partial_file, &final_file)
        .map_err(|error| format!("MODEL_INSTALL_ERROR: {error}"))?;
    // Juniper wrote and hashed these bytes itself; nothing else holds the file.
    write_record(&final_file, current_record(&final_file, artifact));
    emit_progress(
        &app,
        request_id,
        "ready",
        completed,
        artifact.size_bytes,
        None,
    );
    crate::commands::record_runtime_log(
        state,
        "model.ready",
        None,
        Some("juniper-local"),
        Some(catalog_id),
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

/// Hashing a multi-gigabyte model takes minutes on a hard disk, so a
/// successful verification is recorded beside the file and trusted only while
/// the file's size and modification time are unchanged. The record is no
/// stronger than the file it describes: both live in Juniper's data directory.
#[derive(Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerificationRecord {
    sha256: String,
    size_bytes: u64,
    modified_nanos: u128,
}

fn record_path(path: &Path) -> PathBuf {
    path.with_extension("gguf.verified")
}

fn current_record(path: &Path, artifact: &CatalogArtifact) -> Option<VerificationRecord> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    Some(VerificationRecord {
        sha256: artifact.sha256.clone()?,
        size_bytes: metadata.len(),
        modified_nanos: metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos(),
    })
}

fn write_record(path: &Path, record: Option<VerificationRecord>) {
    if let Some(record) = record
        && let Ok(text) = serde_json::to_string(&record)
    {
        let _ = fs::write(record_path(path), text);
    }
}

fn verify_file(path: &Path, artifact: &CatalogArtifact) -> Result<bool, String> {
    let link_metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
        return Ok(false);
    }
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() != artifact.size_bytes {
        return Ok(false);
    }
    let recorded = fs::read_to_string(record_path(path))
        .ok()
        .and_then(|text| serde_json::from_str::<VerificationRecord>(&text).ok());
    let before = current_record(path, artifact);
    if recorded.is_some() && recorded == before {
        return Ok(true);
    }
    let mut hasher = Sha256::new();
    hash_existing(path, &mut hasher)?;
    let verified = artifact.sha256.as_deref() == Some(format!("{:x}", hasher.finalize()).as_str());
    // A file that changed while it was being hashed is not the file that was
    // hashed; it is neither trusted now nor recorded.
    let unchanged = before.is_some() && before == current_record(path, artifact);
    if verified && unchanged {
        write_record(path, before);
    } else {
        let _ = fs::remove_file(record_path(path));
    }
    Ok(verified && unchanged)
}

/// Installs a catalog artifact from a file the user selected. On the same
/// filesystem the file is hard-linked rather than copied, so a 12 GB model is
/// not duplicated; either way the installed bytes are hashed against the
/// catalog before they become usable.
pub async fn import<R: Runtime>(
    app: AppHandle<R>,
    state: &AppState,
    catalog_id: &str,
    source: &Path,
    request_id: &str,
    cancellation: Cancellation,
) -> Result<(), String> {
    let entry = catalog::find(catalog_id)?;
    let artifact = entry
        .artifacts
        .first()
        .ok_or_else(|| "The catalog entry has no artifact.".to_owned())?
        .clone();
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
    let final_file = final_path(&directory, &artifact)?;
    let partial_file = partial_path(&directory, &artifact)?;
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
        let required = artifact
            .size_bytes
            .saturating_add(STORAGE_HEADROOM_BYTES)
            .max(entry.minimum_storage_bytes);
        if device::collect(&directory)
            .free_storage_bytes
            .is_some_and(|free| free < required)
        {
            return Err(format!(
                "INSUFFICIENT_STORAGE: Free {} before importing this model.",
                format_bytes(required)
            ));
        }
    }
    let before = current_record(&partial_file, &artifact);
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
    if artifact.sha256.as_deref() != Some(digest.as_str()) {
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
    if linked && before != current_record(&partial_file, &artifact) {
        let _ = fs::remove_file(&partial_file);
        return Err("MODEL_CHECKSUM_MISMATCH: The selected file changed during import.".into());
    }
    fs::rename(&partial_file, &final_file)
        .map_err(|error| format!("MODEL_INSTALL_ERROR: {error}"))?;
    write_record(&final_file, current_record(&final_file, &artifact));
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

fn is_symlink(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn final_path(directory: &Path, artifact: &CatalogArtifact) -> Result<PathBuf, String> {
    // Keep the pre-v2 filename stable so existing managed GGUF files survive
    // the catalog migration without a destructive re-download.
    safe_path(directory, &format!("{}.gguf", artifact.id))
}

fn partial_path(directory: &Path, artifact: &CatalogArtifact) -> Result<PathBuf, String> {
    safe_path(directory, &format!("{}.gguf.part", artifact.id))
}

fn safe_path(directory: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
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
    fn a_verification_record_is_trusted_only_for_the_unchanged_file() {
        let root =
            std::env::temp_dir().join(format!("juniper-record-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("test directory should be created");
        let path = root.join("model.gguf");
        fs::write(&path, b"model").expect("model written");
        let artifact = artifact_for_test(5, &format!("{:x}", Sha256::digest(b"model")));
        assert!(verify_file(&path, &artifact).expect("verify"));
        assert!(record_path(&path).exists());
        assert!(verify_file(&path, &artifact).expect("cached verify"));
        // Same size, different bytes, newer modification time: rehashed and rejected.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, b"MODEL").expect("model replaced");
        assert!(!verify_file(&path, &artifact).expect("verify"));
        assert!(!record_path(&path).exists());
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
