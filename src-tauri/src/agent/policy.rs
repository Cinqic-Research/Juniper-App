//! The agent's capability registry and the one place its permission
//! decisions are made. Risk comes from this table, never from a model, a
//! skill, a document, an imported profile, or a plugin's own annotations.

use super::types::{AgentProfile, Grant};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    /// Deterministic, local, no user data: never asks.
    Safe,
    /// Reads inside folders the user granted to the project, or the task's
    /// own records. The folder grant is the consent.
    ProjectRead,
    /// Changes files in a granted folder.
    ProjectWrite,
    /// Saved memory changes: approved one call at a time, always.
    MemoryWrite,
    /// Creates future automation (a schedule): approved one call at a time,
    /// with the concrete next run times shown.
    Automation,
    /// Sends a query or URL to the network.
    Network,
    /// Runs code in the sandbox.
    Process,
    /// A tool on a connected plugin; its effects are unknown to Juniper.
    Plugin,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub risk: Risk,
    pub schema: Value,
    /// Built in, or the plugin that provides it.
    pub source: String,
}

/// Tools a new agent starts with. Network, code, and plugin tools are
/// opt-in per agent.
pub const DEFAULT_TOOLS: &[&str] = &[
    "calculator.evaluate",
    "datetime.current",
    "unit.convert",
    "task.update_plan",
    "task.ask_user",
    "artifact.create",
    "artifact.read",
    "workspace.list",
    "workspace.read",
    "workspace.search",
    "workspace.write",
    "project.search",
    "skill.load",
    "memory.list",
    "schedule.create",
];

/// Tools only the host adds; they are part of how a task works rather than
/// capabilities a user enables.
pub const CONTROL_TOOLS: &[&str] = &["task.update_plan", "task.ask_user"];

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn spec(name: &str, display: &str, description: &str, risk: Risk, schema: Value) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        display_name: display.into(),
        description: description.into(),
        risk,
        schema,
        source: "built-in".into(),
    }
}

/// Every built-in agent tool.
pub fn builtin_tools() -> Vec<ToolSpec> {
    vec![
        spec(
            "calculator.evaluate",
            "Calculator",
            "Evaluate an arithmetic expression exactly (+ - * / ^ and parentheses).",
            Risk::Safe,
            object(
                json!({ "expression": { "type": "string", "maxLength": 256 } }),
                &["expression"],
            ),
        ),
        spec(
            "datetime.current",
            "Current date and time",
            "Get the current date, time, and the user's time zone.",
            Risk::Safe,
            object(json!({}), &[]),
        ),
        spec(
            "unit.convert",
            "Unit converter",
            "Convert a value between units of length, mass, temperature, speed, or data size.",
            Risk::Safe,
            object(
                json!({
                    "value": { "type": "number" },
                    "from": { "type": "string", "maxLength": 64 },
                    "to": { "type": "string", "maxLength": 64 }
                }),
                &["value", "from", "to"],
            ),
        ),
        spec(
            "task.update_plan",
            "Update plan",
            "Record or update the task's short checklist. Call it early, then mark items done as you go.",
            Risk::Safe,
            object(
                json!({
                    "items": {
                        "type": "array",
                        "maxItems": 12,
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": { "type": "string", "maxLength": 200 },
                                "status": { "type": "string", "enum": ["pending", "in-progress", "done"] }
                            },
                            "required": ["text", "status"],
                            "additionalProperties": false
                        }
                    }
                }),
                &["items"],
            ),
        ),
        spec(
            "task.ask_user",
            "Ask the user",
            "Ask the user one question when you need information only they have. The task waits for the answer.",
            Risk::Safe,
            object(
                json!({ "question": { "type": "string", "maxLength": 500 } }),
                &["question"],
            ),
        ),
        spec(
            "artifact.create",
            "Create a document",
            "Save a deliverable (report, draft, notes, data, or code) as a document the user can open and export. Use Markdown for documents. Cite sources by their ids, like [S1].",
            Risk::Safe,
            object(
                json!({
                    "title": { "type": "string", "maxLength": 120 },
                    "fileName": { "type": "string", "maxLength": 80, "description": "For example report.md, data.csv, notes.txt" },
                    "content": { "type": "string", "maxLength": 200000 },
                    "sourceIds": { "type": "array", "items": { "type": "string" }, "maxItems": 50 }
                }),
                &["title", "fileName", "content"],
            ),
        ),
        spec(
            "artifact.read",
            "Read a document",
            "Read a document this task or one of its subtasks created.",
            Risk::ProjectRead,
            object(
                json!({ "artifactId": { "type": "string", "maxLength": 128 } }),
                &["artifactId"],
            ),
        ),
        spec(
            "workspace.list",
            "List project files",
            "List files and folders in a folder granted to this project. Paths are relative to the granted folder.",
            Risk::ProjectRead,
            object(
                json!({
                    "rootId": { "type": "string", "maxLength": 128 },
                    "path": { "type": "string", "maxLength": 1024, "description": "Relative folder path; empty for the top" }
                }),
                &["rootId"],
            ),
        ),
        spec(
            "workspace.read",
            "Read a project file",
            "Read a text file in a granted project folder. Returns the content and its version for later edits.",
            Risk::ProjectRead,
            object(
                json!({
                    "rootId": { "type": "string", "maxLength": 128 },
                    "path": { "type": "string", "maxLength": 1024 },
                    "offset": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200000 }
                }),
                &["rootId", "path"],
            ),
        ),
        spec(
            "workspace.search",
            "Search project files",
            "Find text in files of a granted project folder (plain, case-insensitive match).",
            Risk::ProjectRead,
            object(
                json!({
                    "rootId": { "type": "string", "maxLength": 128 },
                    "query": { "type": "string", "maxLength": 200 },
                    "path": { "type": "string", "maxLength": 1024 }
                }),
                &["rootId", "query"],
            ),
        ),
        spec(
            "workspace.write",
            "Write a project file",
            "Create or replace a text file in a granted read-write project folder. To edit an existing file, pass the version from workspace.read; the write is refused if the file changed. When approval is required, Juniper shows the proposed diff before writing. Project autonomy or a standing grant can allow the write without an approval preview.",
            Risk::ProjectWrite,
            object(
                json!({
                    "rootId": { "type": "string", "maxLength": 128 },
                    "path": { "type": "string", "maxLength": 1024 },
                    "content": { "type": "string", "maxLength": 1000000 },
                    "expectedVersion": { "type": "string", "maxLength": 80, "description": "The version workspace.read returned, or \"absent\" for a new file" }
                }),
                &["rootId", "path", "content", "expectedVersion"],
            ),
        ),
        spec(
            "project.search",
            "Search project documents",
            "Search the project's indexed documents and return matching passages with their source ids.",
            Risk::ProjectRead,
            object(
                json!({ "query": { "type": "string", "maxLength": 200 } }),
                &["query"],
            ),
        ),
        spec(
            "skill.load",
            "Load a skill",
            "Load the full procedure of one of the enabled skills listed in your instructions.",
            Risk::Safe,
            object(
                json!({ "skillId": { "type": "string", "maxLength": 128 } }),
                &["skillId"],
            ),
        ),
        spec(
            "memory.list",
            "Read saved memories",
            "Read the notes the user saved for this agent.",
            Risk::ProjectRead,
            object(json!({}), &[]),
        ),
        spec(
            "memory.save",
            "Save a memory",
            "Propose a short note to remember for future tasks. The user approves the exact text.",
            Risk::MemoryWrite,
            object(
                json!({ "content": { "type": "string", "maxLength": 1000 } }),
                &["content"],
            ),
        ),
        spec(
            "schedule.create",
            "Create a schedule",
            "Propose a one-time or recurring schedule for this agent. The user sees the exact next run times and approves it. Times are local to the time zone in your instructions.",
            Risk::Automation,
            object(
                json!({
                    "title": { "type": "string", "maxLength": 100 },
                    "prompt": { "type": "string", "maxLength": 4000, "description": "What the agent should do at each run" },
                    "kind": { "type": "string", "enum": ["once", "daily", "weekly", "monthly", "interval"] },
                    "at": { "type": "string", "maxLength": 16, "description": "For once: local date and time, YYYY-MM-DDTHH:MM" },
                    "time": { "type": "string", "maxLength": 5, "description": "For daily, weekly, monthly: local HH:MM (24-hour)" },
                    "days": { "type": "array", "items": { "type": "integer", "minimum": 1, "maximum": 7 }, "maxItems": 7, "description": "For weekly: 1 = Monday … 7 = Sunday" },
                    "day": { "type": "integer", "minimum": 1, "maximum": 31, "description": "For monthly" },
                    "minutes": { "type": "integer", "minimum": 15, "maximum": 10080, "description": "For interval" }
                }),
                &["title", "prompt", "kind"],
            ),
        ),
        spec(
            "web.search",
            "Web search",
            "Search the web. Results are short snippets with source ids; use web.fetch to read a page before relying on it.",
            Risk::Network,
            object(
                json!({ "query": { "type": "string", "maxLength": 300 } }),
                &["query"],
            ),
        ),
        spec(
            "web.fetch",
            "Read a web page",
            "Retrieve a public web page as text and record it as a source.",
            Risk::Network,
            object(
                json!({ "url": { "type": "string", "maxLength": 2048 } }),
                &["url"],
            ),
        ),
        spec(
            "code.python",
            "Run Python",
            "Run a Python 3 script in an isolated sandbox with no network and no access to your files. Files written to the current folder are returned as outputs.",
            Risk::Process,
            object(
                json!({
                    "code": { "type": "string", "maxLength": 20000 },
                    "projectRootId": { "type": "string", "maxLength": 128, "description": "Optional: mount this granted folder read-only at /project" }
                }),
                &["code"],
            ),
        ),
        spec(
            "code.run",
            "Run a project check",
            "Run a test or check command (python3 or node with arguments, no shell) inside the sandbox, with the project folder mounted read-only.",
            Risk::Process,
            object(
                json!({
                    "rootId": { "type": "string", "maxLength": 128 },
                    "program": { "type": "string", "enum": ["python3", "node"] },
                    "args": { "type": "array", "items": { "type": "string", "maxLength": 512 }, "maxItems": 32 },
                    "path": { "type": "string", "maxLength": 1024, "description": "Working folder relative to the root" }
                }),
                &["rootId", "program", "args"],
            ),
        ),
        spec(
            "agent.delegate",
            "Delegate a subtask",
            "Hand an independent part of the work to a helper with a role (researcher, file-analyst, implementer, reviewer). Only use it for separable work; it costs budget. The helper's report comes back as this tool's result.",
            Risk::Safe,
            object(
                json!({
                    "role": { "type": "string", "enum": ["researcher", "file-analyst", "implementer", "reviewer"] },
                    "objective": { "type": "string", "maxLength": 2000 },
                    "context": { "type": "string", "maxLength": 8000, "description": "Only the material the helper needs" },
                    "artifactIds": { "type": "array", "items": { "type": "string" }, "maxItems": 10, "description": "Documents the helper may read (a reviewer needs the work to review)" }
                }),
                &["role", "objective"],
            ),
        ),
    ]
}

/// Tools each delegated role may use, before intersecting with the parent's.
pub fn role_tools(role: &str) -> &'static [&'static str] {
    match role {
        "researcher" => &[
            "web.search",
            "web.fetch",
            "project.search",
            "workspace.read",
            "workspace.search",
            "workspace.list",
            "artifact.create",
            "calculator.evaluate",
            "datetime.current",
        ],
        "file-analyst" => &[
            "workspace.list",
            "workspace.read",
            "workspace.search",
            "project.search",
            "artifact.create",
            "artifact.read",
            "calculator.evaluate",
        ],
        "implementer" => &[
            "workspace.list",
            "workspace.read",
            "workspace.search",
            "workspace.write",
            "artifact.create",
            "artifact.read",
            "code.python",
            "code.run",
        ],
        // A reviewer inspects; it cannot change what it reviews.
        "reviewer" => &[
            "artifact.read",
            "workspace.list",
            "workspace.read",
            "workspace.search",
            "code.run",
        ],
        _ => &[],
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Allow,
    /// Ask the user. `standing` says whether a standing grant may be offered.
    Ask {
        standing: bool,
    },
    Deny {
        code: &'static str,
        message: String,
    },
}

/// Inputs to a decision, all resolved by the host.
pub struct DecisionContext<'a> {
    pub profile: &'a AgentProfile,
    pub assistant_id: &'a str,
    pub project_id: Option<&'a str>,
    pub grants: &'a [Grant],
    /// The scope a grant must match for this call (for example a plugin's
    /// tool-set digest), beyond the capability name.
    pub scope: Value,
    pub offered: bool,
}

/// Whether an unexpired, unrevoked grant covers this capability for this
/// agent and project with exactly this scope.
pub fn granted(context: &DecisionContext<'_>, capability: &str) -> bool {
    let now = chrono::Utc::now();
    context.grants.iter().any(|grant| {
        grant.revoked_at.is_none()
            && grant.capability == capability
            && grant.assistant_id == context.assistant_id
            && (grant.project_id.is_none() || grant.project_id.as_deref() == context.project_id)
            && match grant.expires_at.as_deref() {
                None => true,
                Some(value) => {
                    chrono::DateTime::parse_from_rfc3339(value).is_ok_and(|expires| expires > now)
                }
            }
            && scope_covers(capability, &grant.scope, &context.scope)
    })
}

fn scope_covers(capability: &str, granted: &Value, requested: &Value) -> bool {
    // Network destinations and plugin identities are security boundaries. A
    // legacy empty/subset grant must not become a wildcard after endpoints or
    // executable identities are added to the requested scope.
    if capability.starts_with("mcp:") || matches!(capability, "web.search" | "web.fetch") {
        return requested
            .as_object()
            .is_some_and(|fields| !fields.is_empty() && granted == requested);
    }
    match granted.as_object() {
        None => true,
        Some(fields) if fields.is_empty() => true,
        Some(fields) => fields
            .iter()
            .all(|(key, value)| requested.get(key) == Some(value)),
    }
}

/// The policy decision for one call.
pub fn decide(risk: Risk, capability: &str, context: &DecisionContext<'_>) -> Decision {
    if !context.offered {
        return Decision::Deny {
            code: "TOOL_NOT_ENABLED",
            message: "This tool is not enabled for this agent and task.".into(),
        };
    }
    match risk {
        Risk::Safe | Risk::ProjectRead => Decision::Allow,
        // Saved memory is approved one call at a time, exactly as in chat.
        Risk::MemoryWrite | Risk::Automation => Decision::Ask { standing: false },
        Risk::ProjectWrite | Risk::Network | Risk::Process | Risk::Plugin => {
            if granted(context, capability) {
                return Decision::Allow;
            }
            // "Within projects" lets writes inside the task's own granted
            // folders proceed without an approval prompt. The tool action is
            // recorded in task activity, but the activity entry is not a diff
            // preview. Network, code, and plugins ask unless an applicable
            // standing permission covers that exact capability.
            let preset_allows = context.profile.autonomy == "project"
                && matches!(risk, Risk::ProjectWrite)
                && context.project_id.is_some();
            if preset_allows {
                Decision::Allow
            } else {
                Decision::Ask { standing: true }
            }
        }
    }
}

/// What a preset permits, shown before the user selects it.
pub fn preset_description(preset: &str) -> &'static str {
    match preset {
        "project" => {
            "Reads and changes files in the task’s granted read-write project folders without asking. Project writes are recorded in task activity; Juniper shows a diff when it asks for approval. Asks before a web request, code run, or plugin call unless a matching standing permission exists; schedule changes and memory changes always ask."
        }
        "custom" => {
            "Uses only the standing permissions you granted. Asks before anything else that writes, runs code, uses the network, or uses a plugin. Schedule changes and memory changes always ask."
        }
        _ => {
            "Reads granted project folders and uses safe local tools. Asks before every file change, web request, code run, or plugin call unless a matching standing permission exists; schedule changes and memory changes always ask."
        }
    }
}

/// The schema the model sees: the host's own, without string length
/// bounds. llama.cpp turns `maxLength` into a grammar repetition and refuses
/// large ones ("failed to parse grammar"); the host still enforces every
/// bound when it checks the arguments.
pub fn model_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "maxLength")
                .map(|(key, value)| (key.clone(), model_schema(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(model_schema).collect()),
        other => other.clone(),
    }
}

/// Binds an approval to the exact call and to the state it was previewed
/// against, so a change in either makes the approval stale.
pub fn binding(tool: &str, arguments: &Value, target: &Value, identity: &Value) -> String {
    let canonical = json!({
        "tool": tool,
        "arguments": canonical_json(arguments),
        "target": canonical_json(target),
        "identity": canonical_json(identity),
    });
    format!("{:x}", Sha256::digest(canonical.to_string().as_bytes()))
}

/// JSON with object keys sorted, so equal values hash equally.
pub fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical_json(&map[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::AgentProfile;

    fn grant(capability: &str, scope: Value, revoked: bool) -> Grant {
        Grant {
            id: "grant".into(),
            assistant_id: "agent-1".into(),
            project_id: Some("project-1".into()),
            capability: capability.into(),
            scope,
            created_at: "2026-10-15T00:00:00Z".into(),
            expires_at: None,
            revoked_at: revoked.then(|| "2026-10-15T01:00:00Z".into()),
        }
    }

    fn context<'a>(
        profile: &'a AgentProfile,
        grants: &'a [Grant],
        scope: Value,
    ) -> DecisionContext<'a> {
        DecisionContext {
            profile,
            assistant_id: "agent-1",
            project_id: Some("project-1"),
            grants,
            scope,
            offered: true,
        }
    }

    #[test]
    fn safe_and_granted_reads_never_ask_and_unoffered_tools_are_denied() {
        let profile = AgentProfile::default_for("agent-1");
        let ctx = context(&profile, &[], json!({}));
        assert_eq!(
            decide(Risk::Safe, "calculator.evaluate", &ctx),
            Decision::Allow
        );
        assert_eq!(
            decide(Risk::ProjectRead, "workspace.read", &ctx),
            Decision::Allow
        );
        let mut hidden = context(&profile, &[], json!({}));
        hidden.offered = false;
        assert!(matches!(
            decide(Risk::Safe, "x", &hidden),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn writes_ask_until_granted_and_revocation_takes_effect() {
        let profile = AgentProfile::default_for("agent-1");
        let ctx = context(&profile, &[], json!({}));
        assert_eq!(
            decide(Risk::ProjectWrite, "workspace.write", &ctx),
            Decision::Ask { standing: true }
        );
        let grants = [grant("workspace.write", json!({}), false)];
        let ctx = context(&profile, &grants, json!({}));
        assert_eq!(
            decide(Risk::ProjectWrite, "workspace.write", &ctx),
            Decision::Allow
        );
        let revoked = [grant("workspace.write", json!({}), true)];
        let ctx = context(&profile, &revoked, json!({}));
        assert_eq!(
            decide(Risk::ProjectWrite, "workspace.write", &ctx),
            Decision::Ask { standing: true }
        );
    }

    #[test]
    fn memory_writes_are_always_per_call_even_with_a_grant() {
        let profile = AgentProfile::default_for("agent-1");
        let grants = [grant("memory.save", json!({}), false)];
        let ctx = context(&profile, &grants, json!({}));
        assert_eq!(
            decide(Risk::MemoryWrite, "memory.save", &ctx),
            Decision::Ask { standing: false }
        );
    }

    #[test]
    fn malformed_grant_expiration_fails_closed() {
        let profile = AgentProfile::default_for("agent-1");
        let mut malformed = grant("web", json!({}), false);
        malformed.expires_at = Some("not-a-timestamp".into());
        let grants = [malformed];
        let ctx = context(&profile, &grants, json!({}));
        assert_eq!(
            decide(Risk::Network, "web", &ctx),
            Decision::Ask { standing: true }
        );
    }

    #[test]
    fn a_plugin_grant_does_not_survive_a_changed_tool_set() {
        let profile = AgentProfile::default_for("agent-1");
        let grants = [grant(
            "mcp:server-1:search",
            json!({ "toolsDigest": "aaa" }),
            false,
        )];
        let same = context(&profile, &grants, json!({ "toolsDigest": "aaa" }));
        assert_eq!(
            decide(Risk::Plugin, "mcp:server-1:search", &same),
            Decision::Allow
        );
        let changed = context(&profile, &grants, json!({ "toolsDigest": "bbb" }));
        assert_eq!(
            decide(Risk::Plugin, "mcp:server-1:search", &changed),
            Decision::Ask { standing: true }
        );
    }

    #[test]
    fn legacy_or_changed_destination_grants_do_not_authorize_plugins_or_web() {
        let profile = AgentProfile::default_for("agent-1");
        let legacy_plugin = grant(
            "mcp:server-1:search",
            json!({ "server": "server-1", "toolsDigest": "aaa" }),
            false,
        );
        let legacy_plugin_grants = [legacy_plugin];
        let changed_plugin = context(
            &profile,
            &legacy_plugin_grants,
            json!({ "server": "server-1", "toolsDigest": "aaa", "targetDigest": "new-binary" }),
        );
        assert_eq!(
            decide(Risk::Plugin, "mcp:server-1:search", &changed_plugin),
            Decision::Ask { standing: true }
        );

        let wildcard_web = grant("web.search", json!({}), false);
        let wildcard_web_grants = [wildcard_web];
        let changed_search = context(
            &profile,
            &wildcard_web_grants,
            json!({ "destination": { "provider": "searxng", "baseUrl": "https://new.example/" } }),
        );
        assert_eq!(
            decide(Risk::Network, "web.search", &changed_search),
            Decision::Ask { standing: true }
        );

        let old_page = grant("web.fetch", json!({ "url": "https://old.example/" }), false);
        let old_page_grants = [old_page];
        let changed_page = context(
            &profile,
            &old_page_grants,
            json!({ "url": "https://new.example/" }),
        );
        assert_eq!(
            decide(Risk::Network, "web.fetch", &changed_page),
            Decision::Ask { standing: true }
        );
    }

    #[test]
    fn a_grant_for_another_project_or_agent_does_not_apply() {
        let profile = AgentProfile::default_for("agent-1");
        let mut other_project = grant("web", json!({}), false);
        other_project.project_id = Some("project-2".into());
        let mut other_agent = grant("web", json!({}), false);
        other_agent.assistant_id = "agent-2".into();
        let grants = [other_project, other_agent];
        let ctx = context(&profile, &grants, json!({}));
        assert_eq!(
            decide(Risk::Network, "web", &ctx),
            Decision::Ask { standing: true }
        );
    }

    #[test]
    fn bindings_change_with_arguments_or_target_state_but_not_key_order() {
        let identity = json!({
            "taskId": "task-1",
            "rootId": "root-1",
            "assistantId": "agent-1",
            "projectId": "project-1",
            "actionId": "action-1",
        });
        let a = binding(
            "workspace.write",
            &json!({"path": "a", "content": "x"}),
            &json!({"version": "1"}),
            &identity,
        );
        let reordered = binding(
            "workspace.write",
            &json!({"content": "x", "path": "a"}),
            &json!({"version": "1"}),
            &identity,
        );
        let changed_target = binding(
            "workspace.write",
            &json!({"path": "a", "content": "x"}),
            &json!({"version": "2"}),
            &identity,
        );
        let changed_args = binding(
            "workspace.write",
            &json!({"path": "a", "content": "y"}),
            &json!({"version": "1"}),
            &identity,
        );
        assert_eq!(a, reordered);
        assert_ne!(a, changed_target);
        assert_ne!(a, changed_args);
        let other_task = json!({
            "taskId": "task-2",
            "rootId": "root-1",
            "assistantId": "agent-1",
            "projectId": "project-1",
            "actionId": "action-1",
        });
        let other_project = json!({
            "taskId": "task-1",
            "rootId": "root-1",
            "assistantId": "agent-1",
            "projectId": "project-2",
            "actionId": "action-1",
        });
        assert_ne!(
            a,
            binding(
                "workspace.write",
                &json!({"path": "a", "content": "x"}),
                &json!({"version": "1"}),
                &other_task,
            )
        );
        assert_ne!(
            a,
            binding(
                "workspace.write",
                &json!({"path": "a", "content": "x"}),
                &json!({"version": "1"}),
                &other_project,
            )
        );
    }

    #[test]
    fn the_project_preset_allows_project_writes_but_never_the_network() {
        let mut profile = AgentProfile::default_for("agent-1");
        profile.autonomy = "project".into();
        let ctx = context(&profile, &[], json!({}));
        assert_eq!(
            decide(Risk::ProjectWrite, "workspace.write", &ctx),
            Decision::Allow
        );
        assert_eq!(
            decide(Risk::Network, "web.fetch", &ctx),
            Decision::Ask { standing: true }
        );
        assert_eq!(
            decide(Risk::Process, "code.python", &ctx),
            Decision::Ask { standing: true }
        );
        assert_eq!(
            decide(Risk::Automation, "schedule.create", &ctx),
            Decision::Ask { standing: false }
        );
        let mut outside = context(&profile, &[], json!({}));
        outside.project_id = None;
        assert_eq!(
            decide(Risk::ProjectWrite, "workspace.write", &outside),
            Decision::Ask { standing: true }
        );
    }

    #[test]
    fn project_preset_description_does_not_promise_a_diff_for_every_write() {
        let description = preset_description("project");
        assert!(description.contains("recorded in task activity"));
        assert!(description.contains("shows a diff when it asks for approval"));
        assert!(description.contains("unless a matching standing permission exists"));
        assert!(description.contains("memory changes always ask"));
        assert!(description.contains("schedule changes"));
        assert!(!description.contains("each change is listed with its diff"));
    }

    #[test]
    fn ask_first_description_accounts_for_existing_standing_permissions() {
        let description = preset_description("ask");
        assert!(description.contains("unless a matching standing permission exists"));
        assert!(description.contains("memory changes always ask"));
        assert!(description.contains("schedule changes"));
    }

    #[test]
    fn the_model_sees_schemas_without_length_bounds_but_otherwise_unchanged() {
        let spec = builtin_tools()
            .into_iter()
            .find(|tool| tool.name == "agent.delegate")
            .unwrap();
        let shown = model_schema(&spec.schema);
        assert!(!shown.to_string().contains("maxLength"));
        assert_eq!(
            shown["properties"]["role"]["enum"],
            spec.schema["properties"]["role"]["enum"]
        );
        assert_eq!(shown["required"], spec.schema["required"]);
    }

    #[test]
    fn reviewers_cannot_write_what_they_review() {
        assert!(!role_tools("reviewer").contains(&"workspace.write"));
        assert!(!role_tools("reviewer").contains(&"artifact.create"));
        assert!(role_tools("unknown").is_empty());
    }
}
