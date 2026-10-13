//! `juniper-tool-trace`: records what the Juniper host does with tool calls.
//!
//! Reads one JSON request per line on stdin and writes exactly one JSON line
//! per request on stdout (after a first environment line), so a driver can run
//! it interactively. It never reads user data, never opens a network
//! connection and never runs a process. See README.md for the line protocol.
//!
//! ```text
//! juniper-tool-trace --policy fixtures/host_policy_v1.json < requests.jsonl > traces.jsonl
//! ```

#[allow(dead_code)]
#[path = "../../../src-tauri/src/tools.rs"]
mod tools;

#[allow(dead_code)]
mod app;
mod drift;
mod policy;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const HARNESS_SOURCES: &[(&str, &str)] = &[
    ("build.rs", include_str!("../build.rs")),
    ("src/main.rs", include_str!("main.rs")),
    ("src/app.rs", include_str!("app.rs")),
    ("src/drift.rs", include_str!("drift.rs")),
    ("src/policy.rs", include_str!("policy.rs")),
];
const CATALOG_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/catalog.json"));
const ENV_SCHEMA: &str = "j1.host-trace-env.v1";
const TRACE_SCHEMA: &str = "j1.host-trace.v1";

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// The App's `builtinTools`, with the risk from the host's own table.
fn catalog() -> Result<Vec<app::ToolDefinition>, String> {
    let entries: Vec<Value> =
        serde_json::from_str(CATALOG_JSON).map_err(|error| format!("catalog: {error}"))?;
    entries
        .into_iter()
        .map(|entry| {
            let name = entry["name"].as_str().ok_or("catalog entry without name")?;
            let risk = tools::host_risk(name)
                .ok_or_else(|| format!("catalog tool {name} is unknown to the host"))?;
            if entry["risk"].as_str() != Some(risk) {
                return Err(format!("catalog risk of {name} differs from host_risk"));
            }
            Ok(app::ToolDefinition {
                name: name.to_owned(),
                description: entry["description"].as_str().unwrap_or_default().to_owned(),
                risk: risk.to_owned(),
                enabled: false,
                schema: entry["schema"].clone(),
            })
        })
        .collect()
}

fn os_release() -> Value {
    let pretty = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|value| value.trim_matches('"').to_owned())
        });
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|text| text.trim().to_owned());
    json!({
        "family": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "pretty_name": pretty,
        "kernel": kernel,
    })
}

fn app_sources() -> (Value, bool) {
    let mut sources = Map::new();
    let mut clean = true;
    for entry in env!("JUNIPER_APP_BLOBS").split(';') {
        let Some((path, blobs)) = entry.split_once('=') else {
            clean = false;
            continue;
        };
        let (at_commit, in_tree) = blobs.split_once(':').unwrap_or(("", ""));
        let matches = !at_commit.is_empty() && at_commit == in_tree;
        clean &= matches;
        sources.insert(
            path.to_owned(),
            json!({ "blob_at_commit": at_commit, "blob_in_tree": in_tree, "matches_commit": matches }),
        );
    }
    let embedded = [
        ("src-tauri/src/tools.rs", drift::TOOLS_RS),
        ("src-tauri/src/providers.rs", drift::PROVIDERS_RS),
        ("src-tauri/src/behavior.rs", drift::BEHAVIOR_RS),
        ("src/lib/defaults.ts", drift::DEFAULTS_TS),
    ];
    for (path, text) in embedded {
        if let Some(Value::Object(entry)) = sources.get_mut(path) {
            entry.insert("sha256".into(), json!(sha256_hex(text.as_bytes())));
        }
    }
    (Value::Object(sources), clean)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachmentInput {
    id: String,
    name: String,
    content: String,
    #[serde(default)]
    size_bytes: Option<u64>,
    #[serde(default)]
    content_type: Option<String>,
    /// Must be true: the harness only accepts generated, non-user files.
    synthetic: bool,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    OpenSession {
        session: String,
        policy: String,
        #[serde(default)]
        attachments: Vec<AttachmentInput>,
    },
    Call {
        session: String,
        name: String,
        arguments: Value,
        round: u32,
        calls_this_round: u32,
        /// Only for exercising the duplicate-ID check; the harness otherwise
        /// assigns every call ID itself (the model never supplies one).
        #[serde(default)]
        call_id: Option<String>,
    },
    CloseSession {
        session: String,
    },
    RenderAttachments {
        attachments: Vec<AttachmentInput>,
    },
}

struct Session {
    policy_id: String,
    request: app::ChatRequest,
    state: app::ToolSession,
    seq: u64,
    terminated: Option<&'static str>,
}

struct Harness {
    env_id: String,
    policies: policy::PolicySet,
    catalog: Vec<app::ToolDefinition>,
    sessions: HashMap<String, Session>,
}

fn attachments(inputs: Vec<AttachmentInput>) -> Result<Vec<app::AttachmentContext>, String> {
    inputs
        .into_iter()
        .map(|input| {
            if !input.synthetic {
                return Err(format!(
                    "attachment {} is not marked synthetic; the harness never reads user files",
                    input.id
                ));
            }
            Ok(app::AttachmentContext {
                id: input.id,
                name: input.name,
                content: input.content,
                size_bytes: input.size_bytes,
                content_type: input.content_type,
            })
        })
        .collect()
}

impl Harness {
    fn handle(&mut self, line: &str) -> Value {
        let input: Input = match serde_json::from_str(line) {
            Ok(input) => input,
            Err(error) => {
                return json!({ "ok": false, "error": format!("bad request line: {error}") });
            }
        };
        match self.dispatch(input) {
            Ok(value) => value,
            Err(error) => json!({ "ok": false, "error": error }),
        }
    }

    fn dispatch(&mut self, input: Input) -> Result<Value, String> {
        match input {
            Input::OpenSession {
                session,
                policy,
                attachments: files,
            } => {
                if self.sessions.contains_key(&session) {
                    return Err(format!("session {session} is already open"));
                }
                let request =
                    self.policies
                        .request(&policy, &session, &self.catalog, attachments(files)?)?;
                let offered = app::offered_names(&request);
                self.sessions.insert(
                    session.clone(),
                    Session {
                        policy_id: policy.clone(),
                        request,
                        state: app::ToolSession::default(),
                        seq: 0,
                        terminated: None,
                    },
                );
                Ok(json!({
                    "ok": true, "op": "open_session", "session": session, "policy": policy,
                    "offered_tools": offered, "env_id": self.env_id,
                }))
            }
            Input::CloseSession { session } => {
                let closed = self.sessions.remove(&session).is_some();
                Ok(json!({ "ok": closed, "op": "close_session", "session": session }))
            }
            Input::RenderAttachments { attachments: files } => {
                let message = app::render_attachments(&attachments(files)?);
                Ok(json!({ "ok": true, "op": "render_attachments", "message": message }))
            }
            Input::Call {
                session,
                name,
                arguments,
                round,
                calls_this_round,
                call_id,
            } => self.call(session, name, arguments, round, calls_this_round, call_id),
        }
    }

    fn call(
        &mut self,
        session_id: String,
        name: String,
        arguments: Value,
        round: u32,
        calls_this_round: u32,
        call_id: Option<String>,
    ) -> Result<Value, String> {
        let policies = &self.policies;
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("session {session_id} is not open"))?;
        if let Some(code) = session.terminated {
            return Err(format!(
                "session {session_id} was ended by the host ({code}); no further calls run"
            ));
        }
        let seq = session.seq;
        session.seq += 1;
        let call_id_source = if call_id.is_some() {
            "request"
        } else {
            "harness"
        };
        let id = call_id.unwrap_or_else(|| {
            let digest = sha256_hex(format!("{}:{session_id}:{seq}", self.env_id).as_bytes());
            format!("call_{}", &digest[..24])
        });
        let call = app::NormalizedToolCall {
            id: id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        };
        let policy_id = session.policy_id.clone();
        let started_unix_ms = unix_ms();
        let clock = Instant::now();
        let outcome = app::host_decide(
            &session.request,
            &mut session.state,
            &call,
            round,
            calls_this_round,
            |tool| policies.decision(&policy_id, tool),
        )?;
        let wall_ns = clock.elapsed().as_nanos();
        if let Some((code, _)) = outcome.terminated {
            session.terminated = Some(code);
        }
        let status = match (&outcome.result, outcome.terminated) {
            (Some(result), _) => result["status"].as_str().unwrap_or("invalid").to_owned(),
            (None, Some(_)) => "request-terminated".to_owned(),
            (None, None) => return Err("host produced neither a result nor a termination".into()),
        };
        let trace_id = sha256_hex(format!("{}:{session_id}:{seq}", self.env_id).as_bytes());
        Ok(json!({
            "ok": true,
            "schema": TRACE_SCHEMA,
            "trace_id": &trace_id[..32],
            "env_id": self.env_id,
            "session": session_id,
            "seq": seq,
            "policy": policy_id,
            "call_id": id,
            "call_id_source": call_id_source,
            "request": {
                "name": name,
                "arguments": arguments,
                "round": round,
                "calls_this_round": calls_this_round,
            },
            "gate": outcome.gate,
            "permission": {
                "asked": outcome.permission_asked,
                "decision": outcome.permission_decision,
                "decision_source": if outcome.permission_asked { Some("host-policy-fixture") } else { None },
                "preview": outcome.preview,
            },
            "path": outcome.path,
            "status": status,
            "result": outcome.result,
            "tool_message": outcome.result.as_ref().map(app::tool_message),
            "terminated": outcome.terminated.map(|(code, message)| json!({ "code": code, "message": message })),
            "timing": { "started_unix_ms": started_unix_ms, "host_wall_ns": wall_ns },
        }))
    }
}

fn environment(
    policy_bytes: &[u8],
    policies: &policy::PolicySet,
    catalog: &[app::ToolDefinition],
) -> Result<Value, String> {
    let drift_checks = drift::check()?;
    let (sources, clean) = app_sources();
    let harness: Map<String, Value> = HARNESS_SOURCES
        .iter()
        .map(|(path, text)| ((*path).to_owned(), json!(sha256_hex(text.as_bytes()))))
        .collect();
    let catalog: Vec<Value> = catalog
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name, "description": tool.description,
                "risk": tool.risk, "parameters": tool.schema,
            })
        })
        .collect();
    Ok(json!({
        "schema": ENV_SCHEMA,
        "harness": {
            "name": env!("CARGO_PKG_NAME"),
            "version": env!("CARGO_PKG_VERSION"),
            "build_profile": env!("JUNIPER_BUILD_PROFILE"),
            "sources_sha256": harness,
        },
        "app": {
            "commit": env!("JUNIPER_APP_COMMIT"),
            "sources_match_commit": clean,
            "sources": sources,
            "tool_protocol": tools::PROTOCOL_VERSION,
            "limits": {
                "max_tool_rounds": tools::MAX_TOOL_ROUNDS,
                "max_tool_calls_per_round": tools::MAX_TOOL_CALLS_PER_ROUND,
                "max_payload_bytes": tools::MAX_PAYLOAD_BYTES,
                "max_expression_bytes": tools::MAX_EXPRESSION_BYTES,
            },
        },
        "drift_checks": drift_checks,
        "rustc": env!("JUNIPER_RUSTC_VERSION"),
        "os": os_release(),
        "policy": {
            "set": policies.set_id(),
            "sha256": sha256_hex(policy_bytes),
            "ids": policies.ids(),
        },
        "catalog": catalog,
        "started_unix_ms": unix_ms(),
        "network": "none",
        "user_data": "none",
    }))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let policy_path = match args.as_slice() {
        [_, flag, path] if flag == "--policy" => path.clone(),
        _ => return Err("usage: juniper-tool-trace --policy <host_policy.json>".into()),
    };
    let policy_bytes =
        std::fs::read(&policy_path).map_err(|error| format!("{policy_path}: {error}"))?;
    let catalog = catalog()?;
    let policies = policy::PolicySet::parse(&policy_bytes, &catalog)?;
    let env = environment(&policy_bytes, &policies, &catalog)?;
    let env_text = env.to_string();
    let env_id = sha256_hex(env_text.as_bytes())[..32].to_owned();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut emit = |value: &Value| -> Result<(), String> {
        writeln!(out, "{value}")
            .and_then(|()| out.flush())
            .map_err(|error| format!("stdout: {error}"))
    };
    emit(&json!({ "ok": true, "op": "environment", "env_id": env_id, "environment": env }))?;
    let mut harness = Harness {
        env_id,
        policies,
        catalog,
        sessions: HashMap::new(),
    };
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|error| format!("stdin: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        emit(&harness.handle(&line))?;
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("juniper-tool-trace: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests;
