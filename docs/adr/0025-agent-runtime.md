# ADR 0025: Host-owned agent runtime

Status: Proposed for review on the "Fixing Up A Little" branch (draft PR #49).

## Context

Juniper needed an Agent beside Chat. An agent works on a goal over many
steps, waits for approvals or answers, delegates, and runs on a schedule. The
state Chat already has is a frontend-owned snapshot that `save_app_data`
rebuilds on every save. Task progress and approvals kept there could be
overwritten or recreated by a stale save. Chat's tool loop (four rounds,
answered in one request) cannot hold work that waits for a person.

## Decision

- **The native host owns agent work.** Tasks, events, actions, approvals,
  grants, artifacts, sources, projects, schedules, skills, and plugins live in
  SQLite schema v5 tables that only the host writes. None of these tables
  references a snapshot table by foreign key.
- **Migration.** It runs in one transaction. Before it runs, the existing
  database is copied with `VACUUM INTO` to `juniper.db.schema-v4.backup`.
- **One worker.** A single worker runs a bounded loop: observe, plan, act,
  inspect, finish. It commits after every model turn and tool result, has
  explicit budgets and per-cause retries, and follows a documented transition
  model (`docs/architecture/agent-runtime.md`).
- **One model path.** The agent calls models through `providers::agent_turn`
  over a provider-neutral transcript. Local models are reached through the
  same resident llama-server as chat, with the lease held for one turn. Juniper
  1's token-ID backend is refused for agent work.
- **One decision point.** Every tool call is prepared (schema check, scope,
  preview, target state) and decided by `agent::policy`. Risk comes from the
  host's registry. Approvals bind to the call and its target state, and
  standing grants are scoped to the agent and project, plus a plugin's
  tool-set digest.
- **Identity.** An agent is an assistant (name, avatar, personality, model,
  memory policy) plus a native agent profile (tools, skills, autonomy preset,
  budgets, language). This keeps a single personality system.

## Consequences

- The interface only reads native state and sends intents, so a crashed or
  stale view cannot change task state.
- Agent memory writes reach the frontend-owned memory list through a native
  queue (`memory_outbox`), each one approved per call.
- Work runs only while Juniper is running. A restart interrupts running tasks
  and marks started actions uncertain until they are reconciled.

## Security and privacy

Untrusted content is framed as data and cannot grant permissions. A model
cannot choose its tools' risk. Delegated tasks get the intersection of their
parent's tools and their role's tools. Revoking a grant pauses running tasks
of that agent. A global stop interrupts all agent work and survives restart.
