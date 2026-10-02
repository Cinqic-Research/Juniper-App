# ADR-0016: Standalone local runtime boundary

Status: accepted for the rc31 release candidate; Android llama.cpp remains Beta
pending physical qualification. The desktop per-generation server is
superseded by the resident runtime in ADR-0023 (proposed).

## Decision

Juniper owns the normal local inference lifecycle with explicit platform
boundaries. On desktop, release builds package a CPU-safe `llama-server` built
from the pinned `llama.cpp` revision in `scripts/build-llama-runtime.sh`. For
each desktop local chat request, Rust resolves a verified model from the
managed catalog, reserves a loopback port, starts the server with a
loopback-only bind, waits for `/health`, routes the request through Juniper's
normalized OpenAI-compatible adapter, and stops the child when the request
ends. `JUNIPER_LLAMA_SERVER` is a developer-only override.

On Android, the `juniper-local` Tauri plugin uses an in-process Kotlin/JNI
llama.cpp bridge instead of the desktop executable. Rust routes the Android
local provider to that adapter, which loads one SHA-256-verified managed GGUF
from app-private storage, streams through the existing chat event contract,
supports cancellation, and unloads on lifecycle or memory pressure. Android
never packages or starts `llama-server` and does not use localhost for local
inference. The production ABI is `arm64-v8a`; `x86_64` is retained for
bounded emulator tests.

Ollama is not probed and is never a fallback. External provider adapters stay
available under Advanced for users who intentionally configure them.

## Consequences

The ordinary desktop installation path needs no Ollama installation, daemon,
account, or model-specific source-code branch. Runtime binaries are generated
by the release build and ignored by git; model weights remain user-owned,
downloaded on demand, and verified before use. The desktop implementation uses
a short-lived server per generation, which favors isolation and simple cleanup
over warm-server latency. Android keeps one application-scoped native engine
warm across turns and unloads it deterministically when required.

Android's native path is implemented and remains Beta: the physical ARM64
offline first prompt, warm reuse, cancellation, lifecycle, and low-memory
evidence has not yet been recorded. Emulator or package evidence must not be
promoted to a stable runtime claim, but this follow-up does not block the
desktop release contract.

## Security and privacy

The desktop runtime binds to `127.0.0.1`, receives only a trusted catalog model
path, does not accept shell fragments from model metadata, and reports a
structured `LOCAL_RUNTIME_UNAVAILABLE` error when its resource is missing. The
Android bridge stays in process and reports structured native/runtime errors
without exposing prompt or response content. No silent network or Ollama
fallback is permitted on either path.
