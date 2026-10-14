//! Juniper 1's native token-ID backend (finding A-02).
//!
//! llama.cpp's own text tokenizer does not reproduce Juniper 1's tokenizer
//! (`j1tok`) and, with special parsing, turns literal control spellings into
//! control IDs. So Juniper 1 runs only here: the host tokenizes with the
//! canonical `j1tok`, serializes `juniper-chat-v1` itself, and exchanges token
//! IDs with a qualified `llama-server`. Every text path keeps refusing Juniper 1
//! GGUFs (`crate::gguf_guard`), and this backend is reachable only for an
//! artifact the token-ID profile pins.

pub mod chat;
pub mod client;
pub mod install;
pub mod profile;
pub mod protocol;
pub mod tokenizer;

#[cfg(test)]
mod golden;
#[cfg(test)]
mod tests;

use crate::gguf_guard;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A Juniper 1 artifact whose file, metadata, and tokenizer were verified.
pub struct Verified {
    pub profile: profile::TokenIdProfile,
    pub artifact: profile::TokenIdArtifact,
    pub model_path: PathBuf,
    pub tokenizer: Arc<tokenizer::J1Tokenizer>,
}

/// Verifies what can be checked before a server starts: the profile names
/// the artifact; the installed file has its pinned size and SHA-256; its GGUF
/// metadata declares the pinned contract, protocol, tokenizer hash, source
/// manifest, and shape; and the tokenizer file hashes to the pinned value and
/// loads with `encode_special_tokens`.
pub fn verify_installed(models_directory: &Path, artifact_id: &str) -> Result<Verified, String> {
    let (profile, artifact) = profile::find(artifact_id).ok_or_else(|| {
        "BACKEND_PROFILE_UNKNOWN: This model is not a qualified Juniper 1 artifact.".to_owned()
    })?;
    let model_path = models_directory.join(artifact.file_name());
    let not_ready = || {
        "LOCAL_MODEL_NOT_READY: Install and verify this Juniper 1 model before using it.".to_owned()
    };
    if !crate::managed_models::verify_expected(&model_path, artifact.size_bytes, &artifact.sha256)
        .map_err(|_| not_ready())?
    {
        return Err(not_ready());
    }
    let mut model = BufReader::with_capacity(
        1 << 16,
        crate::commands::open_regular_file(
            &model_path,
            "LOCAL_MODEL_NOT_READY: The Juniper 1 model file is unavailable.",
        )?,
    );
    let tokenizer = verify_contents(
        &mut model,
        read_tokenizer(&models_directory.join(&profile.tokenizer.file)),
        &profile,
        &artifact,
    )?;
    Ok(Verified {
        profile,
        artifact,
        model_path,
        tokenizer: Arc::new(tokenizer),
    })
}

fn read_tokenizer(path: &Path) -> Result<Vec<u8>, String> {
    let unavailable =
        "JUNIPER1_TOKENIZER_UNAVAILABLE: Juniper 1's tokenizer file is not installed.";
    let mut bytes = Vec::new();
    crate::commands::open_regular_file(path, unavailable)?
        .take(tokenizer::MAX_TOKENIZER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable.to_owned())?;
    Ok(bytes)
}

/// The GGUF header must declare what the profile pins, then the tokenizer
/// must be the pinned file.
fn verify_contents<R: Read + Seek>(
    model: &mut R,
    tokenizer: Result<Vec<u8>, String>,
    profile: &profile::TokenIdProfile,
    artifact: &profile::TokenIdArtifact,
) -> Result<tokenizer::J1Tokenizer, String> {
    let header = gguf_guard::read_header(model).map_err(|_| {
        "JUNIPER1_MODEL_MISMATCH: The Juniper 1 model file has no readable GGUF header.".to_owned()
    })?;
    profile::verify_header(&header, profile, artifact)?;
    tokenizer::J1Tokenizer::load(
        &tokenizer?,
        &profile.tokenizer.sha256,
        profile.tokenizer.vocab_size,
    )
}
