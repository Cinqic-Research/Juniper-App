//! The fixed host policy fixture: which tools a request enables and how the
//! permission prompt is answered. In the App a person answers the prompt; here
//! the fixture does, and every trace records that the decision came from it.

use crate::app::{
    AttachmentContext, ChatRequest, ModelCapabilities, ModelProfile, PermissionGrant,
    PromptDecision, ProviderProfile, ToolDefinition, needs_user_or_host_data,
};
use crate::tools;
use serde::Deserialize;
use std::collections::BTreeMap;

const SCHEMA: &str = "j1.host-policy.v1";
/// Stands for the session (conversation) ID in a fixture's standing grants.
const SESSION: &str = "$session";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema: String,
    policy_set: String,
    assistant_id: String,
    policies: BTreeMap<String, Policy>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    id: String,
    tool_name: String,
    scope: String,
    conversation_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    description: String,
    provider_kind: String,
    model_tools: String,
    #[serde(default)]
    private_chat: bool,
    enabled_tools: Vec<String>,
    #[serde(default)]
    decisions: BTreeMap<String, String>,
    #[serde(default)]
    standing_grants: Vec<Grant>,
}

pub struct PolicySet {
    fixture: Fixture,
}

impl PolicySet {
    pub fn parse(bytes: &[u8], catalog: &[ToolDefinition]) -> Result<Self, String> {
        let fixture: Fixture =
            serde_json::from_slice(bytes).map_err(|error| format!("policy fixture: {error}"))?;
        if fixture.schema != SCHEMA {
            return Err(format!("policy fixture schema must be {SCHEMA}"));
        }
        for (id, policy) in &fixture.policies {
            if policy.description.trim().is_empty() {
                return Err(format!("policy {id}: description is empty"));
            }
            for name in policy.enabled_tools.iter().chain(policy.decisions.keys()) {
                if !catalog.iter().any(|tool| &tool.name == name) {
                    return Err(format!("policy {id}: {name} is not an App tool"));
                }
            }
            for (name, decision) in &policy.decisions {
                let decision = PromptDecision::parse(decision)
                    .ok_or_else(|| format!("policy {id}: unknown decision {decision}"))?;
                if tools::host_risk(name) == Some("automatic-safe") {
                    return Err(format!(
                        "policy {id}: {name} is automatic-safe; the App never asks"
                    ));
                }
                if needs_user_or_host_data(name)
                    && !matches!(decision, PromptDecision::Deny | PromptDecision::Timeout)
                {
                    return Err(format!(
                        "policy {id}: {name} may only be denied or time out (it needs user data)"
                    ));
                }
            }
            for grant in &policy.standing_grants {
                if needs_user_or_host_data(&grant.tool_name) {
                    return Err(format!(
                        "policy {id}: no standing grant for {} (it needs user data)",
                        grant.tool_name
                    ));
                }
            }
            if policy
                .enabled_tools
                .iter()
                .any(|name| name == "system.info")
            {
                return Err(format!(
                    "policy {id}: system.info reports the real host and is never enabled"
                ));
            }
        }
        Ok(Self { fixture })
    }

    pub fn set_id(&self) -> &str {
        &self.fixture.policy_set
    }

    pub fn ids(&self) -> Vec<&str> {
        self.fixture.policies.keys().map(String::as_str).collect()
    }

    pub fn decision(&self, policy: &str, tool: &str) -> Option<PromptDecision> {
        self.fixture
            .policies
            .get(policy)?
            .decisions
            .get(tool)
            .and_then(|decision| PromptDecision::parse(decision))
    }

    /// The `ChatRequest` fields the host's tool logic reads, for one session.
    pub fn request(
        &self,
        policy_id: &str,
        session: &str,
        catalog: &[ToolDefinition],
        attachments: Vec<AttachmentContext>,
    ) -> Result<ChatRequest, String> {
        let policy = self
            .fixture
            .policies
            .get(policy_id)
            .ok_or_else(|| format!("unknown policy {policy_id}"))?;
        let tools = catalog
            .iter()
            .map(|tool| ToolDefinition {
                enabled: policy.enabled_tools.contains(&tool.name),
                ..tool.clone()
            })
            .collect();
        let permission_grants = policy
            .standing_grants
            .iter()
            .map(|grant| PermissionGrant {
                id: grant.id.clone(),
                tool_name: grant.tool_name.clone(),
                scope: grant.scope.clone(),
                assistant_id: self.fixture.assistant_id.clone(),
                conversation_id: grant.conversation_id.as_deref().map(|id| {
                    if id == SESSION {
                        session.to_owned()
                    } else {
                        id.to_owned()
                    }
                }),
            })
            .collect();
        Ok(ChatRequest {
            assistant_id: self.fixture.assistant_id.clone(),
            conversation_id: session.to_owned(),
            private_chat: policy.private_chat,
            provider: ProviderProfile {
                kind: policy.provider_kind.clone(),
            },
            model: ModelProfile {
                capabilities: ModelCapabilities {
                    tools: policy.model_tools.clone(),
                },
            },
            tools,
            permission_grants,
            attachments,
        })
    }
}
