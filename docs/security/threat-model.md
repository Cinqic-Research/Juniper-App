# Security threat model

| Threat                                | Control                                                                                                                                             |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fake model-authored tool result       | Only Rust host functions create `host_result`; model continuation is never trusted.                                                                 |
| Prompt injection in files/tool output | Content is data framed outside the host instructions; it cannot alter permissions, the offered tools, or the constitution (ADR-0022).               |
| Arbitrary file read                   | Opaque attachment grants, type allowlist, regular-file check, and 1 MiB cap.                                                                        |
| Shell injection                       | No shell command is exposed in default capabilities; GGUF import uses a fixed executable with separate validated arguments.                         |
| Secrets in logs/export                | Credentials never enter normal UI state, logs, or exports.                                                                                          |
| Dangerous Markdown                    | Renderer escapes all text and only creates controlled links with `target`/`rel`.                                                                    |
| Unbounded tool loop/payload           | Four rounds, eight calls per round, and 64 KiB argument/result budget.                                                                              |
| Local/remote confusion                | Locality is a typed profile property, shown on the model pill, in the model picker, in chat details, in Models, and in Settings › Privacy & data.   |
| Malformed assistant import            | Versioned schema validation and inert JSON parsing.                                                                                                 |
| Database corruption                   | SQLite migrations, foreign-key enforcement, and normalized entities.                                                                                |
| Untrusted model download              | Catalog entries are checked in, HTTPS-only, size-bounded, and SHA-256 verified before atomic installation.                                          |
| Substituted imported model            | Imports must match the catalog's size and SHA-256; the installed bytes are what is hashed; a qualified profile binds to one artifact hash.          |
| Unqualified runtime for a profile     | `--version` before load and `/props` after load must match the profile's build, served template hash, and context; no CPU or remote fallback.       |
| Path traversal or symlink escape      | Managed paths derive only from safe catalog variant IDs; symlinks are rejected for verification and resume.                                         |
| Local runtime network exposure        | Loopback-only port, per-launch API key from an owner-only file deleted after startup, web UI and slot endpoints off, `--offline`; no LAN bind.      |
| Resident runtime left running         | One server, stopped on exit and on unload; switching models waits for in-flight generations; a crashed server is restarted, not reused.             |
| Model-written memory                  | `memory.save`/`memory.delete` need per-call approval of the exact text shown by the host; standing grants never cover them; private chats get none. |
| Raw reasoning disclosure              | Analysis is never emitted to the webview, never persisted (dropped on native save and on load), and never re-sent.                                  |
| Fabricated links and citations        | Links are inert; answers with model-written URLs, DOIs, or arXiv IDs are marked as not checked.                                                     |
| Forged tool history from the webview  | Requests may not contain `tool` messages or non-leading `system` messages.                                                                          |

This document is a v0.3 self-review artifact, not an independent security audit.
