# Changelog

Public names and machine versions differ: "Juniper 0.4.0-Alpha" is machine
version `0.4.0-alpha.1`, tag `v0.4.0-alpha.1`, Android versionCode 4001 and MSI
version 0.4.0.1. Earlier prereleases (`0.3.0-rc.N`) are recorded in
[docs/release/](docs/release/) and on the
[releases page](https://github.com/Cinqic-Research/Juniper-App/releases).

## Unreleased

Windows repairs from the CINQTOP qualification
([docs/qualification/cinqtop-windows-2026-10-23/](docs/qualification/cinqtop-windows-2026-10-23/README.md)):

- Fixed: on Windows the application exited with a stack overflow whenever an
  installed model was listed (a model download completing, or opening Models
  with a model installed). The verification hash used a 1 MiB stack buffer on
  the 1 MiB main thread. Every Windows user of 0.4.0-alpha.1 was affected.
- Fixed: `llama-server.exe`, `ollama` and local MCP servers no longer open a
  console window of their own on Windows.
- Fixed: on Windows the local runtime and local MCP servers are placed in a
  job object, so they end with Juniper even when it is killed.
- Fixed: the main window is fitted to the monitor's work area and centred, so
  it no longer opens under the taskbar on small displays.
- Fixed: the model verification cache now works on Windows (NTFS change time),
  so a listed or loaded model is not re-hashed on every use.
- Fixed: Agent project roots are stored and shown as `C:\...` rather than
  `\\?\C:\...`.
- Fixed: the Models screen shows the host's reason when a download, import,
  unload or removal fails, instead of "Download failed.", and refreshes the
  free-storage figure after a download, import or removal.
- Docs: README, CHANGELOG and the interface guide no longer describe
  0.4.0-alpha.1 as unpublished; Windows setup and troubleshooting are in
  `docs/release/windows-troubleshooting.md`.

## Juniper 0.4.0-Alpha (`0.4.0-alpha.1`) — published 2026-10-17

An alpha prerelease, not a stable release. Gates and evidence:
[docs/release/juniper-0.4.0-alpha-readiness-review.md](docs/release/juniper-0.4.0-alpha-readiness-review.md);
publication record:
[docs/release/juniper-0.4.0-alpha-publication-record.md](docs/release/juniper-0.4.0-alpha-publication-record.md).

### Agent (Beta)

- A fourth destination beside Chat, Models and Settings. Give it a goal and it
  works through it with the tools and folders you allow: project folders
  (read, search, and writes previewed as a diff), web search and page reading
  with source-linked citations, documents it saves, skills, optional MCP tools,
  sandboxed Python on Linux (bubblewrap), bounded helper subtasks, and
  schedules that run while Juniper is open.
- It asks before changing anything, shows a plan and every action, and can be
  paused, steered, cancelled, or stopped entirely. Writes to dependency
  manifests, lock files, build files and wrappers, .NET project files and
  scripts ask every time, with a diff and no standing grant; shell start-up,
  SSH, autostart, service, version-control, editor and CI files are refused;
  hidden folders, Windows `AppData` and macOS `~/Library` cannot be granted as
  project folders.
- A task whose answer claims a saved or changed file that no tool produced ends
  as **Needs review**, never Completed. The check is a text heuristic and can
  miss phrasings; the activity list shows what actually ran.
- MCP is Beta and tools-only (2026-07-28 and 2025-11-25 servers, stdio and
  Streamable HTTP); local MCP servers are not sandboxed. There is no
  background agent service.
- Database schema v6 (agent tables; schedule recovery). The upgrade keeps a
  restorable backup of the previous database.

### Model lifecycle

- Downloads are resumable with HTTP range requests and a transfer journal, can
  be multi-file (sharded) bundles, and are no longer capped at 2 GiB. Every
  file's size and SHA-256 is verified before the model is published to the
  library; one transfer runs at a time, and starting another pauses the first.
- Models whose license must be accepted show the terms first; the native host
  records the acceptance (not the web view). Terms known only by a fingerprint
  of a mutable page are asked again on every transfer and never saved.
- You choose the quantization for each catalog release; "Popular on Hugging
  Face" labels come from a dated download snapshot and are not a quality
  rating.

### Catalog

- 23 entries (26 artifacts): 22 downloadable releases across Qwen, DeepSeek-R1
  distills, Mistral, Phi, SmolLM, Granite, MiniCPM and Nemotron, plus
  import-only gpt-oss-20b. 0.3.0-rc.33 had four models below 1B.
- **Held from this alpha:** Llama 3.2 1B Instruct, Llama 3.2 3B Instruct,
  Llama 3.1 8B Instruct, Gemma 3 1B IT and Gemma 3 4B IT. Their GGUF files come
  from third-party mirrors of weights whose publishers attach their own licence terms, and no owner
  decision on those specific mirrors is recorded (other catalog entries also use community
  mirrors; the hold covers only Llama 3.x and Gemma 3);
  Meta publishes no GGUF, and Google's official GGUFs are gated on Hugging
  Face, which Juniper's account-free download cannot use. A GGUF you obtained
  yourself can still be imported. Detail:
  [exclusions-and-blockers.md](docs/research/model-catalog-2026-10-15/exclusions-and-blockers.md).
- The Juniper 1 local import is a research experiment, not a qualified or
  production model and not Juniper's default. Its public release is
  [Juniper 1 Base](https://github.com/Cinqic-Research/Juniper-1-Base),
  research-only.

### Platforms

| Platform          | Package             | Status                                                                                                                      |
| ----------------- | ------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Linux x86_64      | `.deb`, `.AppImage` | Supported for this alpha. Launch verified on X11; native Wayland not verified. Not signed (`SHA256SUMS.txt`, attestations). |
| Windows 10/11 x64 | `.msi`              | Supported, unsigned: SmartScreen warns. Agent on Windows is not verified; sandboxed code is unavailable there.              |
| Android 7.0+      | `.apk`              | Beta. Signed. A physical ARM64 inference run is still required before Android inference leaves Beta.                        |
| macOS, iOS        | —                   | Not built.                                                                                                                  |

### Known limitations

- Backup restore has not been verified on a clean machine.
- Project-folder grants made before the root guard existed are not re-checked;
  remove and re-add them.
- The Models screen shows a generic "Download failed." that hides the native
  reason, and its "Free storage" figure does not refresh after a download.
