# Phase 12: Fixing Up A Little (2026-10-15)

Status: `MERGED` (updated Saturday, October 17, 2026, EDT). Implemented by
Claude Opus 5.5 under the owner's directive of 2026-10-15. PR #49 merged into
`main` at `ea9cbe566ec7c59036ab9601ab522a5ccb639c34` after three independent
review iterations (Claude Fable 5.1, separate sessions, same vendor): at
`406a0be` and `2450c7a` (CHANGES_REQUIRED) and at `782956a`
(APPROVED_WITH_CONDITIONS). The follow-ups from the last review (F-17 to F-20)
merged with PR #51 at `ad20e74`. The exact-source large-transfer and
native-application workflow gates were then passed by a non-implementer run
(`docs/qualification/0.4.0-alpha-qualification-2026-10-17/`). Release status:
`docs/release/juniper-0.4.0-alpha-readiness-review.md`.

The status before the merge was `BLOCKED - INDEPENDENT REVIEW`: the candidate
was draft and unmerged because exact-source large-transfer evidence and
independent real-app workflow evidence were not yet available. See
`docs/qualification/pr49-independent-review-2026-10-15.md` for that review's
disposition and limits, and its closing note.

## Objective

1. A broad, trustworthy model catalog, with a downloader that handles large
   and multi-file models.
2. An **Agent (Beta)** beside Chat that does real work: projects, files, web,
   skills, plugins, subtasks, and schedules, under host-enforced permissions.

## Starting and final commit

- Base: `main` at `77f5da7cec3791360bc8ab80f3507431c0f3acd8` (PR #46 merge).
- Branch: `claude/fixing-up-a-little`.
- Head: see PR #49. The final report gives the exact SHA.

## Plan (dependency order)

| Stage                        | Result                                                                                                         | Commits                         |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------- | ------------------------------- |
| 0. Inspect and baseline      | Repository inventory, baseline (186 Rust / 132 Vitest pass), storage and runtime facts                         | —                               |
| 1. Foundations               | Schema v5 native tables, pre-migration backup, contracts, policy, runner                                       | `7cbcb4d`, `0e86f44`, `4bea857` |
| 2. Models                    | Dated popularity research, 23 new releases, artifact choice, resumable and sharded transfers, live catalog run | `c288715`                       |
| 3. Single agent              | Chat → Agent [Beta] → Models → Settings; task view; approvals; restart recovery                                | `08c6168`, `c24397a`            |
| 4. Context and capabilities  | Projects and FTS retrieval, web with citations, workspace, sandbox, skills, MCP                                | `0e86f44`, `49e6d0b`, `43d7bac` |
| 5. Delegation and recurrence | Subtasks with carved budgets, schedules with DST and missed-run rules, schedules from conversation             | `0e86f44`, `7db539e`            |
| 6. Completion                | Docs, ADRs 0025–0028, live evidence, validation                                                                | `4665f80` and later             |

## Requirements and evidence

Evidence types:

- **Unit/fixture**: deterministic tests in the repository.
- **Live**: real network, models, or servers on FLOWBOX (Linux x86_64, CPU
  llama.cpp b10788 bundled with Juniper).
- **Not verified**: not run.

| Area                | Implementation                                                                                               | Evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Remaining                                                                                                                                                                                              |
| ------------------- | ------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Catalog coverage    | `config/models/catalog.json`, `scripts/catalog-from-research.mjs`, `docs/research/model-catalog-2026-10-15/` | Unit: ten top-ranked eligible releases present (`catalog::tests`, `model-catalog.test.ts`); every URL pinned to a revision, size, and SHA-256. Live: all 32 downloadable artifacts other than the pre-existing gpt-oss-20b downloaded from their pinned URLs, matched size and SHA-256, loaded, and answered a factual question; 19 made a well-formed tool call in a one-request smoke check. Table: `docs/qualification/catalog-live-2026-10-15.md`                                                                                                                                                            | Windows and Android not exercised; capabilities stay unknown except where measured                                                                                                                     |
| Model lifecycle     | `managed_models.rs` (journal, ranges, bundles, locks, queue, no 2 GiB cap)                                   | Fixture: resume, ignored or wrong range, corrupt, oversize, truncated, bundle publish, locks, journal mismatch. Live: Qwen3 4B Q4_K_M (2.50 GB, above 2 GiB) paused after 234,591,188 bytes, resumed, and verified. The live Range/Content-Range headers were not retained; exact-range behavior is covered by fixtures                                                                                                                                                                                                                                                                                          | Disk full is classified but not exercised on a real full disk                                                                                                                                          |
| Agent runtime       | `src-tauri/src/agent/`                                                                                       | Fixture: 8 end-to-end runner tests (deliverable with an approved write, denial and stale approval, citations and budgets, delegation and cancellation, restart reconciliation, questions and unknown tools, MCP approval chain, schedule from conversation). Live: see `docs/qualification/agent-live-2026-10-15.md`                                                                                                                                                                                                                                                                                             | Quality depends on the model                                                                                                                                                                           |
| Persistence         | `storage.rs` v6 (v4→v5 agent tables; v5→v6 schedule recovery)                                                | Unit: v4 upgrade keeps a restorable backup; v5 upgrade adds schedule recovery and a unique schedule-run index; an interrupted v5 migration leaves v4 intact; duplicate v5 schedule ids fail closed and keep the backup. Live: a copy of a real rc.33 database (v4: 1 assistant, 7 chats, 3 models) upgraded to v5 with every count and setting unchanged. Superseded for v6: a copy of the same v4 database upgraded to v6, restored from its backup, and upgraded again is recorded in `docs/qualification/pr49-gate-evidence-2026-10-16.md` (gate 2a) and was reproduced by the 2026-10-16 independent review. | —                                                                                                                                                                                                      |
| Workspace           | `agent/workspace.rs` (cap-std)                                                                               | Unit: traversal, absolute, and drive paths refused; symlinks inside and out refused; version-bound atomic writes; a concurrent edit is detected                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Windows reparse points are covered only by cap-std, not tested here                                                                                                                                    |
| Sandbox             | `agent/sandbox.rs` (bubblewrap)                                                                              | Unit with a real bwrap: no home folder, no environment, no network, read-only `/usr`, read-only project, CPU limit, cancellation kills children; on bubblewrap without `--size`, `/tmp` lives in the quota-watched work folder (forced-path test)                                                                                                                                                                                                                                                                                                                                                                | Linux only; Windows and Android show it as unavailable                                                                                                                                                 |
| Web                 | `agent/web.rs`                                                                                               | Unit: address policy (v4, v6, mapped, NAT64, 6to4), URL refusals, HTML reduction. Live: Wikipedia search and page read; metadata, localhost, and 10.x refused; a public redirect to loopback refused                                                                                                                                                                                                                                                                                                                                                                                                             | Brave and SearXNG providers not exercised live                                                                                                                                                         |
| Skills              | `agent/skills.rs`, `config/skills/`                                                                          | Unit: four bundled skills round-trip; malformed, oversized, script, link, and stray packages refused; imports start disabled                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | —                                                                                                                                                                                                      |
| MCP                 | `agent/mcp.rs`                                                                                               | Fixture: both eras over stdio and HTTP, pagination, invalid tool excluded, cancellation. Live: `server-everything` connected (legacy 2025-11-25) and its echo tool called                                                                                                                                                                                                                                                                                                                                                                                                                                        | No live 2026-07-28 server was available; OAuth not implemented                                                                                                                                         |
| Subagents           | `runner.rs` delegate and join                                                                                | Fixture: role ∩ parent tools, carved budget, isolated context, join, cancel cascade                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | —                                                                                                                                                                                                      |
| Scheduling          | `agent/schedule.rs`                                                                                          | Unit: New York DST (fall and spring), gap and fold, weekly, monthly, once, interval, single claim, no backlog replay, skip policy, reminders without a model, revision conflicts                                                                                                                                                                                                                                                                                                                                                                                                                                 | No background service by design                                                                                                                                                                        |
| Projects and memory | `agent/retrieval.rs`, `memory_outbox`                                                                        | Unit: index versions, removal, revoked folders, project isolation. Vitest: queued memory changes applied once                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | —                                                                                                                                                                                                      |
| UI                  | `AgentScreen.tsx`, `AgentManage.tsx`, `ModelsScreen.tsx`                                                     | Vitest: four destinations with Beta, approval intents only, once-only approvals, platform limits shown, Dark mode in onboarding                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Rendered app: onboarding, Models transfer with pause, restart, and resume, Chat, and an on-device Agent task (`docs/qualification/agent-live-2026-10-15.md`)                                           |
| Regressions         | —                                                                                                            | `pnpm validate` passes (format, lint, types, Vitest, rustfmt, clippy `-D warnings`, cargo test, schemas, version, license, runtime contract, branding)                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Android compile and Windows install smoke run in CI only (they first failed on Linux-only code, fixed in `8ae7199`); the Linux launch smoke's Settings click moved with the new navigation (`2679d78`) |

## Same-vendor review

A read-only security review by a Claude subagent (same vendor, **not
independent**) found eleven defects in the agent runtime and transfers. All
were fixed in `1100ea1` and `c069c22`; the list is in that commit message.

## Known limitations

- **Platforms.** Agent runs on desktop. Sandboxed code is Linux-only. On
  Android, agent tasks need a connected model, and the Android build was not
  run in this phase.
- **Scheduling and notifications.** Scheduled work runs only while Juniper is
  running. Notifications go to the in-app inbox; operating-system notifications
  are not implemented.
- **MCP.** Tools only. No resources, prompts, sampling, elicitation, roots,
  OAuth, or server apps. Local servers are not sandboxed, and the interface
  says so.
- **Catalog.** Models with model-specific licenses require acceptance in
  Juniper. Gated upstream repositories are not used; downloads come from
  reputable public GGUF conversions, recorded per entry.
  Gemma's mutable terms page has no preserved historical response bytes;
  Juniper links the current terms, asks again before every Gemma download, and
  does not store acceptance for that page.
- **Model quality.** Small models may call tools poorly or not at all. The
  catalog's live table records each model's single-request tool-call smoke.
  Models are not marked qualified for agent work.

## Recovery commands

```bash
# To roll back the latest v5-to-v6 migration, stop Juniper and restore its v5 backup:
cp ~/.local/share/com.cinqic.juniper/juniper.db.schema-v5.backup ~/.local/share/com.cinqic.juniper/juniper.db
# To roll back the original rc.33 v4-to-v5 agent migration instead:
cp ~/.local/share/com.cinqic.juniper/juniper.db.schema-v4.backup ~/.local/share/com.cinqic.juniper/juniper.db
# Regenerate catalog entries from research:
node scripts/catalog-from-research.mjs docs/research/model-catalog-2026-10-15
```

## Reviewer and approval status

Implementer-run evidence for the three review gates (exact-source transfers,
copied-database restore, and native app workflows) is in
`docs/qualification/pr49-gate-evidence-2026-10-16.md`. It is not independent.

`MERGED` (2026-10-17). The review history is recorded in
`docs/qualification/pr49-gate-evidence-2026-10-16.md` (repairs per review
iteration) and in the closing note of
`docs/qualification/pr49-independent-review-2026-10-15.md`. The
non-implementer G2/G3 records are in
`docs/qualification/0.4.0-alpha-qualification-2026-10-17/`. The live catalog
table above remains the implementer's own evidence. Five of its entries (Llama
3.x and Gemma 3, third-party mirrors) were later held from the 0.4.0-Alpha
catalog; see `docs/research/model-catalog-2026-10-15/exclusions-and-blockers.md`.

Earlier status, kept for the record: `BLOCKED - INDEPENDENT REVIEW`, with the
repairs and local validation recorded in the independent review report.
