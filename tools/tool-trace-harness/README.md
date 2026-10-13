# Juniper host tool-trace harness

A non-shipping command-line tool that records what the Juniper host does with a
model's tool request: the gate decision, the permission prompt and its answer,
the actual execution and the exact `juniper-tool-protocol-v1` result the App
would send back to the model. Juniper 1 tool-use data and evaluation
(Juniper-1-Data `j1data.tooltrace`) use it as the only source of tool results.

It is not part of the application. It is never bundled, has no Tauri
dependency, adds no command or IPC surface, opens no network connection, runs
no process, and never reads user files, memories or chats.

## What is the App's code and what is restated

- `src-tauri/src/tools.rs` is compiled unchanged (`#[path]` module):
  `validate_call`, `execute_call`, `host_result`, `host_risk`, `loop_allowed`.
- `build.rs` copies these items verbatim out of `src-tauri/src/providers.rs`
  and compiles them against shim types with the App's field names:
  `offered_tools`, `ToolGate`, `tool_gate`, `requires_per_call_approval`,
  `permission_preview`, `bounded_preview`, `denied`, `permission_grant_allows`,
  `memory_belongs_to_assistant`. From `src-tauri/src/behavior.rs` it copies
  `neutralize_control_tokens`, `strip_tags` and `attachments_message`. A renamed
  or re-typed item fails the build.
- The tool catalog is the App's `builtinTools` array from `src/lib/defaults.ts`,
  converted to JSON at build time; each risk must equal `host_risk`.
- Restated in `src/app.rs` (the App versions are async or need Tauri state):
  the order of `host_tool_turn` (duplicate call ID, loop bound, gate, argument
  validation before the prompt, permission preview, prompt, execution), the
  `PERMISSION_TIMEOUT` end of `request_permission`, and the stateless and
  attachment arms of `execute_host_tool`. `src/drift.rs` checks at startup and
  in `cargo test` that those App bodies still contain every step, code and
  message in the same order; the harness refuses to run otherwise.

## Fidelity limits

- The permission prompt is answered by a fixed policy fixture
  (`fixtures/host_policy_v1.json`), not a person. Each trace records
  `decision_source: host-policy-fixture`.
- `memory.*` and `chat.search` may only be denied or time out: executing them
  needs user data. `system.info` is never enabled: it reports the real host.
  `memory.delete` therefore always ends in the App's `MEMORY_NOT_FOUND` path.
- `file.read` and `file.metadata` run only on attachments the driver marks
  `synthetic: true`.
- A permission timeout ends the request (`PERMISSION_TIMEOUT`), exactly as in
  the App; no tool message follows.
- Call IDs are assigned by the harness (`call_` + 24 hex digits derived from
  the environment, session and sequence number). The model never supplies one.
  A driver may pass `call_id` only to exercise the duplicate-ID check.

## Line protocol

```text
juniper-tool-trace --policy fixtures/host_policy_v1.json
```

The first stdout line is `{"op": "environment", "env_id", "environment"}`:
App commit, git blob IDs of the bound sources at that commit and in the working
tree (`sources_match_commit`), SHA-256 of every harness source, drift checks,
`rustc` version, OS, policy fixture digest and the catalog. Then each stdin line
produces exactly one stdout line:

- `{"op": "open_session", "session", "policy", "attachments": [...]}` replies with
  `offered_tools`, the tools the App's `offered_tools` would offer under that policy.
- `{"op": "call", "session", "name", "arguments", "round", "calls_this_round"}`
  returns a `j1.host-trace.v1` record: `trace_id`, `call_id`, `gate`,
  `permission` (asked, decision, preview), `path`, `status`, `result` (the
  `HostToolResult`), `tool_message` (the exact text the App sends the model),
  `terminated` and `timing` (`host_wall_ns`).
- `{"op": "render_attachments", "attachments": [...]}` returns the App's
  user-role attachments message.
- `{"op": "close_session", "session"}`

`round` is the zero-based model generation within the turn and
`calls_this_round` the one-based index of the call within it (the App's
`index + 1`).

## Build and test

```text
cargo test --locked --manifest-path tools/tool-trace-harness/Cargo.toml
cargo clippy --locked --manifest-path tools/tool-trace-harness/Cargo.toml --all-targets -- -D warnings
```
