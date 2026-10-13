//! The Juniper host's tool decision for one model-requested call, assembled
//! from the App's own code.
//!
//! `app_extracted.rs` holds items copied verbatim from `providers.rs`
//! (`offered_tools`, `ToolGate`, `tool_gate`, `requires_per_call_approval`,
//! `permission_preview`, `bounded_preview`, `denied`,
//! `permission_grant_allows`, `memory_belongs_to_assistant`) and `behavior.rs`
//! (`neutralize_control_tokens`, `strip_tags`, `attachments_message`). The
//! shim types below carry exactly the fields those items read, under the App's
//! names, so the copied code compiles unchanged.
//!
//! `host_decide` reproduces the order of `host_tool_turn` and the stateless and
//! attachment arms of `execute_host_tool` (both async or tied to Tauri state,
//! so they cannot be called here). `drift.rs` checks those two bodies still
//! contain every step and message this file relies on, in this order.

use crate::tools;
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::HashSet;

pub struct ModelCapabilities {
    pub tools: String,
}

pub struct ModelProfile {
    pub capabilities: ModelCapabilities,
}

pub struct ProviderProfile {
    pub kind: String,
}

#[derive(Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub risk: String,
    pub enabled: bool,
    pub schema: Value,
}

#[derive(Clone)]
pub struct PermissionGrant {
    pub id: String,
    pub tool_name: String,
    pub scope: String,
    pub assistant_id: String,
    pub conversation_id: Option<String>,
}

#[derive(Clone)]
pub struct AttachmentContext {
    pub id: String,
    pub name: String,
    pub content: String,
    pub size_bytes: Option<u64>,
    pub content_type: Option<String>,
}

/// Always empty here: the harness never holds user memories or chats.
#[derive(Default)]
pub struct HostToolContext {
    pub memories: Vec<Value>,
    pub conversations: Vec<Value>,
}

pub struct NormalizedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

pub struct ChatRequest {
    pub assistant_id: String,
    pub conversation_id: String,
    pub private_chat: bool,
    pub provider: ProviderProfile,
    pub model: ModelProfile,
    pub tools: Vec<ToolDefinition>,
    pub permission_grants: Vec<PermissionGrant>,
    pub attachments: Vec<AttachmentContext>,
}

include!(concat!(env!("OUT_DIR"), "/app_extracted.rs"));

/// Mutable host state of one request's tool loop (`ToolSession` in the App).
#[derive(Default)]
pub struct ToolSession {
    pub host_context: HostToolContext,
    pub grants: HashSet<String>,
    pub call_ids: HashSet<String>,
}

/// The decision a user would make at the App's permission prompt, supplied by
/// the fixed host policy fixture instead of a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptDecision {
    AllowOnce,
    AllowChat,
    AllowAssistant,
    Deny,
    /// No answer within the App's 300-second prompt timeout.
    Timeout,
}

impl PromptDecision {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "allow-once" => Self::AllowOnce,
            "allow-chat" => Self::AllowChat,
            "allow-assistant" => Self::AllowAssistant,
            "deny" => Self::Deny,
            "timeout" => Self::Timeout,
            _ => return None,
        })
    }

    /// The string the App's UI sends back (`request_permission`'s result).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow-once",
            Self::AllowChat => "allow-chat",
            Self::AllowAssistant => "allow-assistant",
            Self::Deny => "deny",
            Self::Timeout => "timeout",
        }
    }
}

pub struct Outcome {
    /// `HostToolResult`, or `None` when the host ended the request instead.
    pub result: Option<Value>,
    /// Request-level error (`ProviderError` code and message) that ends the turn.
    pub terminated: Option<(&'static str, &'static str)>,
    pub gate: &'static str,
    pub permission_asked: bool,
    pub permission_decision: Option<&'static str>,
    pub preview: Option<String>,
    pub path: Vec<&'static str>,
}

/// Calls the harness refuses to run because their real execution reads or
/// changes user data or reports the real host; a policy may only deny them.
pub fn needs_user_or_host_data(name: &str) -> bool {
    matches!(
        name,
        "memory.list" | "memory.save" | "memory.delete" | "chat.search" | "system.info"
    )
}

/// One call through the host, in `host_tool_turn` order. `calls_this_round`
/// is the App's `index as u32 + 1`.
pub fn host_decide(
    request: &ChatRequest,
    session: &mut ToolSession,
    call: &NormalizedToolCall,
    round: u32,
    calls_this_round: u32,
    decide: impl Fn(&str) -> Option<PromptDecision>,
) -> Result<Outcome, String> {
    let mut outcome = Outcome {
        result: None,
        terminated: None,
        gate: "pre-gate",
        permission_asked: false,
        permission_decision: None,
        preview: None,
        path: vec!["duplicate-call-id"],
    };
    if !session.call_ids.insert(call.id.clone()) {
        outcome.result = Some(denied(
            call,
            "DUPLICATE_CALL_ID",
            "This tool call reused an earlier call ID and was not run.",
        ));
        return Ok(outcome);
    }
    outcome.path.push("loop-allowed");
    if !tools::loop_allowed(round, calls_this_round) {
        outcome.result = Some(denied(call, "TOOL_LOOP_LIMIT", "Tool loop limit reached."));
        return Ok(outcome);
    }
    outcome.path.push("tool-gate");
    match tool_gate(request, &call.name, &session.grants) {
        ToolGate::NotEnabled => {
            outcome.gate = "not-enabled";
            outcome.result = Some(denied(
                call,
                "TOOL_NOT_ENABLED",
                "This tool is not enabled for this request.",
            ));
            return Ok(outcome);
        }
        ToolGate::Allowed => outcome.gate = "allowed",
        ToolGate::NeedsPermission(tool) => {
            outcome.gate = "needs-permission";
            outcome.path.push("validate-before-prompt");
            if let Err(error) = tools::validate_call(&call.name, &call.arguments) {
                outcome.result = Some(tools::host_result(
                    &call.id,
                    &call.name,
                    "error",
                    None,
                    Some(json!({ "code": "INVALID_TOOL_ARGUMENT", "message": error.to_string() })),
                ));
                return Ok(outcome);
            }
            outcome.path.push("permission-preview");
            let preview = match permission_preview(request, &session.host_context, call) {
                Ok(preview) => preview,
                Err((code, message)) => {
                    outcome.result = Some(tools::host_result(
                        &call.id,
                        &call.name,
                        "error",
                        None,
                        Some(json!({ "code": code, "message": message })),
                    ));
                    return Ok(outcome);
                }
            };
            outcome.path.push("permission-prompt");
            outcome.permission_asked = true;
            outcome.preview = preview;
            let decision = decide(&call.name)
                .ok_or_else(|| format!("policy has no permission decision for {}", call.name))?;
            outcome.permission_decision = Some(decision.as_str());
            match decision {
                PromptDecision::AllowOnce => {}
                PromptDecision::AllowChat | PromptDecision::AllowAssistant
                    if !requires_per_call_approval(tool) =>
                {
                    session.grants.insert(call.name.clone());
                }
                PromptDecision::AllowChat | PromptDecision::AllowAssistant => {}
                PromptDecision::Timeout => {
                    outcome.terminated =
                        Some(("PERMISSION_TIMEOUT", "The permission request timed out."));
                    return Ok(outcome);
                }
                PromptDecision::Deny => {
                    outcome.result = Some(denied(
                        call,
                        "PERMISSION_DENIED",
                        "The user denied this host capability.",
                    ));
                    return Ok(outcome);
                }
            }
        }
    }
    outcome.path.push("execute");
    outcome.result = Some(execute_host_tool(request, call, round, calls_this_round)?);
    Ok(outcome)
}

/// The arms of the App's `execute_host_tool` that need no user data.
fn execute_host_tool(
    request: &ChatRequest,
    call: &NormalizedToolCall,
    round: u32,
    calls_this_round: u32,
) -> Result<Value, String> {
    if matches!(
        call.name.as_str(),
        "calculator.evaluate" | "datetime.current" | "unit.convert"
    ) {
        return Ok(tools::execute_call(
            &call.id,
            &call.name,
            &call.arguments,
            round,
            calls_this_round,
        ));
    }
    if let Err(error) = tools::validate_call(&call.name, &call.arguments) {
        return Ok(tools::host_result(
            &call.id,
            &call.name,
            "error",
            None,
            Some(json!({
                "code": "INVALID_TOOL_ARGUMENT",
                "message": error.to_string()
            })),
        ));
    }
    if needs_user_or_host_data(&call.name) {
        return Err(format!(
            "{} would read or change user data or report the real host; the harness never executes it",
            call.name
        ));
    }
    let id = call.arguments["attachmentId"].as_str().unwrap_or_default();
    let attachment = request
        .attachments
        .iter()
        .find(|attachment| attachment.id == id);
    let result = match call.name.as_str() {
        "file.read" => attachment
            .map(|attachment| {
                json!({
                    "attachmentId": attachment.id,
                    "name": attachment.name,
                    "content": attachment.content
                })
            })
            .ok_or((
                "ATTACHMENT_NOT_GRANTED",
                "That attachment was not granted to this request.",
            )),
        "file.metadata" => attachment
            .map(|attachment| {
                json!({
                    "attachmentId": attachment.id,
                    "name": attachment.name,
                    "sizeBytes": attachment.size_bytes.unwrap_or(attachment.content.len() as u64),
                    "contentType": attachment.content_type.as_deref().unwrap_or("text/plain")
                })
            })
            .ok_or((
                "ATTACHMENT_NOT_GRANTED",
                "That attachment was not granted to this request.",
            )),
        _ => Err(("UNKNOWN_TOOL", "This host tool is not available.")),
    };
    Ok(match result {
        Ok(value) => match serde_json::to_vec(&value) {
            Ok(payload) if payload.len() <= tools::MAX_PAYLOAD_BYTES => {
                tools::host_result(&call.id, &call.name, "success", Some(value), None)
            }
            _ => tools::host_result(
                &call.id,
                &call.name,
                "error",
                None,
                Some(json!({
                    "code": "TOOL_RESULT_TOO_LARGE",
                    "message": "The host tool result exceeded the runtime limit."
                })),
            ),
        },
        Err((code, message)) => tools::host_result(
            &call.id,
            &call.name,
            "error",
            None,
            Some(json!({ "code": code, "message": message })),
        ),
    })
}

/// The `<|tool|>`/`role: tool` content the App sends the model for a result.
pub fn tool_message(result: &Value) -> String {
    neutralize_control_tokens(&result.to_string()).into_owned()
}

/// The App's user-role attachments message for `attachments`.
pub fn render_attachments(attachments: &[AttachmentContext]) -> Option<Value> {
    attachments_message(attachments)
}
