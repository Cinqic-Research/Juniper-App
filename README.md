# Juniper

Juniper is a local-first AI desktop and Android app for people who want a thoughtful assistant, visible model controls, and a clear boundary around private data. It works with compatible text-generation models through supported runtimes; no model family is required and no model is bundled.

**Version `0.3.0-rc.33` — release candidate.** The guarded release workflow builds and packages the Juniper-owned local runtime for desktop targets only after the validation and immutable-tag gates pass. See [Download](#download).

## Download

The current prerelease is [Juniper 0.3.0-rc.33](https://github.com/Cinqic-Research/Juniper-App/releases/tag/v0.3.0-rc.33). Installers are published on the [GitHub releases page](https://github.com/Cinqic-Research/Juniper-App/releases) only after an authorized release workflow completes. You do not need Git, a build toolchain, or a GitHub account to use a published desktop artifact.

| Platform          | File                                        | Notes                                                 |
| ----------------- | ------------------------------------------- | ----------------------------------------------------- |
| Windows 10/11 x64 | `Juniper-0.3.0-rc.33-windows-x86_64.msi`    | Unsigned; the bundled local runtime is Juniper-owned. |
| Linux x86_64      | `Juniper-0.3.0-rc.33-linux-x86_64.AppImage` | `chmod +x`, then run. Needs FUSE (`fuse3`).           |
| Linux x86_64      | `Juniper-0.3.0-rc.33-linux-x86_64.deb`      | `sudo apt install ./Juniper-...deb`.                  |
| Android 7.0+      | `Juniper-0.3.0-rc.33-android-universal.apk` | Signed; Android llama.cpp remains Beta.               |

Verify a download against `SHA256SUMS.txt` from the same release:

```bash
sha256sum --check --ignore-missing SHA256SUMS.txt
```

`SIGNING-android.txt` records the APK signing certificate and `SIGNING-windows.txt` records the MSI Authenticode status. Every executable artifact also carries a [GitHub artifact attestation](https://github.com/Cinqic-Research/Juniper-App/attestations) linking it to the workflow run and commit that produced it.

Juniper’s desktop bundle owns its loopback `llama-server` process, so ordinary local use does not require Ollama, a daemon, or an account. On first run, open **Models**, review the device-aware recommendations, and download a verified model. Model weights are separate user-owned files and are never bundled in the installer.

## What Juniper does

- A chat-first interface: Chats, Models, and Settings are the only primary destinations. Conversation actions live in an overflow menu, the model is a compact picker that shows where each model runs, and reasoning, tool activity, and usage are one tap away instead of always on screen. See [docs/product/interface.md](docs/product/interface.md).
- A chat workspace with onboarding, streaming, markdown, export, private chats, and phone layouts with a history drawer and Android back-gesture support.
- Validated appearance settings: System, Light, or Dark; standard or high contrast; curated, neutral, or custom accent colors with computed WCAG-safe text shades; bundled Inter, Atkinson Hyperlegible Next, and OpenDyslexic fonts; interface size, chat text size, line spacing, density, conversation width, sidebar behavior, message style, timestamps, and motion.
- Assistant profiles with personality controls, model selection, tool policy, memory policy, import, and export.
- A first-class Juniper local provider with device detection, model recommendations, verified resumable downloads, atomic installation, pause/resume, and removal.
- A generic runtime registry with explicit maturity and qualification state for llama.cpp, LiteRT-LM, ExecuTorch, MLC LLM, and ONNX Runtime GenAI. Optional runtimes are not presented as installed without a compatible artifact.
- Artifact-centric model metadata: each model points to concrete runtime artifacts and file manifests, so a model is never treated as universally compatible.
- A non-networked Device Link protocol/policy preview. Pairing, discovery, peer connections, remote control, and Juniper Network transport are disabled until a persistent TLS identity and end-to-end transport are implemented and qualified.
- A curated catalog of four instruction-tuned GGUF models below 1B parameters, with source revision, license, size, and SHA-256 shown before download.
- Optional Ollama, OpenAI-compatible, and llama.cpp-compatible provider connections for advanced users and existing setups.
- A deterministic context builder that keeps the system prompt, curated memories, enabled tool definitions, and recent conversation within a context budget.
- A host-authored tool boundary with bounded calculator, unit conversion, datetime, attachment, memory, search, and system-info contracts. A tool the request did not enable is denied, never executed.
- Native SQLite persistence with migrations, OS-keychain credential storage on desktop, scoped text attachments, diagnostics, and privacy settings.

## What Juniper does not do

These are deliberate exclusions in this release, not oversights:

- **No Ollama dependency.** Ollama remains an optional external provider and legacy import path; it is not probed or used as a fallback by the Juniper local provider.
- **Desktop runtime provenance.** The release workflow builds the pinned `llama.cpp` server from source and places it in the Tauri resource slot. A source checkout needs CMake and uses `scripts/build-llama-runtime.sh` before a local bundle can run the native provider.
- **No MCP client.** Settings › Advanced lists it as unavailable.
- **No arbitrary themes.** Appearance is limited to validated settings; Juniper never loads user CSS, scripts, or remote fonts.
- **Android provider secrets use Android Keystore.** Juniper refuses to fall back to plaintext or SQLite storage; the instrumentation test exercises encrypt, decrypt, and deletion on an Android target.
- **Android native local inference is Beta.** The candidate packages an in-process llama.cpp bridge for the managed, SHA-256-verified GGUF path on `arm64-v8a` (with `x86_64` reserved for emulator tests). The owner reports successful local model loading and inference on a physical Android phone. Build identity, device details, and the full lifecycle test record were not captured, so Beta promotion remains pending.
- **No iOS or macOS build.**
- **No telemetry, analytics, crash reporting, or account.**

## Privacy and security

- Telemetry is off and there is no analytics or crash-reporting code. Juniper makes network requests only to provider endpoints you configure and, when you choose to download a model in Models, to the pinned HTTPS Hugging Face URLs listed in `config/models/catalog.json`. At startup it contacts only an enabled Ollama provider you added, to refresh its model list.
- Juniper labels every route as ON DEVICE, LOCAL NETWORK, REMOTE, or UNKNOWN, and never treats UNKNOWN as safe.
- Desktop provider credentials are stored in the OS keychain; Android provider credentials use Android Keystore. Both are referenced only by an opaque identifier. Secrets are never written to the SQLite state and are stripped from exports.
- Device Link is a protocol/policy preview only in this candidate. No listener, usable pairing flow, discovery, peer connection, remote-control path, or Juniper Network provider is enabled.
- Private chats are held in memory only; they are excluded from persistence and from user exports.
- Desktop attachments use a scoped native picker; Android uses its WebView document chooser. Both paths enforce the supported text-extension allowlist, strict UTF-8 decoding, and a 1 MiB cap. Native reads also reject symlinks, and attachment content is labeled untrusted to the model.

See [PRIVACY.md](PRIVACY.md), [SECURITY.md](SECURITY.md), and [docs/privacy/network-policy.md](docs/privacy/network-policy.md).

## Build from source

Install Node.js 22+, pnpm 11, Rust 1.90+, CMake, Git, and the platform prerequisites in the [Tauri prerequisite guide](https://tauri.app/start/prerequisites/).

```bash
pnpm install --frozen-lockfile
pnpm validate      # canonical check: format, lint, types, tests, clippy, schemas, version
scripts/build-llama-runtime.sh
pnpm tauri dev     # run the desktop app
pnpm tauri build   # produce a native bundle
```

`pnpm dev` runs a browser-only UI preview. It cannot reach a provider and uses a clearly-marked deterministic fake responder; it is a development aid, not a way to use Juniper.

Individual checks: `pnpm build`, `pnpm test`, `pnpm lint`, `pnpm format`, `pnpm schema:validate`.

For Android, run `pnpm tauri android init --ci`, then `pnpm android:branding` before `pnpm tauri android build`. A fresh init always writes Tauri's default launcher icons; `android:branding` installs and verifies the official Juniper set.

Repository layout:

```text
src/           React UI, state, context, storage, browser preview
src-tauri/     Tauri commands, providers, SQLite, keychain, tools
config/        checked-in defaults and reference provider/tool data
schemas/       versioned assistant, provider, tool, and export schemas
scripts/       version, schema, and platform bundle verification
tests/         model qualification suites and fixtures
docs/          architecture, ADRs, privacy, security, release, phase history
```

## Testing and qualification

`pnpm validate` runs the frontend and native test suites, static checks, schema validation, and version checks. See [docs/testing/strategy.md](docs/testing/strategy.md).

The historical provider qualification can still run against a locally installed Ollama model:

```bash
JUNIPER_LIVE_OLLAMA_MODEL=qwen3:0.6b \
  cargo test --manifest-path src-tauri/Cargo.toml --lib -- --ignored --nocapture
```

Suites whose capability gate the model does not meet are reported NOT-APPLICABLE, never as passes. That evidence covers the optional Ollama adapter; the standalone runtime is qualified separately by release artifact smoke and a model download/inference run.

## Known limitations

- The Windows MSI may be unsigned, so Windows SmartScreen can show an unrecognized-publisher warning.
- `0.3.0-rc.33` is a prerelease candidate and is not promoted to final `0.3.0`. Its publication record, including a publication step that failed after creating the release and how the three missing assets were completed from the same workflow run, is in [docs/release/rc33-publication.md](docs/release/rc33-publication.md).
- Android native local inference is Beta: use a supported ARM64 phone for real offline inference; x86_64 emulator evidence is diagnostic when hardware acceleration is unavailable. The owner-reported physical run establishes basic feasibility, while missing build and lifecycle metadata limits maturity.
- Linux launch is tested on Ubuntu 22.04 and 24.04 under X11 (Xvfb) for the normal AppImage, the extract-and-run fallback, and the installed DEB. Native Wayland sessions are not verified; the AppImage forces GTK's X11 backend. If Juniper does not start, run it from a terminal and see [docs/release/linux-troubleshooting.md](docs/release/linux-troubleshooting.md).
- `0.3.0-rc.33` reorganises navigation; [docs/product/interface.md](docs/product/interface.md) maps every rc.32 screen to its new place. Stored rc.32 settings migrate automatically.
- The rc.33 interface layout was reviewed on Linux (X11) and an Android x86_64 emulator. A later owner-reported physical Android local-inference run lacks version and device metadata. The published rc.33 Windows smoke did not inspect rendered pixels; the owner subsequently reported an unusable black window. Current Windows qualification is tracked separately from that historical release evidence.
- `0.3.0-rc.31` shipped the default Tauri Android launcher icon and showed a blank Linux window when an enabled Ollama provider had installed models; both are fixed in `0.3.0-rc.32`.
- Android loopback addresses refer to the phone itself. Reaching a computer on your network needs an explicit LAN endpoint.
- Browser-preview attachments are development-only; the real attachment path is the desktop native picker.

## Contributing and history

See [CONTRIBUTING.md](CONTRIBUTING.md). Architecture and decisions are in [docs/architecture/](docs/architecture/) and [docs/adr/](docs/adr/). Release records are in [docs/release/](docs/release/). The `docs/phases/` directory holds dated point-in-time development reports; treat this README and [docs/release/](docs/release/) as authoritative where they differ.

## License

Juniper is released under the Apache License, Version 2.0. See [LICENSE](LICENSE). Bundled third-party components retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
