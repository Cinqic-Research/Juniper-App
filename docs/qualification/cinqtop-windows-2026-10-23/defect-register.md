# Windows defect register, CINQTOP qualification (2026-10-23)

Baseline under test: the published `Juniper-0.4.0-alpha.1-windows-x86_64.msi`
(release commit `96ee95d384383f64bd958f282732fca780fbaddb`, SHA-256
`9477aecbac413310c7e52c6a7e456e0b943ba9f811a4eaccd517a1ca5847391d`), installed
on CINQTOP (see [hardware-qualification.md](hardware-qualification.md)).
Candidate: branch `engineering/cinqtop-windows-qualification` built on the
same machine. Reproduction steps are the ones a person performs in the
installed application unless stated otherwise. Severity: Critical blocks a
documented feature for every Windows user; High leaves processes or data
behind; Medium degrades ordinary use; Low is cosmetic or diagnostic.

"Verified" below means observed on CINQTOP in the candidate binary.
"Compiled only" means the change passes `cargo check`, `cargo clippy
--release --all-targets -D warnings` and `cargo fmt` but could not be executed
on this machine (W-13), so it still needs the Linux native tests, the Windows
CI smoke, and a run on Windows hardware.

| Id   | Severity | Area             | Summary                                                                                               | Candidate status                  |
| ---- | -------- | ---------------- | ----------------------------------------------------------------------------------------------------- | --------------------------------- |
| W-01 | Critical | Model management | Main-thread stack overflow when an installed model is listed; the app exits silently                  | Fixed, verified; regression test  |
| W-02 | Medium   | Desktop shell    | Main window opens under the taskbar and off-centre on a 1366x768 display                              | Fixed, verified; unit tests       |
| W-03 | Low      | Agent projects   | Granted folders are shown and stored with the `\\?\` verbatim prefix                                  | Fixed, compiled only; unit test   |
| W-04 | Low      | Models screen    | Native failure reasons replaced by generic "Download failed." style text                              | Fixed; frontend test              |
| W-05 | Low      | Models screen    | "Free storage" does not change after a download, import or removal                                    | Fixed, verified; frontend test    |
| W-06 | Medium   | Local runtime    | A terminal window opens in front of Juniper whenever `llama-server.exe` starts                        | Reproduced; fixed, verified       |
| W-07 | High     | Local runtime    | `llama-server.exe` (up to gigabytes) outlives a killed or crashed Juniper                             | Reproduced; fixed, verified       |
| W-08 | Medium   | Local runtime    | Every model listing and every server start re-hashes each model file on the main thread               | Reproduced; fixed, verified       |
| W-09 | —        | Agent plugins    | Local MCP server launch on Windows (static finding)                                                   | Not reached; fixture prepared     |
| W-11 | Low      | Documentation    | README, CHANGELOG and interface guide said 0.4.0-alpha.1 was "not yet published"                      | Fixed                             |
| W-12 | Low      | Build            | The runtime build fails on a fresh Windows machine without Git long-path support                      | Documented                        |
| W-13 | —        | Environment      | Smart App Control blocks unsigned, newly built executables on consumer Windows 11                     | Documented; owner decision        |
| W-14 | Low      | Desktop shell    | A second Juniper instance exits silently (`stage agent-runtime-lock failed`)                          | Recorded, not changed             |
| W-15 | Low      | Models screen    | Non-ASCII characters double-encoded by the W-04 edit (showed "Â·")                                    | Fixed, verified                   |
| W-16 | High     | Model licensing  | Accepting a model license fails on Windows ("Access is denied"), so gated models cannot be downloaded | Reproduced; fixed, native tests   |
| W-10 | Medium   | Native tests     | `cargo test` aborts on Windows with `STATUS_ENTRYPOINT_NOT_FOUND` before any test runs                | Root cause found; fixed, verified |

## W-01 Main-thread stack overflow when an installed model is listed

- Expected: the Models screen lists installed models; a completed download
  appears as installed.
- Actual: `juniper.exe` exits without a message. Event Viewer records
  "Application Error" for `juniper.exe 0.4.0.0`, exception `0xc00000fd`,
  fault offset `0xfbcfa7`, followed by a Windows Error Reporting APPCRASH.
  With standard error captured, the last line is
  `thread 'main' has overflowed its stack`.
- Reproduction (baseline): install the MSI, complete onboarding, open Models,
  download SmolLM2 360M Instruct (271 MB). The app exits as the download
  completes (01:52:09). Relaunch, open Models: the app exits again (02:04:58).
  Reproduced twice; the fault offset is identical each time.
- Root cause: `managed_models::hash_existing` allocated its 1 MiB read buffer
  on the stack. `commands::managed_models` is a synchronous Tauri command,
  which Tauri runs on the main thread, and the main thread's stack on Windows
  is 1 MiB. Linux and Android main threads are larger, so the same code
  passed there. On Windows the verification cache was also disabled (W-08),
  so every listing hashed every model.
- Fix: the buffer is a heap `Vec` (`HASH_BUFFER_BYTES`). Regression test
  `managed_models::tests::verification_runs_on_a_small_stack` verifies a
  3 MiB file from a thread with a 512 KiB stack; with the old code the test
  process aborts on every platform.
- Verified in the candidate: Models lists the installed model; a 1.1 GB
  download completed with another model installed; no exit.

## W-02 Main window under the taskbar and off-centre

- Expected: the window opens centred and fully inside the usable screen area
  (`tauri.windows.conf.json` sets 1000x700 with `center: true`).
- Actual (1366x768, 100% scaling, 48 px taskbar): outer rectangle
  (26,26)-(1042,765) on first launch and (104,29)-(1120,768) on later
  launches, so the bottom 28-48 px, including part of the Chat composer, sit
  behind the taskbar; the window is never centred.
- Root cause: 1000x700 client plus the 39 px frame is taller than the 720 px
  work area; the window system places the oversize window at a default
  cascade position and the `center` request has no visible effect.
- Fix: `window::fit_main_window` shrinks the client area to the monitor's
  work area (keeping the frame) and centres the window there, after the
  window exists. Unit tests cover the arithmetic for CINQTOP's geometry, a
  large display, a secondary monitor offset and underflow.
- Verified in the candidate: client 1000x681 at (183,31), outer
  (175,0)-(1191,720).

## W-03 Verbatim path prefix in project roots

- Expected: a granted folder is shown as `C:\Users\…\JuniperTestProject`.
- Actual: `\\?\C:\Users\…\JuniperTestProject` (the form `canonicalize`
  returns on Windows) is stored and displayed.
- Fix: `workspace::plain_path` stores and shows the ordinary spelling for
  drive and UNC paths; other verbatim forms are left unchanged. Existing
  roots keep their stored form; both forms open the same folder. The
  candidate binary predates this change, so it is compiled only.

## W-04 and W-05 Models screen messages and free storage

Recorded in the 0.4.0-Alpha CHANGELOG as known limitations. Tauri delivers a
command's `Err(String)` to the webview as a string, not an `Error`, so every
`error instanceof Error ? error.message : fallback` site in `ModelsScreen`
showed the fallback. `failureText` now keeps the host's sentence (without its
`CODE:` prefix). The device line is refreshed after a download, import or
removal. Frontend test: "shows the native reason for a failed download and
rechecks free storage afterwards". W-05 verified in the candidate (46 GB to
45 GB after a download); W-04 covered by the test only, since no download
failed during the run.

## W-06 Terminal window when the runtime starts

- Expected: no window other than Juniper's.
- Actual: the first chat with a local model opens a terminal window in front
  of Juniper, large enough to cover it; it stays for the life of the model.
  Evidence (`w06-console-window-evidence.txt`): `llama-server.exe` has a
  `conhost.exe` child and a top-level `PseudoConsoleWindow`; Windows
  Terminal hosts it. Reproduced on the first load and again on a model
  switch. This matches the owner's September report of a console behind the
  application.
- Root cause: release Juniper is a GUI-subsystem process, so Windows gives
  each console-subsystem child a new console unless the child is created with
  `CREATE_NO_WINDOW`. No spawn in the code set it; redirecting stdio does not
  prevent the console.
- Fix: `child::without_console` sets `CREATE_NO_WINDOW` for the runtime
  server, its `--version` and `--list-devices` probes, `ollama create` for a
  GGUF import, and local MCP servers. Compiled only.

## W-07 Runtime outlives a killed Juniper

- Expected: the local runtime ends with Juniper, as `PR_SET_PDEATHSIG`
  arranges on Linux.
- Actual (`w07-orphan-evidence.txt`): five seconds after
  `Stop-Process -Force` on `juniper.exe`, `llama-server.exe` was alive with a
  1.96 GB working set and answered `/health`; it had to be terminated by hand.
  A crash of Juniper (W-01 was one) leaves the same orphan.
- Fix: `child::end_with_parent` assigns the runtime and MCP servers to a job
  object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; the kernel closes the
  job when Juniper's process ends for any reason. Compiled only.

## W-08 Re-hashing on every listing and start

- Expected: a model verified once in a session is not hashed again while its
  size, modification time and change time are unchanged (the Linux
  behaviour).
- Actual: on Windows `changed_nanos` returned `None`, so `verify_against`
  hashed the whole file on every `managed_models` call and every server
  start. Measured on CINQTOP: 1.0 s per 258 MB (`Get-FileHash`, three runs),
  so about 4 s for the 1.1 GB model and 8 s for a 2 GB model, on the main
  thread, each time Models is opened or a chat starts the runtime.
- Fix: the NTFS change time read through `GetFileInformationByHandleEx`
  backs the record on Windows, and the precision probe runs there too. The
  trust note on change time is in the code. Compiled only.

## W-09 Local MCP servers on Windows

Static finding from the code review: the restricted child `PATH` is
`C:\Windows\System32;C:\Windows`, bare commands are searched without
`.exe`/`.cmd`, and `env_clear` removes `TEMP`, `APPDATA` and `COMSPEC`. Not
reproduced: the application session was spent on the model and agent paths
before Smart App Control ended binary execution. A fixture server
(`JuniperTestProject/mcp/echo-server.mjs`, one `echo` tool, Node stdio) is
ready; register it as a local plugin with the absolute `node.exe` path and
use the plugin's Test action.

## W-10 Native test loader

Reproduced once Smart App Control was off: the harness binary
`juniper_lib-<hash>.exe` exits with `0xc0000139` before `running N tests`.
Every import of the binary was checked against the exports of the DLL it
resolves to; one is missing: `TaskDialogIndirect` from `comctl32.dll`. That
function exists only in Common Controls 6, the side-by-side assembly an
executable gets when its application manifest requests it. tauri-build puts
such a manifest into the resource it links into binaries
(`cargo:rustc-link-arg-bins`), so `juniper.exe` loads Common Controls 6, but
the test harness is a separate executable with no manifest and binds to the
System32 `comctl32.dll` (version 5), which has no `TaskDialogIndirect`.

Fix: `build.rs` keeps the resource without the manifest and asks the linker
to embed `windows-app.manifest` (the same content) into every linked target
with `/MANIFEST:EMBED` and `/MANIFESTINPUT`, on MSVC targets only. Verified
with `mt.exe`: both the test binary and the packaged `juniper.exe` carry the
Common Controls 6 dependency (the linker also adds `requestedExecutionLevel
asInvoker`), and `cargo test` runs the suite (302 tests).

Running the suite on Windows for the first time then found W-16 and three
test-only Windows assumptions, all fixed on this branch: a fake server
read from a socket that Winsock had left non-blocking after `accept`; two
storage tests deleted a SQLite file while its connection was still open;
and the Windows symlink fixture needs `SeCreateSymbolicLinkPrivilege`, which
ordinary accounts lack without Developer Mode, so that test now says it was
not run instead of failing on the fixture (CI runners are elevated and run
it).

## W-11 Documentation

`README.md`, `CHANGELOG.md` and `docs/product/interface.md` described
`0.4.0-alpha.1` as not yet published and listed the `0.3.0-rc.33` files for
download six days after the release. Corrected; historical records in
`docs/release/` and `docs/phases/` are unchanged.

## W-12 Runtime build on a fresh Windows machine

`scripts/build-llama-runtime.sh` checks out the pinned llama.cpp commit,
whose tree contains paths over 260 characters. Git for Windows refuses them
(`Filename too long`) unless `core.longpaths` is set; GitHub's runners set it
machine-wide, which is why CI never saw it. Documented in
`docs/release/windows-troubleshooting.md` with a process-scoped setting.

## W-13 Smart App Control

CINQTOP's Smart App Control switched from evaluation to on during the run
(`SAC_PreviousState=2`, `VerifiedAndReputablePolicyState=1`,
`SAC_EnforcementReason=1`; the first block is logged at 03:00:46 as Code
Integrity event 3033 for a cargo build script). From then on every newly
built unsigned executable is refused with "An Application Control policy has
blocked this file" (os error 4551); executables that existed before the
switch, including the installed MSI and the 02:57 candidate, still run. Two
consequences:

- For this assignment: no further native build could be executed on
  CINQTOP; see the candidate report for what that leaves unverified.
- For the product: Smart App Control is on by default on new consumer
  Windows 11 installations once its evaluation passes. An unsigned Juniper
  MSI is then not installable and `juniper.exe` not runnable without the
  user turning the protection off permanently. Authenticode signing is the
  only remedy that does not ask users to lower a protection. This is a
  release-readiness item for the owner, not something this branch changes.

## W-14 Second instance

Starting `juniper.exe` while another instance holds `agent-runtime.lock`
writes `stage agent-runtime-lock failed` to stderr and exits. Nothing is
shown to the user and the existing window is not raised. Recorded for a
later decision; not changed here.

## W-15 Double-encoded characters in the Models screen

Introduced on this branch by the W-04 change, not present in
0.4.0-alpha.1: the file was rewritten through a shell that read it as
Windows-1252, so "·", "…", "’" and "–" were committed double-encoded and the
packaged candidate rendered "Â·" between model facts. Found by running the
packaged build, restored by reversing the encoding, and checked by comparing
the file's non-ASCII character inventory with the original. The frontend
tests did not catch it because none asserts those separators.

## W-16 Model license acceptance fails on Windows

- Expected: accepting a model's license in the dialog records the acceptance
  and the download starts (Nemotron 3 Nano 4B is the catalog entry that
  requires acceptance in 0.4.0-alpha.1).
- Actual (installed candidate, and by code the published MSI): after "Accept
  and download" the Models screen shows "Access is denied. (os error 5)" and
  nothing is downloaded. The native tests
  `model_licenses::tests::a_required_license_is_refused_until_accepted_and_bound_to_its_revision`
  and `native_download_and_import_refuse_a_gated_model_without_acceptance`
  fail the same way on Windows (`MODEL_LICENSE_STORE_ERROR: Access is
denied`), which W-10 had hidden.
- Root cause: `model_licenses::write` wrote the staging file, then opened it
  read-only and called `sync_all`. On Windows `FlushFileBuffers` needs a
  handle with write access, so the flush fails and the record is never
  renamed into place. Linux allows the flush on a read-only descriptor.
- Fix: the staging file is written and flushed through the same writable
  handle, then renamed. The two existing native tests cover it on Windows.
