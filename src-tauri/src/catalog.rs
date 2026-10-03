use serde::{Deserialize, Serialize};

const CATALOG_JSON: &str = include_str!("../../config/models/catalog.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogArtifactFile {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactCapabilities {
    pub tools: String,
    pub thinking: String,
    pub generation_parameters: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogArtifact {
    pub id: String,
    pub runtime_id: String,
    /// A qualified backend profile (`config/backends`) this artifact must run
    /// with. Absent for models served by the generic local backend.
    #[serde(default)]
    pub backend_profile: Option<String>,
    /// Capabilities established by qualification. Absent means unknown, and
    /// unknown is never treated as supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ArtifactCapabilities>,
    pub format: String,
    pub platforms: Vec<String>,
    pub architectures: Vec<String>,
    pub quantization: Option<String>,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub source_url: Option<String>,
    pub source_revision: String,
    pub files: Vec<CatalogArtifactFile>,
    pub minimum_runtime_version: Option<String>,
    pub maturity: String,
    pub qualification: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: String,
    pub display_name: String,
    pub organization: String,
    pub family: String,
    pub parameter_count: u64,
    pub description: String,
    pub use_cases: Vec<String>,
    pub instruction_tuned: bool,
    pub architecture: String,
    pub source_repository: String,
    pub source_revision: String,
    pub original_model: Option<String>,
    pub license: String,
    pub license_url: String,
    pub attribution: String,
    pub chat_template: String,
    pub context_length: u64,
    pub minimum_recommended_ram_bytes: u64,
    pub recommended_ram_bytes: u64,
    pub minimum_storage_bytes: u64,
    pub supported_architectures: Vec<String>,
    pub tags: Vec<String>,
    pub release_status: String,
    pub artifacts: Vec<CatalogArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogDocument {
    version: u32,
    minimum_app_version: String,
    models: Vec<CatalogEntry>,
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

pub fn entries() -> Result<Vec<CatalogEntry>, String> {
    let document: CatalogDocument = serde_json::from_str(CATALOG_JSON)
        .map_err(|_| "The bundled model catalog is malformed.".to_owned())?;
    if document.version != 2 || document.minimum_app_version.is_empty() {
        return Err("The bundled model catalog version is unsupported.".into());
    }
    let mut ids = std::collections::HashSet::new();
    let mut artifact_ids = std::collections::HashSet::new();
    for entry in &document.models {
        if entry.id.is_empty()
            || !ids.insert(entry.id.clone())
            || entry.parameter_count == 0
            || entry.artifacts.is_empty()
        {
            return Err("The bundled model catalog contains invalid metadata.".into());
        }
        for artifact in &entry.artifacts {
            if artifact.id.is_empty()
                || !artifact_ids.insert(artifact.id.clone())
                || artifact.runtime_id.is_empty()
                || artifact.format.is_empty()
                || artifact.platforms.is_empty()
                || artifact.architectures.is_empty()
                || artifact.size_bytes == 0
                || artifact.source_revision.is_empty()
                || artifact
                    .backend_profile
                    .as_deref()
                    .is_some_and(|profile| crate::backend::profile(profile).is_err())
                || artifact.capabilities.as_ref().is_some_and(|capabilities| {
                    [&capabilities.tools, &capabilities.thinking]
                        .iter()
                        .any(|level| {
                            !matches!(level.as_str(), "supported" | "unsupported" | "unknown")
                        })
                })
                || !matches!(
                    artifact.maturity.as_str(),
                    "stable" | "beta" | "experimental"
                )
                || !matches!(
                    artifact.qualification.as_str(),
                    "qualified" | "package-only" | "not-qualified" | "unknown"
                )
                || artifact
                    .sha256
                    .as_deref()
                    .is_some_and(|hash| !valid_hash(hash))
                || artifact.source_url.as_deref().is_some_and(|url| {
                    !url.starts_with("https://") || !url.contains("huggingface.co/")
                })
                || artifact.files.is_empty()
                || artifact.files.iter().any(|file| {
                    file.path.is_empty()
                        || file.path.contains('/')
                        || file.path.contains('\\')
                        || file.size_bytes == 0
                        || !valid_hash(&file.sha256)
                        || file
                            .url
                            .as_deref()
                            .is_some_and(|url| !url.starts_with("https://"))
                })
                || artifact
                    .files
                    .iter()
                    .map(|file| file.size_bytes)
                    .sum::<u64>()
                    != artifact.size_bytes
            {
                return Err("The bundled model catalog contains invalid artifact metadata.".into());
            }
        }
    }
    Ok(document.models)
}

pub fn find(id: &str) -> Result<CatalogEntry, String> {
    entries()?
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| "That model is not in Juniper's trusted catalog.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_is_artifact_centric_and_integrity_pinned() {
        let entries = entries().expect("catalog should parse");
        assert_eq!(entries.len(), 5);
        assert!(
            entries
                .iter()
                .flat_map(|entry| entry.artifacts.iter())
                .all(|artifact| {
                    artifact.runtime_id == "llama.cpp"
                        && artifact.files.iter().all(|file| valid_hash(&file.sha256))
                })
        );
        // Downloadable entries stay small; a large model is installed only
        // from a file the user already has and that matches the pinned hash.
        for entry in &entries {
            let downloadable = entry.artifacts[0]
                .files
                .iter()
                .all(|file| file.url.is_some());
            assert!(
                !downloadable || entry.parameter_count < 1_000_000_000,
                "{}",
                entry.id
            );
        }
    }

    #[test]
    fn gpt_oss_is_import_only_and_bound_to_its_qualified_profile() {
        let entry = find("gpt-oss-20b").expect("gpt-oss-20b should be cataloged");
        let artifact = &entry.artifacts[0];
        assert!(artifact.files.iter().all(|file| file.url.is_none()));
        assert!(artifact.source_url.is_none());
        let profile = crate::backend::profile(
            artifact
                .backend_profile
                .as_deref()
                .expect("backend profile"),
        )
        .expect("profile resolves");
        assert_eq!(artifact.id, profile.artifact.id);
        assert_eq!(
            artifact.sha256.as_deref(),
            Some(profile.artifact.sha256.as_str())
        );
        assert_eq!(artifact.size_bytes, profile.artifact.size_bytes);
        assert_eq!(entry.source_revision, profile.model.revision);
        assert_eq!(entry.context_length, u64::from(profile.server.ctx_size));
    }

    #[test]
    fn artifact_manifest_size_is_deterministic() {
        for entry in entries().expect("catalog should parse") {
            for artifact in entry.artifacts {
                assert_eq!(
                    artifact.size_bytes,
                    artifact
                        .files
                        .iter()
                        .map(|file| file.size_bytes)
                        .sum::<u64>()
                );
            }
        }
    }
}
