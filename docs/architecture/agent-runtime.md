# Agent runtime (Beta)

Agent sits beside Chat. Chat is a conversation. Agent hands a goal to a
personal agent, which works through it with tools the user allowed and shows
each step. The decision record is [ADR 0025](../adr/0025-agent-runtime.md).
This page describes how the runtime works.

## Ownership

| Part                                                        | Owner                                                             |
| ----------------------------------------------------------- | ----------------------------------------------------------------- |
| Task scheduling, lifecycle, budgets, retries                | Rust (`src-tauri/src/agent/runner.rs`)                            |
| Tool dispatch, permission decisions, approvals, grants      | Rust (`agent/tools.rs`, `agent/policy.rs`)                        |
| Durable state: tasks, events, actions, approvals, artifacts | Rust, SQLite schema v5 (`agent/store.rs`)                         |
| Agent identity: name, avatar, personality, model, memory    | The existing assistant profile, edited in Settings › Assistants   |
| Agent working settings: tools, skills, autonomy, budgets    | A native agent profile per assistant (`agent_profiles`, schema 1) |
| Presentation and user intents                               | React (`src/app/AgentScreen.tsx`, `AgentManage.tsx`)              |

The interface sends only intents: start, steer, answer, approve or deny,
pause, resume, cancel, and stop all. The host checks each intent against stored
state and the transition model. A stale interface snapshot cannot write to the
agent tables, because `save_app_data` rebuilds only the snapshot projections and
the agent tables hold no foreign keys to them.

## Task states

```text
queued ──► running ──► completed | failed | cancelled
  ▲  │        │
  │  │        ├──► waiting-input ────┐
  │  │        ├──► waiting-approval ─┤──► queued (answered / decided)
  │  │        ├──► paused ───────────┤──► queued (resumed)
  │  │        └──► interrupted ──────┘    (restart or global stop)
  │  └──► paused | cancelled
  └── completed | failed (a follow-up from the user reopens the task)
```

`TaskStatus::can_become` enforces these transitions, and every change is
recorded as an event. Other conditions are not separate states:

- A parent waiting for subtasks stays `queued` with `pending.wait = children`.
  The worker does not pick it until every child has finished.
- A task waiting for a busy local model stays `queued` with `usage.notBefore`.

**Completed** means the model finished with a final answer and the host
accepted it. If the answer cites a source the task never retrieved, the
citation is listed under `result.unverified`. A task that runs out of budget
ends **failed**, with `outcome: "partial"`, and keeps what it produced.

## The loop

Each model step builds its prompt the same way:

1. Host instructions: identity, constitution rules, runtime facts (model,
   where it runs, time zone, granted folders, offered tools), the untrusted-data
   rule, and how to work.
2. Below those, the user's agent profile text and the project instructions,
   each labelled as unable to override the host.
3. Curated memories.
4. The task transcript.

The model returns either an answer or tool calls. Calls are processed in order.
For each call the host does the following:

1. Record an action as `proposed`. A reused call id is refused here.
2. **Prepare** it: check the arguments against the host's schema, resolve its
   scope, and compute the target state and a host-written preview (for example,
   a unified diff of a file write).
3. **Decide** it with `policy::decide`. The tool's risk comes from the host's
   registry, never from the model, a skill, or a plugin.
   - Safe tools and reads in granted folders run without asking. The folder
     grant is checked in the database at each call, so removing a folder stops
     a running task from using it.
   - Writes, network use, sandboxed code, and plugin calls ask unless a matching
     standing grant exists.
   - Memory changes always ask, one call at a time.
4. **Bind** an approval to `sha256(tool, canonical arguments, target)`. On
   resume, the host prepares the call again. If anything changed (the file's
   version, the arguments, or a plugin's tool set), the approval is stale and
   the user is asked again.
5. Mark the action `started`, execute it, and record the host result.

The transcript is committed after every model turn and every tool result.

**Restart recovery.** On startup:

- Running tasks become `interrupted`, and `started` actions become `uncertain`.
- On resume, a `workspace.write` is reconciled by comparing the file with the
  intended content.
- Any other uncertain action is reported to the model as "outcome unknown". It
  is never silently repeated.

## Budgets

| Limit                         | Default | Ceiling   |
| ----------------------------- | ------- | --------- |
| Model steps                   | 16      | 64        |
| Tool calls                    | 48      | 200       |
| Tokens (estimated if unknown) | 120,000 | 1,000,000 |
| Active time                   | 30 min  | 4 h       |
| Documents                     | 10      | 50        |
| Subtasks per task / depth     | 4 / 2   | 8 / 2     |
| Retries per failure cause     | 2       | 5         |

Other per-turn limits:

- At most 8 tool calls are processed per model turn.
- Tool results are cut to 24,000 characters.
- Older tool results are shortened (and labelled as shortened) once the
  transcript passes 60,000 characters.

Chat's own four-round tool loop is unchanged.

## Delegation

`agent.delegate` creates a child task with a role: researcher, file analyst,
implementer, or reviewer.

- **Tools.** The child gets the parent's offered tools intersected with the
  role's list. A reviewer has no write tools.
- **Budget.** The child's budget is carved from the parent's remainder, and the
  parent is charged the child's full token allowance up front, so delegation
  never enlarges the total.
- **Context.** The child sees only the objective and the context the parent
  passed, framed as data. It does not see the parent's conversation.
- **Execution.** Children run on the same single worker, so local inference
  stays one at a time.
- **Joining.** The parent resumes when every child has finished and receives
  their structured reports as a tool result.
- **Cancellation.** Cancelling a parent cancels every unfinished descendant.

## Scheduling

Schedules store a rule, an IANA time zone, and the next run as a UTC instant.
They run **while Juniper is running**; there is no background service.

- **Time zones and DST.** A time skipped by a DST change runs at the first
  minute after the gap. A time that occurs twice runs once.
- **Claiming.** Each due run is claimed with a compare-and-swap on `next_run_at`
  and recorded under a unique (schedule, scheduled time, attempt) key.
- **Missed runs.** After the computer was off, only the latest missed run runs
  (policy `run-once`), or it is skipped (`skip`). A backlog is never replayed.
- **Overlap.** A run is skipped while the previous one is unfinished.
- **Retries.** A failed run is retried only if it changed nothing outside
  Juniper.
- **Reminders.** A reminder goes to the inbox without loading a model.
- **Permissions.** Scheduled tasks run under the agent's current permissions.
  Anything that needs approval waits, and an approval request expires after
  72 hours without ever becoming an allow.

## What Beta means here

- **Quality depends on the model.** A model without measured tool-use evidence
  is not offered for agent tasks unless the user allows it explicitly, and its
  tasks say so.
- **Platforms.** Desktop has project folders. Linux also has sandboxed code;
  Windows and Android do not. On Android, agent tasks need a connected model,
  because the on-device engine has no tool calling.
- **Notifications** go to the Agent inbox. Operating-system notifications are
  not implemented.
