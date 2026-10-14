//! Local, explicitly chosen research artifacts. No download source or public
//! catalog entry is created; the bundled profile owns every accepted identity.

// Only the Linux native route imports; other desktop builds compile the listing alone.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::{profile, tokenizer};
use crate::commands::{Cancellation, open_regular_file};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    pub id: String,
    pub display_name: String,
    pub checkpoint: String,
    pub stage: String,
    pub context_size: u32,
    pub max_output: u32,
    pub installed: bool,
}

pub fn list(directory: &Path) -> Result<Vec<LocalModel>, String> {
    if !cfg!(target_os = "linux") {
        return Ok(Vec::new());
    }
    let profile = profile::profile()?;
    Ok(profile
        .artifacts
        .iter()
        .map(|artifact| LocalModel {
            id: artifact.id.clone(),
            display_name: artifact.display_name.clone(),
            checkpoint: artifact.source_checkpoint.clone(),
            stage: artifact.stage.clone(),
            context_size: profile.server.ctx_size,
            max_output: profile.generation.max_tokens_ceiling,
            installed: super::verify_installed(directory, &artifact.id).is_ok(),
        })
        .collect())
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_verified(
    source: &Path,
    destination: &Path,
    size: u64,
    expected: &str,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let failed = || {
        "JUNIPER1_IMPORT_INVALID: The selected file does not match the qualified identity."
            .to_owned()
    };
    let mut input = open_regular_file(
        source,
        "JUNIPER1_IMPORT_INVALID: Select a regular local file.",
    )?;
    if input.metadata().map_err(|_| failed())?.len() != size {
        return Err(failed());
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut output = options.open(destination).map_err(|_| failed())?;
    let mut hash = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        if cancellation.is_cancelled() {
            return Err("REQUEST_CANCELLED: Import cancelled.".into());
        }
        let count = input.read(&mut buffer).map_err(|_| failed())?;
        if count == 0 {
            break;
        }
        copied = copied.saturating_add(count as u64);
        if copied > size {
            return Err(failed());
        }
        output.write_all(&buffer[..count]).map_err(|_| failed())?;
        hash.update(&buffer[..count]);
    }
    if copied != size || format!("{:x}", hash.finalize()) != expected {
        return Err(failed());
    }
    output.sync_all().map_err(|_| failed())
}

/// Atomically publishes a verified file without replacing existing material.
fn publish(source: &Path, destination: &Path, size: u64, hash: &str) -> Result<(), String> {
    match fs::hard_link(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if crate::managed_models::verify_expected(destination, size, hash).unwrap_or(false) {
                Ok(())
            } else {
                Err("JUNIPER1_IMPORT_CONFLICT: An existing file has a different identity; preserve it and choose a clean model directory.".into())
            }
        }
        Err(_) => Err("JUNIPER1_IMPORT_FAILED: The verified bundle could not be installed.".into()),
    }
}

pub fn import(
    directory: &Path,
    artifact_id: &str,
    model_source: &Path,
    tokenizer_source: &Path,
    cancellation: &Cancellation,
) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("JUNIPER1_PLATFORM_NOT_QUALIFIED: Native Juniper 1 activation is qualified only on Linux.".into());
    }
    let (profile, artifact) = profile::find(artifact_id).ok_or_else(|| {
        "BACKEND_PROFILE_UNKNOWN: Select a bundled Juniper 1 research profile.".to_owned()
    })?;
    let tokenizer_bytes = super::read_tokenizer(tokenizer_source)?;
    tokenizer::J1Tokenizer::load(
        &tokenizer_bytes,
        &profile.tokenizer.sha256,
        profile.tokenizer.vocab_size,
    )?;
    let required = artifact.size_bytes + tokenizer_bytes.len() as u64 + 512 * 1024 * 1024;
    if crate::device::collect(directory)
        .free_storage_bytes
        .is_none_or(|free| free < required)
    {
        return Err("INSUFFICIENT_STORAGE: This verified local import needs space for the bundle and 512 MiB of headroom.".into());
    }
    let staging = Staging(directory.join(format!(
        ".juniper1-import-{}",
        uuid::Uuid::new_v4().simple()
    )));
    fs::create_dir(&staging.0).map_err(|_| {
        "JUNIPER1_IMPORT_FAILED: The import directory could not be prepared.".to_owned()
    })?;
    let model = staging.0.join(artifact.file_name());
    let tokenizer = staging.0.join(&profile.tokenizer.file);
    copy_verified(
        model_source,
        &model,
        artifact.size_bytes,
        &artifact.sha256,
        cancellation,
    )?;
    copy_verified(
        tokenizer_source,
        &tokenizer,
        tokenizer_bytes.len() as u64,
        &profile.tokenizer.sha256,
        cancellation,
    )?;
    let mut model_file = open_regular_file(
        &model,
        "JUNIPER1_IMPORT_FAILED: The verified model became unavailable.",
    )?;
    super::verify_contents(
        &mut model_file,
        Ok(tokenizer_bytes.clone()),
        &profile,
        &artifact,
    )?;
    if cancellation.is_cancelled() {
        return Err("REQUEST_CANCELLED: Import cancelled.".into());
    }
    // A tokenizer alone never makes a model usable. Each file is installed
    // atomically; interruption between them leaves a safely incomplete bundle.
    publish(
        &tokenizer,
        &directory.join(&profile.tokenizer.file),
        tokenizer_bytes.len() as u64,
        &profile.tokenizer.sha256,
    )?;
    publish(
        &model,
        &directory.join(artifact.file_name()),
        artifact.size_bytes,
        &artifact.sha256,
    )?;
    super::verify_installed(directory, artifact_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publishing_refuses_conflicting_files_and_preserves_their_bytes() {
        let root =
            std::env::temp_dir().join(format!("j1-import-conflict-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).expect("directory");
        let _cleanup = Staging(root.clone());
        let staged = root.join("staged");
        let installed = root.join("installed");
        fs::write(&staged, b"RIGHT").expect("staged");
        fs::write(&installed, b"WRONG").expect("existing");
        let digest = format!("{:x}", Sha256::digest(b"RIGHT"));
        assert!(
            publish(&staged, &installed, 5, &digest)
                .unwrap_err()
                .starts_with("JUNIPER1_IMPORT_CONFLICT")
        );
        assert_eq!(fs::read(&installed).expect("preserved"), b"WRONG");
        fs::remove_file(&installed).expect("remove synthetic conflicting file");
        publish(&staged, &installed, 5, &digest).expect("install");
        publish(&staged, &installed, 5, &digest).expect("same identity");
    }
}
