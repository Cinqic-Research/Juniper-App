# Testing strategy

`pnpm validate` is the canonical validation command. It runs formatting, lint,
TypeScript, frontend tests, Rust formatting, Clippy with `-D warnings`, Rust
tests, JSON schema checks, version consistency, license and runtime contracts,
and branding checks in one sequence.

`pnpm typecheck` checks `tsconfig.app.json` and `tsconfig.node.json`
explicitly. Before `0.3.0-rc.32` it ran `tsc --noEmit` against the solution
`tsconfig.json`, whose `files` list is empty, so it type-checked nothing and
an unimported identifier reached a release.

A gate passes only when it has exercised the property it claims. Surviving a
timeout, finding a file by name, or matching a source checksum is not
evidence that a shipped artifact works or carries the right branding.

## Deterministic tests

Frontend tests cover the native startup path under a mocked Tauri runtime
(stored Ollama discovery, the frontend-ready report, and the root error
boundary, no save after a failed load, and a visible save failure), the zero-model shell and navigation, assistant
import/export, context order and truncation, private-chat exclusion from both
persistence and user export, attachment metadata privacy, model-fit estimates,
markdown rendering, and browser-preview streaming.

Interface tests (`src/app/App.test.tsx`) cover the three primary destinations,
reachability of every rc.32 surface through Settings, per-chat assistant
avatars and composer names, routing a chat through its own assistant's model,
model switching without rewriting attribution, overflow actions, confirmed
deletion, in-app rename, private-chat export restrictions, progressive
disclosure of reasoning, tools, and usage, touch versus keyboard composer
behaviour, the sidebar preference, the default assistant, per-assistant tool
policy, and that density, width, font, text size, message style, contrast, and
motion change the rendered document. `src/lib/settings.test.ts` loads a
realistic rc.32 state fixture (`src/test/fixtures.ts`) and checks field-by-field
migration and rejection of malformed values. `src/lib/appearance.test.ts`
sweeps curated and generated accents through light, dark, and high contrast
and asserts WCAG ratios for every text, focus, and fill token.

Conversation-continuity tests in `src/lib/context.test.ts` verify that the
current user message remains separate from retained user/assistant history,
that prior repetitive assistant wording is preserved as history rather than
imitated by the context builder, that the context distinguishes a first turn
from an ongoing exchange, and that the default prompt and personality bands
provide directness and warmth without defining warmth as greeting or emoji
repetition. `src/lib/storage.test.ts` verifies exact stock-prompt migration
for the built-in assistant, preservation of customized prompts and unrelated
fields, isolation of custom assistants, and save/reload stability.

Nine negative controls for this work reintroduce a defect (global avatars, a
hard-coded composer name, unvalidated settings, an inert density token,
unconfirmed deletion, uncorrected accent text, cross-assistant model routing,
exportable private chats, and a missed motion migration); each makes the
corresponding test fail. The log is recorded in the rc.33 release record.

Native tests cover SQLite snapshot saves whose chats or models reference a
removed provider or model, exit cleanup of the bundled `llama-server` child,
the per-round bound on every host tool call, the safe calculator, unit conversion, host-authored
result shape, tool loop bounds, the default-deny tool gate, permission scope
matching, capability gating of generation controls, provider JSON/SSE/pull
parsing, fake HTTP discovery/inspection/chat/tool/error servers, unknown-model
behavior, timeout and cancellation, scoped attachment and GGUF grants,
read-time attachment revalidation, bounded runtime logs, restart-safe
attachment persistence, SQLite migrations across schema v1 to v4, runtime
registry maturity, artifact manifests, and Device Link pairing/framing/scope
policy.

Two native tests are `#[ignore]`d because they require a live Ollama service
and an owner-selected installed model. They are not counted as passes when
skipped.

`scripts/test-release-publication-guard.sh` runs the release workflow's
publication completeness step verbatim against a stub `gh`. It proves that a
complete publication passes, that a partial publication is repaired by
re-uploading only the missing assets, and that a publication whose uploads never
take effect fails. 0.3.0-rc.33 was published without three assets because the
publication action failed part way through its uploads while the release stayed
public, so a failed job alone is not treated as sufficient protection.

## Real-model qualification

Qualification runs against a real installed model, not a fixture:

```bash
JUNIPER_LIVE_OLLAMA_MODEL=<installed model> \
  cargo test --manifest-path src-tauri/Cargo.toml --lib -- --ignored --nocapture
```

The suites in `tests/qualification/` each declare an `applies_when` capability
gate. The harness reads the capabilities the runtime actually reports from
`/api/show` and reports a suite whose gate is unmet as NOT-APPLICABLE. A
capability a model does not have is never recorded as a pass. Recorded results
are in [../qualification/ollama-real-model-evidence.md](../qualification/ollama-real-model-evidence.md).

The standalone local path adds deterministic catalog, device-fit, symlink
rejection, and managed-state tests. A release build must additionally run the
pinned runtime build script and the Linux/Windows install smoke; those checks
prove that the Tauri resource exists and that the app-owned process can be
started without Ollama.

`tests/fixtures/qwen3-8b-qualification.yaml` is a historical fixture retained
for reference. It is not evidence of a real-model result.

The current conversation-continuity qualification is recorded separately in
[../qualification/qwen3-8b-conversation-continuity-2026-09-21.md](../qualification/qwen3-8b-conversation-continuity-2026-09-21.md).
It records the exact source/model/settings boundary and preserves residual
stochastic observations; deterministic tests do not claim to prove arbitrary
model wording.

## Platform validation

Linux bundles are built once on Ubuntu 22.04 and smoked on Ubuntu 22.04 and
24.04 under Xvfb (X11) in pull-request validation and in the release workflow.
`scripts/verify-linux-bundles.sh` launches the normal FUSE AppImage, the
`APPIMAGE_EXTRACT_AND_RUN=1` fallback, and the installed DEB through its desktop
entry. Each launch goes through `scripts/linux-launch-probe.sh`, which passes
only after Juniper reports `[juniper-startup] frontend ready`, a viewable
Juniper window is rendered with real content, no fatal diagnostic appears
during a settle period, and the process exits on SIGTERM. A timeout is always a
failure. Each mode runs from an empty profile and again with stored state that
enables an Ollama provider backed by a loopback stand-in (the rc.31 blank-window
state), and must persist the discovered model. The script also checks
architecture, an executable bundled `llama-server`, licenses, the desktop
entry, icons against the official artwork, and that purging the DEB removes
package files but keeps user data. `scripts/test-linux-launch-probe.sh` proves
the probe rejects a hung process, an early exit, a window that never becomes
ready, a blank surface, and a crash after readiness. No automated check covers
a native Wayland session.

Android builds start from a clean `tauri android init`, install the official
launcher icons, and verify them in the generated project and in the built APK
with `scripts/verify-android-branding.mjs` (pixel comparison of every launcher,
round, and adaptive-foreground density plus the adaptive XML and background).
The emulator smoke also resolves the launcher activity, checks that the
launcher drew the Juniper icon, opens Juniper from that launcher entry, and
requires the frontend-ready report and non-blank content.

Windows MSI bundling plus an install, launch, and uninstall smoke run on a
Windows runner in pull-request validation and again in the release workflow.
`scripts/verify-windows-msi.ps1` checks the MSI `ProductVersion`, installs it,
requires the installed `llama-server.exe`, `LICENSE`, and notices, and launches
Juniper twice: from a fresh profile and with stored state that enables an
Ollama provider backed by a loopback stand-in. Each launch must report
`[juniper-startup] frontend ready`, stay alive through a settle period, and
produce no fatal startup or frontend report; the second must persist the
discovered model. Before `0.3.0-rc.32` this smoke passed on ten seconds of
process survival. It does not capture window pixels, so a blank-but-ready
Windows surface is not detected. A signed Android APK is built and put through an
emulator credential-vault instrumentation run plus install, launch, rotation,
relaunch, and uninstall smoke. The credential test proves plaintext absence,
Android Keystore key ownership, per-reference AAD binding, deletion, and failed
post-deletion retrieval. The package audit also requires the two supported
native ABIs, rejects server-runtime libraries, and checks 16 KiB `PT_LOAD`
alignment for every shared object.
Release artifacts include the unstripped native symbol archive. The emulator
cannot prove real offline token generation, so a physical ARM64 run remains a
separate llama.cpp Beta-promotion follow-up. The validation and release
workflows do not make that physical run a global desktop release gate.

Native unit tests are excluded from the Windows release job only: the Tauri mock
runtime fails to load there, aborting the test binary at startup with
`STATUS_ENTRYPOINT_NOT_FOUND` before any test runs. Every compile-level check,
including `cargo clippy -D warnings`, still runs on Windows, and the native
tests run on Linux.

## Fault injection

The fault-injection review targets malformed provider records, tool calls the
request never enabled, unknown tool names, invalid arguments, private-chat
leakage, attachment IDs outside the picker grant set, denied permissions,
oversized results, and tool-loop overruns. These invariants are represented by
the provider, tool, storage, and frontend tests; a release reviewer should run
the same fixtures against any adapter change.
