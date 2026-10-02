# ADR 0023: GPT-OSS backend and resident desktop runtime

Status: Proposed for review (branch `feature/juniper-wrapper-gpt-oss`).
Supersedes the per-request desktop server in ADR-0016; ADR-0011 is unchanged.

## Context

The owner froze unpruned `openai/gpt-oss-20b` (revision `6cee5e81`) as
Juniper's first large local language model on 2026-10-02, on the evidence in
Cinqic-Research/Juniper-LM-1.1 (`reports/final/final-selection.json`). That
qualification measured one configuration and recorded the requirements any
integration must meet (`reports/final/INTEGRATION_REQUIREMENTS.md`):

- the CPU-only `llama-server` Juniper packages cannot serve it acceptably; the
  qualified build is llama.cpp b11270 (`748d4225`) with CUDA for sm_75;
- loading takes about 150 s from a cold HDD and 6–131 s on reloads, so one
  server per request is not viable;
- the GGUF's embedded chat template differs from OpenAI's Harmony renderer in
  four places; the corrected template renders token-identically;
- reasoning effort is low, medium, or high (no "none"); analysis must stay out
  of history except while a tool call is pending; one tool call per message;
- `finish_reason: "length"` is not an answer; the host prompt cache must be
  bounded explicitly; expert placement and context are one decision and must
  not be silently refitted.

## Decision

**A backend boundary inside the provider loop.** `src-tauri/src/backend.rs`
defines `RequestPolicy`, which carries everything model-family-specific:
request shaping, reasoning-effort mapping, single-call tools, analysis
returned with a pending call, protocol markers that invalidate an answer,
idle timeout, loopback key, context window, and verified lineage.
`Backend::Generic` reproduces the previous behavior for every other model.
`Backend::GptOss` loads `config/backends/gpt-oss-20b.json`, the qualified
profile. No other module branches on model family.

**A catalog artifact names its profile.** `gpt-oss-20b` in
`config/models/catalog.json` carries `backendProfile` and qualified
capabilities. The runtime refuses to apply a profile to an artifact whose ID,
size, or SHA-256 differ from the profile's.

**The artifact is imported, not downloaded.** The qualified GGUF was converted
and quantized by Cinqic from OpenAI's weights; there is no public URL for these
exact bytes and Juniper does not redistribute weights. The catalog lists the
file with its size and SHA-256 and no URL; `import_managed_model` installs a
user-selected file only if it matches, hard-linking it on the same filesystem
so a 12 GB model is not duplicated, copying (and hashing what it writes)
otherwise. A verification record beside the file avoids re-hashing 12 GB on
every start while the file's size and modification time are unchanged.

**One resident server, owned by Juniper.** `src-tauri/src/local_runtime.rs`
keeps a single `llama-server` loaded across turns, for every managed local
model:

- loopback bind, a per-launch API key read from an owner-only file that is
  deleted once the server has read it, `--no-webui`, `--no-slots`,
  `--offline`, and a bounded `--cache-ram`;
- a qualified model starts only with its profile's exact flags (`--fit off`),
  the corrected template, and the CUDA build; `--version` must report the
  profile's commit before load, and `/props` must report the same build, the
  expected served-template hash, and context size after load;
- one warm-up request after a qualified load; the interface shows loading,
  warm-up, restart, and reasoning activity;
- a crashed server is detected on the next request and restarted; switching
  models or unloading waits for in-flight generations instead of killing them;
- the server is stopped on application exit and on "Unload from memory".

**Request rules for GPT-OSS.** Sampling is pinned to the qualified values
(temperature 1.0, top_p 1.0); `max_tokens` is always set and capped at 8,192;
reasoning effort is `medium` by default, `low` for "Off or lowest", `high`
only when chosen; `parallel_tool_calls: false`; the prompt is rendered and
tokenized by the server before each turn, and a request that cannot fit its
output budget fails with `CONTEXT_OVERFLOW`.

**No fallback.** A missing CUDA build, a different build, a template or
identity mismatch, or insufficient memory is an explicit error. Juniper never
substitutes the CPU runtime, another model, or a remote provider.

## Consequences

- Small local models also stay resident now. They load quickly, so this costs
  memory rather than latency; "Unload from memory" frees it.
- Release builds do not include the CUDA server yet; until they do, the
  GPT-OSS entry reports that its runtime is unavailable. Developers use
  `JUNIPER_LLAMA_SERVER_CUDA` or `JUNIPER_LLAMA_VARIANT=cuda
scripts/build-llama-runtime.sh`.
- The profile is FLOWBOX-specific (8 threads, 18 expert layers on the CPU for a
  6 GB GPU). Other machines need their own measured profile; the runtime does
  not adapt it.
- The 8K fallback context measured during qualification is not wired in: using
  it would be an explicit, user-visible choice, which this change does not add.
- An external server running gpt-oss (Ollama, another OpenAI-compatible
  endpoint) still uses the generic backend. It is not the qualified
  configuration and is not treated as one.

## Security and privacy

The server accepts only loopback connections bearing the per-launch key, so
other local users and browser pages cannot use the loaded model. Same-user
processes can still read the key file during the moments before the server
reads it; that is within the existing desktop threat model. Server stderr is
kept only in memory, bounded, to classify startup failures, and is never
logged or displayed. Nothing leaves the device.
