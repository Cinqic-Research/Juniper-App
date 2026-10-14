//! The qualified token-ID profile for Juniper 1 (`config/backends/juniper1-token-ids.v1.json`).
//!
//! A Juniper 1 artifact runs only if the profile names it, its file has the
//! pinned size and SHA-256, its GGUF metadata declares the token-ID contract,
//! protocol, tokenizer hash, and source manifest the profile pins, the
//! tokenizer file hashes to the pinned value, and the runtime reports the
//! pinned llama.cpp build. A file name alone never qualifies a model. The
//! artifacts are deliberately absent from the public model catalog.

use super::protocol::PROTOCOL;
use crate::backend::RuntimeRequirement;
use crate::gguf_guard::Header;
use serde::Deserialize;

const PROFILE_JSON: &str = include_str!("../../../config/backends/juniper1-token-ids.v1.json");
pub const IO_CONTRACT: &str = "token-ids-only";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenIdProfile {
    pub id: String,
    pub backend: String,
    pub io_contract: String,
    pub protocol: String,
    pub qualification: String,
    pub tokenizer: TokenizerIdentity,
    pub runtime: RuntimeRequirement,
    pub server: TokenIdServer,
    pub lifecycle: TokenIdLifecycle,
    pub generation: TokenIdGeneration,
    pub artifacts: Vec<TokenIdArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenizerIdentity {
    pub id: String,
    pub file: String,
    pub sha256: String,
    pub vocab_size: u32,
    pub library: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenIdServer {
    pub ctx_size: u32,
    pub n_gpu_layers: u32,
    pub parallel: u32,
    pub cache_ram_mib: u32,
    pub batch_size: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenIdLifecycle {
    pub startup_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenIdGeneration {
    pub temperature: f32,
    pub top_k: u32,
    pub top_p: f32,
    pub default_max_tokens: u32,
    pub max_tokens_ceiling: u32,
    pub max_segments: u32,
    pub max_tool_result_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenIdArtifact {
    pub id: String,
    pub display_name: String,
    pub stage: String,
    pub release: String,
    pub source_checkpoint: String,
    pub source_manifest_sha256: String,
    pub architecture: String,
    pub quantization: String,
    pub size_bytes: u64,
    pub sha256: String,
}

impl TokenIdArtifact {
    /// The installed file name in Juniper's managed model directory.
    pub fn file_name(&self) -> String {
        format!("{}.gguf", self.id)
    }

    /// What the model is, as the host can verify it; never a production claim.
    pub fn lineage(&self, profile: &TokenIdProfile) -> String {
        format!(
            "{}, trained by Cinqic (checkpoint {}, {} weights), served on this device by {} {} through Juniper's token-ID backend",
            self.display_name,
            self.source_checkpoint,
            self.quantization,
            profile.runtime.engine,
            profile.runtime.tag
        )
    }
}

fn hex64(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
}

pub fn profile() -> Result<TokenIdProfile, String> {
    let invalid = || {
        "BACKEND_PROFILE_INVALID: The bundled Juniper 1 token-ID profile is malformed.".to_owned()
    };
    let profile: TokenIdProfile = serde_json::from_str(PROFILE_JSON).map_err(|_| invalid())?;
    let generation = &profile.generation;
    let mut ids = std::collections::HashSet::new();
    let valid = profile.backend == "juniper-chat-v1-token-ids"
        && !profile.qualification.is_empty()
        && profile.io_contract == IO_CONTRACT
        && profile.protocol == PROTOCOL
        && !profile.tokenizer.id.is_empty()
        && profile.tokenizer.library == "tokenizers 0.23.2"
        && hex64(&profile.tokenizer.sha256)
        && safe_name(&profile.tokenizer.file)
        && profile.tokenizer.vocab_size > super::tokenizer::FIRST_LEARNED_ID
        && profile.runtime.commit.len() == 40
        && profile
            .runtime
            .commit
            .chars()
            .all(|c| c.is_ascii_hexdigit())
        && profile.server.ctx_size >= 256
        && profile.server.parallel == 1
        && generation.default_max_tokens >= 1
        && generation.default_max_tokens <= generation.max_tokens_ceiling
        && generation.max_tokens_ceiling < profile.server.ctx_size
        && generation.max_segments >= 1
        && generation.max_tool_result_tokens >= 64
        && (0.0..=2.0).contains(&generation.temperature)
        && (0.0..=1.0).contains(&generation.top_p)
        && !profile.artifacts.is_empty()
        && profile.artifacts.iter().all(|artifact| {
            safe_name(&artifact.id)
                && ids.insert(artifact.id.clone())
                && hex64(&artifact.sha256)
                && hex64(&artifact.source_manifest_sha256)
                && artifact.size_bytes > 0
                && !artifact.architecture.is_empty()
                && matches!(artifact.stage.as_str(), "base" | "sft-candidate")
                // Never presented as a production model.
                && matches!(artifact.release.as_str(), "research-only" | "qualified-non-production")
        });
    if valid { Ok(profile) } else { Err(invalid()) }
}

/// The profile and artifact for a model ID, if the profile names it.
pub fn find(artifact_id: &str) -> Option<(TokenIdProfile, TokenIdArtifact)> {
    let profile = profile().ok()?;
    let artifact = profile
        .artifacts
        .iter()
        .find(|artifact| artifact.id == artifact_id)?
        .clone();
    Some((profile, artifact))
}

/// The GGUF must declare exactly what the profile pins for this artifact.
pub fn verify_header(
    header: &Header,
    profile: &TokenIdProfile,
    artifact: &TokenIdArtifact,
) -> Result<(), String> {
    let mismatch = |what: &str| {
        Err(format!(
            "JUNIPER1_MODEL_MISMATCH: The model file's {what} does not match its qualified Juniper 1 profile."
        ))
    };
    if header.string("juniper.io_contract") != Some(IO_CONTRACT) {
        return mismatch("I/O contract");
    }
    if header.string("juniper.protocol") != Some(profile.protocol.as_str()) {
        return mismatch("protocol");
    }
    if header.string("juniper.tokenizer_sha256") != Some(profile.tokenizer.sha256.as_str()) {
        return mismatch("tokenizer hash");
    }
    if header.string("juniper.source_checkpoint_manifest_sha256")
        != Some(artifact.source_manifest_sha256.as_str())
    {
        return mismatch("source checkpoint manifest");
    }
    if header.string("general.architecture") != Some(artifact.architecture.as_str()) {
        return mismatch("architecture");
    }
    if header
        .integers
        .get(&format!("{}.context_length", artifact.architecture))
        != Some(&u64::from(profile.server.ctx_size))
    {
        return mismatch("context length");
    }
    if header.array_lengths.get("tokenizer.ggml.tokens")
        != Some(&u64::from(profile.tokenizer.vocab_size))
    {
        return mismatch("vocabulary size");
    }
    if header.integers.get("tokenizer.ggml.eot_token_id") != Some(&u64::from(super::tokenizer::EOT))
        || header.bools.get("tokenizer.ggml.add_bos_token") != Some(&false)
    {
        return mismatch("special-token settings");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_profile_pins_the_qualified_identities() {
        let profile = profile().expect("profile");
        assert_eq!(profile.io_contract, "token-ids-only");
        assert_eq!(profile.protocol, "juniper-chat-v1");
        assert_eq!(
            profile.tokenizer.sha256,
            "176a89e2197c24d3f024fbb60ce6f346635e7cba912b3579d93b37d486c9adf1"
        );
        assert_eq!(profile.tokenizer.vocab_size, 24_576);
        assert_eq!(
            profile.runtime.commit,
            "748d4225b9016b17ce4bcfa69fdc2c39f473a965"
        );
        assert_eq!(profile.server.ctx_size, 2048);
        let ids: Vec<&str> = profile.artifacts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["juniper1-base-v1-f16", "juniper1-sft-pilot-v1-f16"]);
        let candidate = &profile.artifacts[1];
        assert_eq!(candidate.release, "qualified-non-production");
        assert!(candidate.display_name.contains("not a production model"));
        assert!(candidate.lineage(&profile).contains("trained by Cinqic"));
    }

    #[test]
    fn juniper_one_is_not_in_the_public_catalog() {
        let profile = profile().expect("profile");
        for entry in crate::catalog::entries().expect("catalog") {
            assert!(!entry.id.contains("juniper1"), "{}", entry.id);
            for artifact in entry.artifacts {
                assert!(
                    profile.artifacts.iter().all(|j1| j1.id != artifact.id
                        && artifact.sha256.as_deref() != Some(j1.sha256.as_str())),
                    "{}",
                    artifact.id
                );
            }
        }
        assert!(find("gpt-oss-20b").is_none());
        assert!(find("juniper1-sft-pilot-v1-f16").is_some());
    }

    pub(crate) fn header_for(artifact: &TokenIdArtifact) -> Header {
        let profile = profile().expect("profile");
        let mut header = Header::default();
        for (key, value) in [
            ("general.architecture", artifact.architecture.as_str()),
            ("general.name", "juniper-1 (experimental export)"),
            ("juniper.io_contract", "token-ids-only"),
            ("juniper.protocol", "juniper-chat-v1"),
            (
                "juniper.tokenizer_sha256",
                profile.tokenizer.sha256.as_str(),
            ),
            (
                "juniper.source_checkpoint_manifest_sha256",
                artifact.source_manifest_sha256.as_str(),
            ),
        ] {
            header.strings.insert(key.into(), value.into());
            if key.starts_with("juniper.") {
                header.juniper_keys.insert(key.into());
            }
        }
        header
            .integers
            .insert(format!("{}.context_length", artifact.architecture), 2048);
        header
            .integers
            .insert("tokenizer.ggml.eot_token_id".into(), 11);
        header
            .bools
            .insert("tokenizer.ggml.add_bos_token".into(), false);
        header
            .array_lengths
            .insert("tokenizer.ggml.tokens".into(), 24_576);
        header
    }

    #[test]
    fn metadata_mismatches_are_refused() {
        let profile = profile().expect("profile");
        let base = &profile.artifacts[0];
        let candidate = &profile.artifacts[1];
        assert!(verify_header(&header_for(base), &profile, base).is_ok());
        // The base's metadata does not qualify the candidate's profile entry.
        assert!(verify_header(&header_for(base), &profile, candidate).is_err());
        type Mutation = Box<dyn Fn(&mut Header)>;
        let mutations: Vec<Mutation> = vec![
            Box::new(|h| {
                h.strings.remove("juniper.io_contract");
            }),
            Box::new(|h| {
                h.strings
                    .insert("juniper.io_contract".into(), "text".into());
            }),
            Box::new(|h| {
                h.strings.insert("juniper.protocol".into(), "chatml".into());
            }),
            Box::new(|h| {
                h.strings
                    .insert("juniper.tokenizer_sha256".into(), "0".repeat(64));
            }),
            Box::new(|h| {
                h.strings
                    .insert("general.architecture".into(), "llama".into());
            }),
            Box::new(|h| {
                h.integers.insert("qwen3.context_length".into(), 4096);
            }),
            Box::new(|h| {
                h.array_lengths
                    .insert("tokenizer.ggml.tokens".into(), 32_768);
            }),
            Box::new(|h| {
                h.bools.insert("tokenizer.ggml.add_bos_token".into(), true);
            }),
            Box::new(|h| {
                h.integers.insert("tokenizer.ggml.eot_token_id".into(), 0);
            }),
        ];
        for (index, mutate) in mutations.iter().enumerate() {
            let mut header = header_for(base);
            mutate(&mut header);
            assert!(
                verify_header(&header, &profile, base)
                    .unwrap_err()
                    .starts_with("JUNIPER1_MODEL_MISMATCH"),
                "mutation {index}"
            );
        }
    }
}
