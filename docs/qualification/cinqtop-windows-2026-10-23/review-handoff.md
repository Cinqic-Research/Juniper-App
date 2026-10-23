# Independent-review handoff: CINQTOP Windows qualification and repairs

Repository `Cinqic-Research/Juniper-App`, branch
`engineering/cinqtop-windows-qualification`, base `main`
`fc1f1d7937995ca68d14e8df1682f6869ae90d8b`. The code and record commits end
at `6bb5d5d301dfb659fd9b640b54414892e4aafcbe`; the branch then adds this
handoff and two dependency updates for the audits the validation job runs:
Vite 7.3.7 (GHSA-587h-7wmj-h5pm) and the ctutils, rustls-webpki and
webpki-roots lockfile entries (RUSTSEC-2026-0385, -0382, -0381). The same text is the draft pull request's description. Not
approved, not merged, no tag or release; the implementer never certifies
its own work.

## Purpose and scope

Qualify Juniper 0.4.0-alpha.1 on a real Windows 11 machine (CINQTOP, HP
14-dq6xxx, Intel N150, 16 GB, 1366x768) and repair the confirmed Windows
defects without changing Linux or Android behaviour, model licensing,
artifact verification or the agent's security boundaries.

## Environment

See [hardware-qualification.md](hardware-qualification.md). Toolchain
installed for the run: Git 2.55, Node 24.22 LTS, pnpm 11.19.0, Rust 1.90.0
MSVC via rustup, CMake 4.4.4, Visual Studio Build Tools 2022 17.14 (C++
workload, Windows SDK 10.0.26100); WebView2 153 was already present.

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
| W-11 | Docs say 0.4.0-alpha.1 unpublished                               | not updated after the release                                                         |
| W-12 | Runtime build fails on a fresh Windows machine                   | Git long paths off by default; CI runners have it on                                  |

Full detail: [defect-register.md](defect-register.md).

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
- `src/app/ModelsScreen.tsx` and its test: `failureText`, device refresh;
  one new frontend test.
- `README.md`, `CHANGELOG.md`, `docs/product/interface.md`,
  `docs/release/windows-troubleshooting.md` (new): release references,
  Windows page, Unreleased entry.
- `docs/qualification/cinqtop-windows-2026-10-23/`: hardware qualification,
  baseline and candidate test reports, defect register, this handoff.

## Tests performed (engineering self-checks)

On CINQTOP at `6bb5d5d`: `pnpm format`, `pnpm lint`, `pnpm typecheck`,
`pnpm test` (21 files, 157 tests, one new), `pnpm schema:validate`,
`pnpm version:check`, `pnpm license:check`, `pnpm runtime:check`,
`pnpm branding:verify`, `cargo fmt --check`,
`cargo clippy --release --locked --all-targets -- -D warnings`,
`cargo check --release --locked --all-targets`: all pass.

Not run on CINQTOP: `cargo test` and the packaged MSI build, because Smart
App Control switched on mid-run and blocks newly built unsigned executables
(W-13). The reviewer should run `pnpm validate` on FLOWBOX (the new Rust
tests are platform-independent) and let the Windows CI job build and smoke
the MSI.

## Real Windows evidence

- Baseline MSI ([baseline-test-report.md](baseline-test-report.md)):
  installs, renders, onboarding, settings, appearance, accessibility, agent
  projects and refusals work; crashes on any installed model (reproduced
  twice, event log and WER captured).
- Candidate ([candidate-test-report.md](candidate-test-report.md); binary
  built 02:57 with W-01, W-02 and W-03, frontend served by Vite): Models
  lists installed models, a 1.1 GB download completes, pause and resume
  work, free storage refreshes, chats with SmolLM2 360M and Qwen2.5 1.5B run
  on the bundled runtime without Ollama with context kept across turns,
  model switching restarts the runtime, Agent tasks run, persist and accept
  steering with the fixture folder untouched; the sandbox is disclosed
  unavailable. W-06, W-07 and W-08 were reproduced on that binary; their
  fixes are compiled only.

## Before and after

| Measurement                                   | Before (MSI)                       | After                                                                |
| --------------------------------------------- | ---------------------------------- | -------------------------------------------------------------------- |
| Models screen with an installed model         | process exits                      | lists the model (verified)                                           |
| Window on 1366x768                            | (26,26)-(1042,765), under taskbar  | (175,0)-(1191,720), centred above the taskbar (verified)             |
| Cold launch to frontend ready                 | 6.8 s (first launch, busy machine) | not comparable: candidate ran through the dev server                 |
| Warm launch to frontend ready                 | 1.5-2.4 s                          | not comparable (same reason); must be measured on the packaged build |
| Idle working set, whole process tree          | 380-401 MB                         | 350-463 MB in dev-server mode                                        |
| Runtime working set, SmolLM2 360M / Qwen 1.5B | n/a (unreachable)                  | 658 MB / 2.1 GB (`--cache-ram 1024`, 16k context)                    |
| Hash per listing (W-08)                       | 1.0 s per 258 MB, every time       | once per process after the fix (compiled only)                       |

## Security implications

- No change to default-deny tool gating, root refusals, protected files,
  license checks or artifact verification. The NTFS change time can be set
  by a process that can already write the model file, which is inside the
  existing same-user trust boundary; the code comment records this.
- `CREATE_NO_WINDOW` and the job object only affect processes Juniper
  already starts.
- W-13: Smart App Control refuses the unsigned MSI and `juniper.exe` on
  consumer machines where it is on. Signing is the remedy; owner decision.

## Remaining limitations and outstanding validation

- W-06, W-07 and W-08 fixes need a Windows run of a `pnpm tauri build` MSI.
- Uninstall and reinstall of the candidate MSI (UAC) not run; the baseline
  MSI is still installed on CINQTOP for that test.
- Agent workspace read and diff-previewed write not exercised (no
  tool-qualified model fits the machine); MCP stdio (W-09) not reached.
- W-10 (`STATUS_ENTRYPOINT_NOT_FOUND`) not diagnosed.
- Backup restore, schema upgrade from an older database, and display
  scaling other than 100% not tested.
- Timing after the change must be re-measured on the packaged build, on
  mains power.

## Independent verification

Steps in [README.md](README.md), "Reproducing on another Windows machine".
