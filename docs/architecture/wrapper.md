# The Juniper wrapper

The language model writes text and proposes tool calls. Juniper decides what
the model is told, what it may use, what runs, what is saved, and what reaches
the user. This document describes that boundary; ADR-0022 and ADR-0023 record
the decisions.

## Control flow

```text
ChatScreen (webview)
  buildContext: profile layer, curated memory IDs, history by whole exchanges,
  output budget reserved
        │  ChatRequest (profile, history, current message, attachments,
        │  memory IDs, offered tool definitions, grants)
        ▼
commands::chat_stream ── juniper-local ──► local_runtime::stream_chat
        │                                    resident llama-server, identity
        │                                    checks, RequestPolicy (GptOss|Generic)
        │ other providers: RequestPolicy::generic
        ▼
providers::stream
  validate_chat_request ─ roles, sizes, IDs, grants
  request_messages ────── behavior::instructions (identity, constitution,
                          runtime section, profile) + framed memories,
                          history, framed attachments
  per turn:
    openai_body + RequestPolicy::shape_body
    check_context ──────── server template + tokenizer (qualified local only)
    stream turn ────────── text deltas → UI; reasoning → "reasoning" activity only
    no tool calls → TurnOutcome::accept_answer (length, empty, protocol markers)
    tool calls  → one per turn (Harmony) → host_tool_turn:
                   call-ID check → round bound → tool_gate (offered tools)
                   → argument validation → host preview → permission prompt
                   → execute_host_tool → host-authored result
  done event: error, or provenance
        ▼
ChatScreen: text, tool activity, errors (truncated answers kept and marked),
provenance in message details, "links not checked" note
```

## Instruction layers

| Order | Layer                                  | Author                                        | Editable by the user |
| ----- | -------------------------------------- | --------------------------------------------- | -------------------- |
| 1     | Identity and lineage                   | host (`behavior.rs`)                          | no                   |
| 2     | Constitution `juniper-constitution.v1` | `config/behavior/constitution.v1.json`        | no                   |
| 3     | Runtime section                        | host, from the request it is serving          | no                   |
| 4     | Assistant profile                      | assistant `systemPrompt`, personality, length | yes                  |
| —     | Memories                               | user data, framed as a `user` message         | yes (curation)       |
| —     | Attachments                            | user files, framed as a `user` message        | yes                  |

On GPT-OSS the first four become the Harmony developer message. The Harmony
system message keeps its required form ("You are ChatGPT…", knowledge cutoff,
current date, reasoning level), which the template renders; Juniper identity
lives in the developer message, as the qualification recommends.

## Context construction

The interface budgets with a 4-characters-per-token estimate:

1. host layers (constitution, identity, runtime section, tool list) — estimated;
2. profile;
3. current message and attachments;
4. the answer's output budget (`maxOutput`);
5. curated memories, newest first, while they fit;
6. history, newest exchange first, whole exchanges only.

If 1–4 alone exceed the model's context the request is not sent
(`CONTEXT_OVERFLOW`). Failed or truncated replies are never used as history.
For a qualified local backend the host then renders the exact prompt with the
server's template, counts it with the server's tokenizer, and refuses the turn
if prompt plus `max_tokens` exceeds the server's context. This also covers
tool transcripts added during the turn.

## Reasoning

Reasoning-capable models emit analysis separately from the answer. Juniper
never forwards it: the interface receives `activity: "reasoning"` and shows
"… is reasoning", nothing more. Analysis is not persisted (the native save
path drops any `reasoning` part, and older stored ones are dropped on load),
and it is not sent back in later requests. The single exception is Harmony's
rule that the analysis behind a pending tool call accompanies that call within
the same turn; it is held in memory (bounded to 64 KiB) for that turn only.

"Thinking" settings map per backend. For GPT-OSS: Auto and On → medium, Off →
low (GPT-OSS always reasons), Low/Medium/High as named.

## Failure states

| Code                            | Meaning                                                               |
| ------------------------------- | --------------------------------------------------------------------- |
| `LOCAL_MODEL_NOT_READY`         | the model file is missing or fails verification                       |
| `MODEL_IMPORT_ONLY`             | the catalog has no download source for this artifact                  |
| `MODEL_CHECKSUM_MISMATCH`       | an imported file is not the catalog's file                            |
| `LOCAL_RUNTIME_UNAVAILABLE`     | the required server build is not installed                            |
| `RUNTIME_IDENTITY_MISMATCH`     | the server build or context differs from the profile                  |
| `TEMPLATE_IDENTITY_MISMATCH`    | the served chat template differs from the profile                     |
| `BACKEND_PROFILE_MISMATCH`      | a profile was named by a different artifact                           |
| `LOCAL_RUNTIME_OUT_OF_MEMORY`   | the qualified configuration did not fit in GPU or system memory       |
| `LOCAL_RUNTIME_GPU_UNAVAILABLE` | the CUDA build could not use the GPU                                  |
| `LOCAL_RUNTIME_TIMEOUT`         | loading exceeded the profile's startup limit                          |
| `LOCAL_RUNTIME_FAILED`          | the server exited during load or failed warm-up                       |
| `LOCAL_RUNTIME_BUSY`            | a different model is mid-generation                                   |
| `CONTEXT_OVERFLOW`              | the prompt and output budget do not fit                               |
| `CONTEXT_CHECK_FAILED`          | the server could not render or tokenize the prompt                    |
| `GENERATION_TRUNCATED`          | the model hit its output limit; any partial text is marked incomplete |
| `EMPTY_ANSWER`                  | the model finished without text                                       |
| `MODEL_OUTPUT_INVALID`          | the answer contained protocol markers and was not accepted            |
| `MALFORMED_TOOL_CALL`           | a tool call could not be parsed                                       |
| `STREAM_TIMEOUT`                | the stream was silent past the policy's idle limit                    |
| `REQUEST_CANCELLED`             | the user stopped the request                                          |

Tool-level denials (`TOOL_NOT_ENABLED`, `PERMISSION_DENIED`,
`DUPLICATE_CALL_ID`, `TOOL_LOOP_LIMIT`, `INVALID_TOOL_ARGUMENT`,
`MEMORY_NOT_FOUND`, `ATTACHMENT_NOT_GRANTED`) are host-authored tool results:
the model sees them and the turn continues.

## Provenance

The `done` event of a successful answer carries, and the chat stores:
backend profile ID, constitution ID, tool protocol, provider kind, model ID,
execution location, reasoning effort, and for local models the artifact ID and
SHA-256, runtime build (`b11270-748d4225b`), template SHA-256, model repository
and revision, and context size. Message details show the runtime and effort;
developer mode adds the hashes.

## Grounding

Juniper has no retrieval or browsing tool. The runtime section tells the model
so, and the constitution tells it to say when something needs checking rather
than invent it. The interface marks any link, DOI, or arXiv ID in an answer as
model-written and unchecked unless a host tool result in the same reply
contains it. Links are already rendered inert. A future retrieval tool would
add host-authored sources; the model's own claims would still be unverified.
