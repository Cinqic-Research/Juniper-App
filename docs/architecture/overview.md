# Juniper architecture

```text
React UI
  ↓ normalized use cases
Juniper state and context builder
  ↓
Rust Tauri commands / orchestrator boundary
  ├─ runtime registry and artifact-centric model catalog
  ├─ instruction layers (constitution, runtime section) and backend policies
  ├─ resident local runtime (one Juniper-owned loopback llama-server)
  ├─ provider adapters (Juniper local, Ollama, OpenAI-compatible, llama.cpp)
  ├─ Device Link preview policy (no listener, pairing UI, or transport)
  ├─ host tool runtime and permissions
  ├─ SQLite repositories and migrations
  ├─ OS credential store
  └─ scoped attachment/runtime operations
```

Models supply inference. Assistants supply behavior. The UI never depends on
raw Juniper-local, Ollama, or OpenAI response shapes; `ChatStreamEvent` is the
normalized stream contract.

The browser preview is intentionally useful without a native runtime: it
uses a deterministic fake stream and clearly says that a real provider is not
connected. In the Tauri shell, provider requests happen behind Rust commands.

## Data flow

1. The webview builds the assistant profile, selects curated memories and
   history within the context budget, and sends them with the current message.
2. The native host composes the identity, constitution, and runtime section in
   front of the profile and frames memories and attachments as data
   ([wrapper.md](wrapper.md), ADR-0022).
3. A backend policy shapes the request for the model family; the provider
   adapter receives normalized messages and only the generation parameters it
   knows the provider can accept (ADR-0023).
4. Model tool calls remain untrusted until offered-tool, schema, and
   permission checks complete. Only host implementations create tool results.
5. An answer is accepted only if it finished normally and carries no protocol
   text; raw reasoning never reaches the webview.
6. Persistence uses normalized tables, not a provider-specific message blob.

## Platform boundary

Desktop can connect to external local providers, launch the Juniper-owned
loopback runtime, and use the OS keychain. Mobile shares the UI/domain model and
catalog management path, uses Android Keystore for provider secrets, and uses
an in-process Kotlin/JNI llama.cpp bridge for the managed Android local
provider. The Android bridge is implemented for `arm64-v8a` (with `x86_64` test
packaging) and is reported as Beta until physical-device qualification; that
promotion follow-up does not block core desktop release gates.
