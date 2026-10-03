//! Language-model backends behind Juniper's provider loop.
//!
//! Everything the orchestrator needs to know about a specific model family
//! lives here: request shaping, reasoning control, protocol markers that must
//! never appear in an answer, and the qualified server profile. The generic
//! backend reproduces the provider-neutral behavior every other model gets.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::time::Duration;

const GPT_OSS_PROFILE_JSON: &str = include_str!("../../config/backends/gpt-oss-20b.json");
pub const GPT_OSS_TEMPLATE: &str = include_str!("../../config/backends/gpt-oss-harmony.v1.jinja");

/// Harmony special tokens. Text from the final channel containing one of these
/// means the runtime failed to parse the model's output, so the text is not an
/// answer and may expose analysis or a raw tool call.
const HARMONY_MARKERS: &[&str] = &[
    "<|start|>",
    "<|end|>",
    "<|message|>",
    "<|channel|>",
    "<|constrain|>",
    "<|call|>",
    "<|return|>",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendProfile {
    pub id: String,
    pub backend: String,
    pub qualification: String,
    pub model: ModelLineage,
    pub artifact: ArtifactIdentity,
    pub runtime: RuntimeRequirement,
    pub template: TemplateIdentity,
    pub server: ServerProfile,
    pub lifecycle: LifecycleProfile,
    pub generation: GenerationProfile,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLineage {
    pub name: String,
    pub developer: String,
    pub repository: String,
    pub revision: String,
    pub license: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactIdentity {
    pub id: String,
    pub quantization: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRequirement {
    pub engine: String,
    pub tag: String,
    pub commit: String,
    pub accelerator: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateIdentity {
    pub file: String,
    pub sha256: String,
    /// The template as llama-server reports it after loading. llama.cpp
    /// rewrites two channel-tag guards in GPT-OSS templates, so this differs
    /// from `sha256` but is fixed for a given template and runtime build.
    pub served_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProfile {
    pub ctx_size: u32,
    pub n_gpu_layers: u32,
    pub n_cpu_moe: u32,
    pub flash_attn: String,
    pub cache_type_k: String,
    pub cache_type_v: String,
    pub threads: u32,
    pub batch_size: u32,
    pub ubatch_size: u32,
    pub parallel: u32,
    pub cache_ram_mib: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleProfile {
    pub startup_timeout_seconds: u64,
    pub first_token_timeout_seconds: u64,
    pub warm_up: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationProfile {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub min_p: f32,
    pub default_max_tokens: u32,
    pub max_tokens_ceiling: u32,
}

impl BackendProfile {
    /// llama-server arguments for the qualified configuration. `--fit off`
    /// keeps a configuration that does not fit from being silently adjusted
    /// into an unqualified one; it fails at startup instead.
    pub fn server_args(&self) -> Vec<String> {
        let server = &self.server;
        [
            ("-c", server.ctx_size.to_string()),
            ("-ngl", server.n_gpu_layers.to_string()),
            ("--n-cpu-moe", server.n_cpu_moe.to_string()),
            ("-fa", server.flash_attn.clone()),
            ("-ctk", server.cache_type_k.clone()),
            ("-ctv", server.cache_type_v.clone()),
            ("-t", server.threads.to_string()),
            ("-b", server.batch_size.to_string()),
            ("-ub", server.ubatch_size.to_string()),
            ("--parallel", server.parallel.to_string()),
            ("--cache-ram", server.cache_ram_mib.to_string()),
            ("--fit", "off".to_owned()),
            ("--temp", self.generation.temperature.to_string()),
            ("--top-p", self.generation.top_p.to_string()),
        ]
        .into_iter()
        .flat_map(|(flag, value)| [flag.to_owned(), value])
        .chain(["--jinja".to_owned()])
        .collect()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn gpt_oss_profile() -> Result<BackendProfile, String> {
    let profile: BackendProfile = serde_json::from_str(GPT_OSS_PROFILE_JSON).map_err(|_| {
        "BACKEND_PROFILE_INVALID: The bundled GPT-OSS backend profile is malformed.".to_owned()
    })?;
    if profile.backend != "gpt-oss-harmony"
        || sha256_hex(GPT_OSS_TEMPLATE.as_bytes()) != profile.template.sha256
    {
        return Err(
            "TEMPLATE_IDENTITY_MISMATCH: The bundled Harmony template does not match its qualified hash."
                .into(),
        );
    }
    Ok(profile)
}

/// The qualified profile for a catalog artifact, if the artifact names one.
pub fn profile(name: &str) -> Result<BackendProfile, String> {
    let profile = gpt_oss_profile()?;
    if profile.id == name {
        Ok(profile)
    } else {
        Err(
            "BACKEND_PROFILE_UNKNOWN: This model names a backend profile Juniper does not have."
                .into(),
        )
    }
}

/// How one request is shaped for, and checked against, a specific backend.
#[derive(Debug, Clone)]
pub struct RequestPolicy {
    pub backend: Backend,
    /// How long the stream may stay silent. Prefill on a large local model can
    /// exceed the generic idle limit before the first byte arrives.
    pub idle_timeout: Duration,
    /// Bearer token for a Juniper-owned loopback server.
    pub loopback_key: Option<String>,
    /// PID and port that must still own the private listener before sending
    /// that token. Set only for host-managed local runtime requests.
    #[cfg(target_os = "linux")]
    pub loopback_owner: Option<(u32, u16)>,
    /// Effective context window, when the server reports a fixed one.
    pub context_window: Option<u32>,
    /// Host-verified description of the model, for the runtime section.
    pub lineage: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Backend {
    Generic,
    GptOss(Box<BackendProfile>),
}

impl RequestPolicy {
    pub fn generic(idle_timeout: Duration) -> Self {
        Self {
            backend: Backend::Generic,
            idle_timeout,
            loopback_key: None,
            #[cfg(target_os = "linux")]
            loopback_owner: None,
            context_window: None,
            lineage: None,
        }
    }

    pub fn id(&self) -> &str {
        match &self.backend {
            Backend::Generic => "generic",
            Backend::GptOss(profile) => &profile.id,
        }
    }

    /// Harmony renders one tool call per assistant message and infers the
    /// tool name for the result from the most recent call.
    pub fn single_tool_call(&self) -> bool {
        matches!(self.backend, Backend::GptOss(_))
    }

    /// Harmony requires the analysis that produced a pending tool call to be
    /// sent back with that call. Everywhere else reasoning never re-enters a request.
    pub fn returns_tool_call_reasoning(&self) -> bool {
        matches!(self.backend, Backend::GptOss(_))
    }

    /// llama-server reports `stop` for a completed qualified answer; a stream
    /// that ends any other way is not treated as a finished answer.
    pub fn requires_stop_finish(&self) -> bool {
        matches!(self.backend, Backend::GptOss(_))
    }

    pub fn protocol_markers(&self) -> &'static [&'static str] {
        match self.backend {
            Backend::Generic => &[],
            Backend::GptOss(_) => HARMONY_MARKERS,
        }
    }

    /// GPT-OSS always reasons; effort is low, medium, or high. "Off" maps to
    /// low rather than to an unsupported "none".
    pub fn reasoning_effort(&self, thinking: Option<&str>) -> Option<&'static str> {
        match self.backend {
            Backend::Generic => None,
            Backend::GptOss(_) => Some(match thinking {
                Some("off" | "low") => "low",
                Some("high") => "high",
                _ => "medium",
            }),
        }
    }

    /// Applies backend-owned request fields. Sampling is pinned to the
    /// qualified values and the output is always bounded.
    pub fn shape_body(&self, body: &mut Map<String, Value>, thinking: Option<&str>) {
        let Backend::GptOss(profile) = &self.backend else {
            return;
        };
        let generation = &profile.generation;
        let requested = body
            .get("max_tokens")
            .and_then(Value::as_u64)
            .map_or(generation.default_max_tokens, |value| value as u32);
        body.insert(
            "max_tokens".into(),
            json!(requested.clamp(1, generation.max_tokens_ceiling)),
        );
        body.insert("temperature".into(), json!(generation.temperature));
        body.insert("top_p".into(), json!(generation.top_p));
        body.insert("top_k".into(), json!(generation.top_k));
        body.insert("min_p".into(), json!(generation.min_p));
        if let Some(effort) = self.reasoning_effort(thinking) {
            body.insert("reasoning_effort".into(), json!(effort));
        }
        if body.contains_key("tools") {
            body.insert("parallel_tool_calls".into(), json!(false));
        }
    }
}

/// Fails an answer that carries protocol markers rather than plain text.
pub fn validate_answer(
    policy: &RequestPolicy,
    content: &str,
) -> Result<(), (&'static str, &'static str)> {
    if policy
        .protocol_markers()
        .iter()
        .any(|marker| content.contains(marker))
    {
        return Err((
            "MODEL_OUTPUT_INVALID",
            "The model's answer contained raw protocol text and was not accepted.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpt_oss() -> RequestPolicy {
        RequestPolicy {
            backend: Backend::GptOss(Box::new(gpt_oss_profile().expect("profile"))),
            idle_timeout: Duration::from_secs(1),
            loopback_key: None,
            #[cfg(target_os = "linux")]
            loopback_owner: None,
            context_window: Some(16_384),
            lineage: None,
        }
    }

    #[test]
    fn bundled_profile_matches_the_qualified_record() {
        let profile = gpt_oss_profile().expect("bundled profile should parse");
        assert_eq!(profile.model.repository, "openai/gpt-oss-20b");
        assert_eq!(
            profile.model.revision,
            "6cee5e81ee83917806bbde320786a8fb61efebee"
        );
        assert_eq!(
            profile.artifact.sha256,
            "9d7364f02d9952e158ab462629e72401bec844d2243cc3854b271bb35d33d23d"
        );
        assert_eq!(
            profile.runtime.commit,
            "748d4225b9016b17ce4bcfa69fdc2c39f473a965"
        );
        assert_eq!(
            sha256_hex(GPT_OSS_TEMPLATE.as_bytes()),
            "f9c0f3d3324b708722c56afc83f51b7120f394b59da548ff9bc1c82550628984"
        );
        assert_eq!(profile.server.ctx_size, 16_384);
        assert_eq!(profile.server.n_cpu_moe, 18);
        assert_eq!(profile.server.cache_ram_mib, 1024);
        assert_eq!(profile.server.parallel, 1);
    }

    #[test]
    fn server_arguments_disable_fitting_and_bound_the_prompt_cache() {
        let args = gpt_oss_profile().expect("profile").server_args();
        let pair = |flag: &str| {
            args.iter()
                .position(|arg| arg == flag)
                .and_then(|index| args.get(index + 1))
                .cloned()
        };
        assert_eq!(pair("--fit").as_deref(), Some("off"));
        assert_eq!(pair("--cache-ram").as_deref(), Some("1024"));
        assert_eq!(pair("-c").as_deref(), Some("16384"));
        assert_eq!(pair("--n-cpu-moe").as_deref(), Some("18"));
        assert!(args.iter().any(|arg| arg == "--jinja"));
    }

    #[test]
    fn reasoning_effort_never_sends_none() {
        let policy = gpt_oss();
        for (thinking, effort) in [
            (None, "medium"),
            (Some("auto"), "medium"),
            (Some("on"), "medium"),
            (Some("off"), "low"),
            (Some("low"), "low"),
            (Some("medium"), "medium"),
            (Some("high"), "high"),
        ] {
            assert_eq!(policy.reasoning_effort(thinking), Some(effort));
        }
        assert_eq!(
            RequestPolicy::generic(Duration::from_secs(1)).reasoning_effort(Some("off")),
            None
        );
    }

    #[test]
    fn gpt_oss_body_pins_sampling_bounds_output_and_disables_parallel_calls() {
        let policy = gpt_oss();
        let mut body = json!({ "max_tokens": 100_000, "temperature": 0.2, "tools": [] })
            .as_object()
            .cloned()
            .expect("object");
        policy.shape_body(&mut body, Some("off"));
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["temperature"], 1.0);
        assert_eq!(body["top_p"], 1.0);
        assert_eq!(body["reasoning_effort"], "low");
        assert_eq!(body["parallel_tool_calls"], false);

        let mut unbounded = Map::new();
        policy.shape_body(&mut unbounded, None);
        assert_eq!(unbounded["max_tokens"], 4096);
        assert!(!unbounded.contains_key("parallel_tool_calls"));
    }

    #[test]
    fn generic_body_is_left_to_the_provider_adapter() {
        let mut body = json!({ "temperature": 0.7 })
            .as_object()
            .cloned()
            .expect("object");
        RequestPolicy::generic(Duration::from_secs(1)).shape_body(&mut body, Some("high"));
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn harmony_markers_in_an_answer_are_rejected() {
        let policy = gpt_oss();
        assert!(validate_answer(&policy, "Plain answer with <b>html</b>.").is_ok());
        assert!(validate_answer(&policy, "<|channel|>analysis<|message|>secret").is_err());
        assert!(
            validate_answer(&RequestPolicy::generic(Duration::from_secs(1)), "<|end|>").is_ok()
        );
    }

    #[test]
    fn unknown_profiles_are_refused() {
        assert!(profile("gpt-oss-20b-mxfp4-flowbox.v1").is_ok());
        assert!(profile("something-else").is_err());
    }
}
