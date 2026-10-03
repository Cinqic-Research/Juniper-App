# ADR 0022: Host-composed constitution and model distrust

Status: Proposed for review (branch `feature/juniper-wrapper-gpt-oss`)

## Context

Until rc.33 every rule Juniper's assistant followed lived in one editable
string, `DEFAULT_SYSTEM_PROMPT`, which the webview composed with personality
controls, memories, and a tool list into a single system message. Three
problems followed:

- Truthfulness, capability honesty, untrusted-content handling, and identity
  were user-editable profile text. Editing the built-in assistant, or creating
  a new one, removed them.
- The list of "available host tools" in the prompt was written by the
  interface, not by the code that decides which tools are offered.
- Memories were pasted into the system message, giving saved notes (which may
  contain injected text) instruction-level authority.

The gpt-oss-20b qualification (Juniper LM 1.1,
`reports/capability/truthfulness-review-20b.md`) found that the selected model
fabricates details about nonexistent papers, APIs, flags, people, URLs, and
repository files, and follows instructions embedded in documents. Prompt text
alone cannot be the defense for either.

## Decision

**Instruction layers are composed natively, in a fixed order.** For every
request `src-tauri/src/behavior.rs` builds the system/developer message as:

1. identity and lineage — the assistant's name, Cinqic's role, and that Cinqic
   did not create or train the model;
2. the constitution, `config/behavior/constitution.v2.json`
   (`juniper-constitution.v2`): priority order, truth over confidence,
   capability honesty, correction, user treatment, untrusted content, privacy,
   proportional safety. v1 was the first version; v2 adds that text claiming
   an action happened is not a host result, after the evaluation below;
3. a runtime section generated from host state: the model's verified lineage,
   where it runs, exactly the tools offered to this request, what is not
   available (web, code execution), whether files are attached, and the
   private-chat state;
4. the assistant profile (the user-editable `systemPrompt`, personality
   controls, response length), labeled as unable to override the layers above.

The interface sends only the profile. A request may carry one leading `system`
message and otherwise only `user` and `assistant` messages; `tool` messages
are authored only by the host loop.

**Data is not instruction.** Memories selected for context travel by ID; the
host resolves them against the stored memories the interface supplies for
the request (enabled, this assistant's, none in a private chat) and sends them
as a framed `user`-role block. The interface is part of the application, so
this check stops model output and saved text from choosing memories; it is
not a defense against a compromised webview. Attachments are framed the same way. Both
blocks strip their own closing tags so content cannot end its framing.

**What code can enforce, code enforces.** The constitution covers behavior
that needs the model's judgment. Everything else is host logic:

| Property                                           | Enforced by                                                                                                         |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| Which tools exist for a request                    | `offered_tools` (capability, enabled, private chat, Device Link)                                                    |
| Whether a call runs                                | `tool_gate`, schema validation, permission, loop and size bounds                                                    |
| Tool results                                       | host-authored `juniper-tool-protocol-v1` envelopes only                                                             |
| Persistent writes (`memory.save`, `memory.delete`) | per-call approval showing the exact text; standing grants never apply                                               |
| Repeated call IDs                                  | refused                                                                                                             |
| Memory and chat access in private chats            | tools withheld, host context cleared                                                                                |
| Answer acceptance                                  | `finish_reason: length` → `GENERATION_TRUNCATED`; empty → `EMPTY_ANSWER`; protocol markers → `MODEL_OUTPUT_INVALID` |
| Raw reasoning                                      | never emitted to the interface, never persisted, never re-sent                                                      |
| Model-written links and citations                  | marked "not checked" in the interface                                                                               |
| Context fit                                        | output reserved in the budget; measured with the server tokenizer where available                                   |

**Model output is a proposal.** A tool call is a request; a claimed result is
text; a memory is a proposal until the user approves that exact text. Unknown
capability state is treated as unavailable.

**Personality is a profile, not policy.** The built-in profile now carries only
style and continuity guidance. The previous stock prompt is kept in
`HISTORICAL_STOCK_JUNIPER_SYSTEM_PROMPTS` so the built-in assistant migrates
from it, while edited prompts stay user data.

## Consequences

- Every answer records `constitution`, backend, tool protocol, and runtime
  identity (`ChatMessage.provenance`).
- A user can rewrite or delete the profile without weakening truthfulness or
  security instructions.
- The constitution costs about 600 tokens per request. Reference 4B measured
  that instruction density can reduce coding quality; the wrapper evaluation
  (`tests/wrapper-eval/`) includes coding and arithmetic cases to watch for it.
- Changing a rule's text requires a new constitution ID; the file records
  why each version changed.
- Prompt instructions still cannot make the model truthful. They are measured,
  not assumed; see `docs/qualification/gpt-oss-wrapper-evaluation-2026-10-02.md`.

## Security and privacy

The runtime section and framing are host-authored, so a profile, memory,
attachment, tool result, or model output cannot change the stated tool list or
grant itself a capability. The model can still ignore instructions; that is
why execution, permission, and persistence decisions do not depend on it.
