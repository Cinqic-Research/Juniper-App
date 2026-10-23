# Independent-review handoff: CINQTOP Windows qualification and repairs

Repository `Cinqic-Research/Juniper-App`, branch
`engineering/cinqtop-windows-qualification`, base `main`
`fc1f1d7937995ca68d14e8df1682f6869ae90d8b`, draft pull request #54. The
branch tip is the commit that updates this handoff; `git log fc1f1d7..HEAD`
lists the rest. The same text is the pull request's description. Not
approved, not merged, no tag or release; the implementer never certifies
its own work.

## Purpose and scope

Qualify Juniper 0.4.0-alpha.1 on a real Windows 11 machine (CINQTOP, HP
14-dq6xxx, Intel N150, 16 GB, 1366x768) and repair the confirmed Windows
defects without changing Linux or Android behaviour, model licensing policy,
artifact verification or the agent's security boundaries.

## Environment

See [hardware-qualification.md](hardware-qualification.md). Toolchain
installed for the run: Git 2.55, Node 24.22 LTS, pnpm 11.19.0, Rust 1.90.0
MSVC via rustup, CMake 4.4.4, Visual Studio Build Tools 2022 17.14 (C++
workload, Windows SDK 10.0.26100); WebView2 153 was already present. Smart
App Control switched itself on during the night and was turned off by the
owner in the morning (W-13).

## Reproduced defects and root causes

| Id   | Defect                                                           | Root cause                                                                            |
| ---- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| W-01 | App exits when an installed model is listed (every Windows user) | 1 MiB stack buffer in `hash_existing` on the 1 MiB Windows main thread (sync command) |
| W-02 | Window under the taskbar, off-centre on 1366x768                 | Oversize configured window; `center` ineffective on Windows                           |
| W-03 | `\\?\` prefix in project roots                                   | `canonicalize` verbatim form stored and shown                                         |
| W-04 | "Download failed." hides the host reason                         | Tauri string rejections treated as non-`Error`                                        |
| W-05 | Free storage stale                                               | device line only read on mount                                                        |
| W-06 | Terminal window when the runtime starts                          | no `CREATE_NO_WINDOW` on child spawns from a GUI-subsystem parent                     |
| W-07 | `llama-server.exe` survives a killed Juniper                     | no Windows counterpart to `PR_SET_PDEATHSIG`                                          |
| W-08 | Model re-hashed on every listing and start                       | verification cache disabled on Windows (no change-time signal)                        |
| W-10 | `cargo test` aborts on Windows before any test runs              | test harness has no application manifest, binds to the old `comctl32.dll`             |
| W-11 | Docs say 0.4.0-alpha.1 unpublished                               | not updated after the release                                                         |
| W-12 | Runtime build fails on a fresh Windows machine                   | Git long paths off by default; CI runners have it on                                  |
| W-16 | Accepting a model license fails ("Access is denied")             | acceptance file flushed through a read-only handle                                    |

Full detail, including W-13 (Smart App Control), W-14 (silent second
instance) and W-15 (a regression introduced and fixed on this branch), in
[defect-register.md](defect-register.md).

## Changes

- `src-tauri/src/managed_models.rs`: heap hash buffer; NTFS change time for
  the verification record; probe runs on Windows; small-stack regression
  test.
- `src-tauri/src/window.rs`, `lib.rs`: fit and centre the main window in the
  monitor work area at startup (desktop only); unit tests.
- `src-tauri/src/child.rs`, `local_runtime.rs`, `commands.rs`,
  `agent/mcp.rs`, `Cargo.toml`, `Cargo.lock`: `CREATE_NO_WINDOW` on every
  spawn; job object with `KILL_ON_JOB_CLOSE` for the runtime and MCP
  servers; `windows-sys` added as a direct Windows dependency (already in
  the tree).
- `src-tauri/src/agent/workspace.rs`, `agent/commands.rs`: `plain_path` for
  stored and displayed roots; unit test.
- `src-tauri/build.rs`, `src-tauri/windows-app.manifest`: the linker embeds
  the Common Controls 6 manifest into every target, so the test harness
  loads; `.github/workflows/validation.yml` and `release.yml` run
  `cargo test` on Windows as a gating step; `docs/testing/strategy.md`.
- `src-tauri/src/model_licenses.rs`: the acceptance file is flushed through
  its writable handle.
- `src-tauri/src/commands.rs`, `providers.rs`, `storage.rs`,
  `local_runtime.rs` (tests only): Winsock non-blocking inheritance, SQLite
  files deleted while open, symlink privilege.
- `src/app/ModelsScreen.tsx` and its test: `failureText`, device refresh;
  one new frontend test; non-ASCII characters restored after an editing
  mistake (W-15).
- `README.md`, `CHANGELOG.md`, `docs/product/interface.md`,
  `docs/release/windows-troubleshooting.md` (new): release references,
  Windows page, Unreleased entries.
- `package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml`, `Cargo.lock`:
  Vite 7.3.7 and three RustSec lockfile updates the validation job's audits
  required.
- `docs/qualification/cinqtop-windows-2026-10-23/`: hardware qualification,
  baseline and candidate test reports, defect register, this handoff.

## Tests performed (engineering self-checks)

On CINQTOP at the final tree: `pnpm format`, `pnpm lint`, `pnpm typecheck`,
`pnpm test` (21 files, 157 tests, one new), `pnpm schema:validate`,
`pnpm version:check`, `pnpm license:check`, `pnpm runtime:check`,
`pnpm branding:verify`, `cargo fmt --check`,
`cargo clippy --release --locked --all-targets -- -D warnings`, and
`cargo test --locked` on Windows: 293 passed, 0 failed, 9 ignored (the
ignored tests need the canonical Juniper 1 tokenizer, a live Ollama, or a
real database copy). Under heavy load one wall-clock assertion
(`juniper1::tests::slow_runtimes_time_out_and_cancellation_stops_the_request`)
can exceed its 1.2 s budget; it passes alone every time.

Pull request CI at `2771ea9` (before the W-10 and W-16 commits): Windows
MSI build, install, render and uninstall smoke, both Linux launch smokes and
the Android compile passed; the validation job passed `pnpm validate` on
Linux and failed only on new dependency advisories, fixed since.

## Real Windows evidence

- Baseline MSI ([baseline-test-report.md](baseline-test-report.md)):
  installs, renders, onboarding, settings, appearance, accessibility, agent
  projects and refusals work; crashes on any installed model (reproduced
  twice, event log and WER captured); uninstall keeps user data.
- Candidate ([candidate-test-report.md](candidate-test-report.md)): three
  builds were run on the machine. The MSI built by `pnpm tauri build`
  installed after UAC consent. Verified on it or on its binary: Models lists
  installed models, a 1.1 GB download completes, pause and resume, free
  storage refreshes, chats with SmolLM2 360M and Qwen2.5 1.5B on the bundled
  runtime without Ollama with context kept across turns, model switching
  restarts the runtime, no console window while the runtime runs,
  `llama-server.exe` ends within 1.4 s of a kill, the second Models listing
  comes from the cache, the window is centred above the taskbar, new project
  roots show `C:\...`, hidden-folder and home-folder roots are refused,
  Agent tasks run, persist and accept steering with the fixture folder
  untouched, and the sandbox is disclosed unavailable.
- W-16 was reproduced in the installed app ("Access is denied" after
  accepting the NVIDIA terms). The fix is covered by the two native license
  tests on Windows; the in-app re-check on the final MSI is listed below.

## Before and after

| Measurement                                   | Before (MSI)                      | After (final build)                               |
| --------------------------------------------- | --------------------------------- | ------------------------------------------------- |
| Models screen with an installed model         | process exits                     | lists the model                                   |
| Window on 1366x768                            | (26,26)-(1042,765), under taskbar | (175,0)-(1191,720), centred above the taskbar     |
| Cold launch, window / frontend ready          | 5.2 s / 6.8 s (busy machine)      | 1.7 s / 4.3 s                                     |
| Warm launch, window / frontend ready          | 0.4-0.6 s / 1.5-2.4 s             | 0.2-0.25 s / 1.6-1.7 s                            |
| Idle working set, whole process tree          | 380-401 MB                        | 367-371 MB                                        |
| Runtime working set, SmolLM2 360M / Qwen 1.5B | n/a (unreachable)                 | 658 MB / 2.1 GB (`--cache-ram 1024`, 16k context) |
| Console windows while the runtime runs        | 1 (pseudo-console, visible)       | 0                                                 |
| `llama-server.exe` after Juniper is killed    | still running                     | gone within 1.4 s                                 |
| Second Models listing with 1.4 GB installed   | re-hashed (about 5 s)             | from cache, under 1.5 s                           |
| License acceptance                            | fails                             | native tests pass; in-app re-check pending        |
| `cargo test` on Windows                       | aborts at load                    | 293 passed                                        |

## Security implications

- No change to default-deny tool gating, root refusals, protected files,
  license policy or artifact verification. The NTFS change time can be set
  by a process that can already write the model file, which is inside the
  existing same-user trust boundary; the code comment records this.
- `CREATE_NO_WINDOW` and the job object only affect processes Juniper
  already starts. The embedded manifest adds `requestedExecutionLevel
asInvoker`, which the linker inserts; it matches the previous behaviour
  (no elevation).
- W-13: Smart App Control refuses the unsigned MSI and `juniper.exe` on
  consumer machines where it is on. Signing is the remedy; owner decision.

## Remaining limitations and outstanding validation

- In-app re-check of the license acceptance on the final MSI, and the
  uninstall and reinstall of that MSI (both need the owner present for
  UAC).
- The locally built MSI carries a `juniper_lib.dll` beside `juniper.exe`
  that the CI-built package did not; the bundler picked up the crate's
  cdylib output from the local target directory. Harmless, but the reviewer
  should confirm the CI artifact's contents.
- Agent workspace read and diff-previewed write not exercised (no
  tool-qualified model fits the machine); MCP stdio (W-09) not reached.
- Backup restore, schema upgrade from an older database, display scaling
  other than 100%, and Linux or Android behaviour of the shared changes
  (covered by CI, not by a run here).

## Independent verification

Steps in [README.md](README.md), "Reproducing on another Windows machine";
for W-10, run `cargo test --locked` on a Windows machine and confirm with
`mt.exe -inputresource:<test exe>;#1` that the harness carries the Common
Controls 6 dependency.
