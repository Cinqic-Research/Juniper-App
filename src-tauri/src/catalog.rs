use serde::{Deserialize, Serialize};

const CATALOG_JSON: &str = include_str!("../../config/models/catalog.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogArtifactFile {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    // Optional fields are omitted, never `null`: the frontend catalog validator
    // accepts absent fields but rejects null, which failed the Models refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_profile: Option<String>,
    /// Capabilities established by qualification. Absent means unknown, and
    /// unknown is never treated as supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ArtifactCapabilities>,
    pub format: String,
    pub platforms: Vec<String>,
    pub architectures: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub source_revision: String,
    pub files: Vec<CatalogArtifactFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_revision: Option<String>,
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
    /// `required` when the model's license must be accepted in Juniper
    /// before download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_acceptance: Option<String>,
    /// Pinned model-license revision or recorded external-terms fingerprint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_terms_revision: Option<String>,
    /// Dated popularity evidence (Hugging Face 30-day downloads). Shown as
    /// "popular on Hugging Face"; it says nothing about quality or fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popularity: Option<serde_json::Value>,
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

/// Whether the bundled artifact metadata explicitly supports this device.
/// Unknown native architectures fail closed. Linux/Windows `desktop` entries
/// remain valid for older catalog records that used a shared desktop label.
pub fn supports_device(artifact: &CatalogArtifact, platform: &str, architecture: &str) -> bool {
    let platform_supported = artifact.platforms.iter().any(|supported| {
        supported == platform || (supported == "desktop" && matches!(platform, "linux" | "windows"))
    });
    let architectures: &[&str] = match architecture {
        "x86_64" => &["x86_64"],
        "aarch64" | "arm64" | "arm64-v8a" => &["aarch64", "arm64", "arm64-v8a"],
        _ => &[],
    };
    platform_supported
        && !architectures.is_empty()
        && artifact
            .architectures
            .iter()
            .any(|supported| architectures.contains(&supported.as_str()))
}

pub fn supports_current_device(artifact: &CatalogArtifact) -> bool {
    supports_device(artifact, std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(platforms: &[&str], architectures: &[&str]) -> CatalogArtifact {
        CatalogArtifact {
            id: "artifact".into(),
            runtime_id: "llama.cpp".into(),
            backend_profile: None,
            capabilities: None,
            format: "gguf".into(),
            platforms: platforms.iter().map(|value| (*value).into()).collect(),
            architectures: architectures.iter().map(|value| (*value).into()).collect(),
            quantization: None,
            size_bytes: 1,
            sha256: None,
            source_url: None,
            source_revision: "revision".into(),
            files: vec![],
            minimum_runtime_version: None,
            maturity: "stable".into(),
            qualification: "not-qualified".into(),
        }
    }

    #[test]
    fn artifact_support_requires_a_known_matching_platform_and_architecture() {
        let desktop = artifact(&["linux", "windows"], &["x86_64"]);
        assert!(supports_device(&desktop, "linux", "x86_64"));
        assert!(supports_device(&desktop, "windows", "x86_64"));
        assert!(!supports_device(&desktop, "android", "x86_64"));
        assert!(!supports_device(&desktop, "linux", "unknown"));
        assert!(!supports_device(&desktop, "linux", "aarch64"));

        let android = artifact(&["android"], &["arm64"]);
        assert!(supports_device(&android, "android", "aarch64"));
        assert!(supports_device(&android, "android", "arm64-v8a"));
        assert!(!supports_device(&android, "android", "armv7"));
    }

    #[test]
    fn bundled_catalog_is_artifact_centric_and_integrity_pinned() {
        let entries = entries().expect("catalog should parse");
        for entry in &entries {
            for artifact in &entry.artifacts {
                assert_eq!(artifact.runtime_id, "llama.cpp", "{}", artifact.id);
                for file in &artifact.files {
                    assert!(valid_hash(&file.sha256), "{}", artifact.id);
                    // A download URL resolves the exact pinned revision of
                    // the repository the entry names, never a moving branch.
                    if let Some(url) = &file.url {
                        assert!(
                            url.starts_with(&format!(
                                "{}/resolve/{}/",
                                entry.source_repository, artifact.source_revision
                            )),
                            "{} is not pinned: {url}",
                            artifact.id
                        );
                        assert!(url.ends_with(&file.path), "{}", artifact.id);
                    }
                }
                if artifact.files.len() > 1 {
                    let count = artifact.files.len();
                    for (index, file) in artifact.files.iter().enumerate() {
                        assert!(
                            file.path
                                .contains(&format!("-{:05}-of-{count:05}", index + 1)),
                            "{}: shards must be listed in order",
                            artifact.id
                        );
                    }
                    assert!(artifact.sha256.is_none(), "a bundle has no single digest");
                }
            }
            if entry.license_acceptance.is_some() {
                assert!(entry.license_url.starts_with("https://"), "{}", entry.id);
            }
            assert!(!entry.attribution.is_empty() && !entry.license.is_empty());
        }
    }

    #[test]
    fn the_dated_popularity_snapshot_covers_its_top_ten_eligible_releases() {
        let entries = entries().expect("catalog should parse");
        let mut ranks: Vec<u64> = entries
            .iter()
            .filter_map(|entry| entry.popularity.as_ref())
            .filter(|popularity| {
                popularity["snapshot"]
                    .as_str()
                    .is_some_and(|snapshot| snapshot.starts_with("2026-10-15"))
            })
            .filter_map(|popularity| popularity["eligibleRank"].as_u64())
            .collect();
        ranks.sort_unstable();
        ranks.dedup();
        assert_eq!(&ranks[..10], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
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

    #[test]
    fn absent_optional_fields_are_omitted_not_null() {
        // The frontend validator (src/lib/model-catalog.ts) accepts absent optional
        // fields but rejects null ones (url, sourceUrl, sha256, ...).
        let json = serde_json::to_string(&entries().expect("catalog")).expect("serialize");
        assert!(
            !json.contains(":null"),
            "catalog serializes a null optional field"
        );
    }

    #[test]
    fn native_catalog_serialization_preserves_required_license_revisions() {
        let entries = entries().expect("catalog should parse");
        let native_response = serde_json::to_value(entries).expect("catalog should serialize");
        let models = native_response
            .as_array()
            .expect("command returns a model array");
        for model in models {
            if model["licenseAcceptance"] == "required" {
                assert!(
                    model["licenseTermsRevision"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty()),
                    "{} loses its required license terms revision",
                    model["id"]
                );
            }
        }
        let gemma = models
            .iter()
            .find(|model| model["id"] == "gemma-3-1b-it")
            .expect("Gemma catalog entry exists");
        assert!(
            gemma["licenseTermsRevision"]
                .as_str()
                .unwrap()
                .contains("-sha256-")
        );
    }
}
