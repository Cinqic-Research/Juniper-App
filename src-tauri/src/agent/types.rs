//! Versioned contracts for agent profiles, projects, tasks, events, actions,
//! approvals, artifacts, sources, and grants. These shapes cross the IPC
//! boundary in camelCase and are persisted as JSON in the v5 tables.

use crate::providers::{TranscriptMessage, TranscriptToolCall};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const AGENT_PROFILE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Queued,
    Running,
    WaitingInput,
    WaitingApproval,
    Paused,
    Interrupted,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingInput => "waiting-input",
            Self::WaitingApproval => "waiting-approval",
            Self::Paused => "paused",
            Self::Interrupted => "interrupted",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "waiting-input" => Self::WaitingInput,
            "waiting-approval" => Self::WaitingApproval,
            "paused" => Self::Paused,
            "interrupted" => Self::Interrupted,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// The documented transition model (docs/architecture/agent-runtime.md).
    /// Completed and failed tasks reopen only through a follow-up, which
    /// keeps the earlier goal and transcript.
    pub fn can_become(self, next: Self) -> bool {
        use TaskStatus::*;
        matches!(
            (self, next),
            (Queued, Running | Paused | Cancelled)
                | (
                    Running,
                    Queued
                        | WaitingInput
                        | WaitingApproval
                        | Paused
                        | Interrupted
                        | Completed
                        | Failed
                        | Cancelled
                )
                | (
                    WaitingInput | WaitingApproval,
                    Queued | Paused | Cancelled | Failed
                )
                | (Paused | Interrupted, Queued | Cancelled)
                | (Completed | Failed, Queued)
        )
    }
}

/// Explicit per-task limits. A child's budget is carved out of its parent's
/// remaining budget, so delegation never enlarges the total.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub max_steps: u32,
    pub max_tool_calls: u32,
    pub max_tokens: u64,
    pub max_seconds: u64,
    pub max_artifacts: u32,
    pub max_children: u32,
    pub max_depth: u32,
    /// Retries allowed for each failure cause before the task fails.
    pub max_retries_per_cause: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_steps: 16,
            max_tool_calls: 48,
            max_tokens: 120_000,
            max_seconds: 30 * 60,
            max_artifacts: 10,
            max_children: 4,
            max_depth: 2,
            max_retries_per_cause: 2,
        }
    }
}

impl Budget {
    /// Clamps values from the interface or an import to the host's ceilings.
    pub fn bounded(self) -> Self {
        Self {
            max_steps: self.max_steps.clamp(1, 64),
            max_tool_calls: self.max_tool_calls.clamp(1, 200),
            max_tokens: self.max_tokens.clamp(1_000, 1_000_000),
            max_seconds: self.max_seconds.clamp(30, 4 * 60 * 60),
            max_artifacts: self.max_artifacts.min(50),
            max_children: self.max_children.min(8),
            max_depth: self.max_depth.min(2),
            max_retries_per_cause: self.max_retries_per_cause.min(5),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskUsage {
    pub steps: u32,
    pub tool_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// True when a provider reported no counts and Juniper estimated them
    /// from text length (about four characters per token).
    pub tokens_estimated: bool,
    pub active_ms: u64,
    pub artifacts: u32,
    pub children: u32,
    #[serde(default)]
    pub retries: std::collections::BTreeMap<String, u32>,
    /// Unix seconds before which a waiting-for-model task is not retried.
    #[serde(default)]
    pub not_before: Option<i64>,
}

impl TaskUsage {
    pub fn tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub text: String,
    /// `pending`, `in-progress`, or `done`.
    pub status: String,
}

/// What a task is waiting on between model turns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Wait {
    Approval {
        approval_id: String,
    },
    Input {
        question: String,
        answer: Option<String>,
    },
    Children {
        task_ids: Vec<String>,
    },
}

/// Tool calls from the latest model turn that have not all produced host
/// results yet. `next` is the first unresolved call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub calls: Vec<TranscriptToolCall>,
    pub next: usize,
    #[serde(default)]
    pub wait: Option<Wait>,
}

/// Everything needed to reproduce a run's configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskProvenance {
    pub assistant_id: String,
    pub agent_profile_revision: i64,
    pub model_profile_id: String,
    pub model_id: String,
    pub provider_kind: String,
    pub execution_location: String,
    pub catalog_id: Option<String>,
    pub artifact_id: Option<String>,
    pub runtime_identity: Option<Value>,
    pub skills: Vec<SkillRef>,
    pub project_id: Option<String>,
    pub project_revision: Option<i64>,
    pub autonomy: String,
    pub tools: Vec<String>,
    pub generation: Value,
    pub host_instructions_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillRef {
    pub id: String,
    pub version: String,
    pub digest: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    /// `answer` (text only), `deliverable` (artifacts produced), or `partial`.
    pub outcome: String,
    pub summary: String,
    pub artifact_ids: Vec<String>,
    pub source_ids: Vec<String>,
    /// Claims the model made that the host could not verify, such as a
    /// citation to a source the task never retrieved.
    pub unverified: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub parent_id: Option<String>,
    pub root_id: String,
    pub depth: u32,
    pub role: Option<String>,
    pub assistant_id: String,
    pub project_id: Option<String>,
    pub schedule_run_id: Option<String>,
    pub title: String,
    pub goal: String,
    pub status: TaskStatus,
    pub status_reason: Option<String>,
    pub current_action: Option<String>,
    pub plan: Vec<PlanItem>,
    pub budget: Budget,
    pub usage: TaskUsage,
    pub provenance: TaskProvenance,
    #[serde(skip_serializing)]
    pub transcript: Vec<TranscriptMessage>,
    pub pending: Option<Pending>,
    pub result: Option<TaskResult>,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: String,
    /// `status`, `plan`, `action`, `result`, `approval`, `input`, `steer`,
    /// `child`, `error`, `note`.
    pub kind: String,
    pub summary: String,
    pub detail: Option<Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAction {
    pub id: String,
    pub task_id: String,
    pub step: u32,
    pub call_id: String,
    pub tool: String,
    pub arguments: Value,
    pub binding: String,
    pub status: String,
    pub result: Option<Value>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    pub task_id: String,
    pub action_id: String,
    pub tool: String,
    pub capability: String,
    pub preview: String,
    pub binding: String,
    pub standing_allowed: bool,
    pub status: String,
    pub decision: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub id: String,
    pub assistant_id: String,
    pub project_id: Option<String>,
    pub capability: String,
    pub scope: Value,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: String,
    pub task_id: Option<String>,
    pub project_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub source_ids: Vec<String>,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub task_id: Option<String>,
    /// `web-result` (a search snippet), `web-page` (retrieved content),
    /// `project-file`, or `plugin`.
    pub kind: String,
    pub title: String,
    pub url: Option<String>,
    pub final_url: Option<String>,
    pub location: Option<String>,
    pub retrieved_at: String,
    pub content_sha256: Option<String>,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub schema_version: u32,
    pub assistant_id: String,
    #[serde(default)]
    pub preferred_language: Option<String>,
    /// `ask`, `project`, or `custom`.
    pub autonomy: String,
    pub enabled_tools: Vec<String>,
    #[serde(default)]
    pub enabled_skills: Vec<String>,
    #[serde(default)]
    pub project_ids: Vec<String>,
    #[serde(default)]
    pub model_profile_id: Option<String>,
    /// The user chose to run agent tasks on a model without measured tool
    /// evidence. The task view keeps saying it is not qualified.
    #[serde(default)]
    pub allow_unqualified_model: bool,
    #[serde(default)]
    pub budget: Budget,
    /// The assistant's profile text as the interface composed it (tone,
    /// personality, length). User data: it shapes style, grants nothing.
    #[serde(default)]
    pub composed_profile: String,
}

impl AgentProfile {
    pub fn default_for(assistant_id: &str) -> Self {
        Self {
            schema_version: AGENT_PROFILE_SCHEMA,
            assistant_id: assistant_id.to_owned(),
            preferred_language: None,
            autonomy: "ask".into(),
            enabled_tools: crate::agent::policy::DEFAULT_TOOLS
                .iter()
                .map(|tool| (*tool).to_owned())
                .collect(),
            enabled_skills: Vec::new(),
            project_ids: Vec::new(),
            model_profile_id: None,
            allow_unqualified_model: false,
            budget: Budget::default(),
            composed_profile: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub archived: bool,
    pub roots: Vec<ProjectRoot>,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRoot {
    pub id: String,
    pub project_id: String,
    pub label: String,
    /// The folder the user chose; shown so the grant is inspectable.
    pub path: String,
    pub access: String,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub id: i64,
    pub task_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub read: bool,
    pub created_at: String,
}

/// The task view: the task plus everything the interface shows with it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    pub task: Task,
    pub events: Vec<TaskEvent>,
    pub actions: Vec<TaskAction>,
    pub approvals: Vec<Approval>,
    pub artifacts: Vec<Artifact>,
    pub sources: Vec<Source>,
    pub children: Vec<Task>,
    /// The visible conversation: the goal, follow-ups, questions, and answers.
    pub messages: Vec<TranscriptMessage>,
}
