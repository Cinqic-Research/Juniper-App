# Windows candidate test report: engineering branch on CINQTOP

| Field    | Value                                                                                                                                                                                                                                                                                                                                                                |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source   | branch `engineering/cinqtop-windows-qualification` from `main` `fc1f1d7937995ca68d14e8df1682f6869ae90d8b`                                                                                                                                                                                                                                                            |
| Binary   | `src-tauri/target/release/juniper.exe` built 02:57 on CINQTOP from the tree at that moment: the W-01 hash fix, the W-02 window fit and the W-03 path change, plus the unmodified rest of `main`. It does not contain the later Rust changes (W-06, W-07, W-08). SHA-256 `914110eeb7dcd55d416d55828316e569cb5aa9fa2b290615dd3d68df2ac9f687`.                          |
| Runtime  | `llama-server.exe` built on CINQTOP from the pinned commit `e107984bc` (`version: 0.3.0-dev (build 10788)`, MSVC 19.44), SHA-256 `19cf42de771110f3162cbb37cc0551acb64b2cbac75de9acb720a66e8b726512`                                                                                                                                                                  |
| Frontend | Served to the binary by the Vite dev server (`node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 1420`) from the branch's `src/`, so the W-04 and W-05 frontend changes were in effect. A plain `cargo build --release` yields a binary that loads `devUrl`; the packaged form needs `pnpm tauri build`, which could not be executed here (see Constraints). |
| Profile  | The baseline profile from the MSI run (`%APPDATA%\com.cinqic.juniper`), with SmolLM2 360M already installed                                                                                                                                                                                                                                                          |
| When     | 2026-10-23, 03:19 to 03:47 local time, on battery (the charger was unplugged during the night; see hardware report for the power caveat)                                                                                                                                                                                                                             |
| Who      | Claude Fable 5.1 as implementer (engineering self-check)                                                                                                                                                                                                                                                                                                             |

## Constraints that shaped this run

- **Smart App Control** on CINQTOP moved from evaluation to on at about
  02:58 (`SAC_PreviousState=2`, `VerifiedAndReputablePolicyState=1`). From
  then on Windows refuses to run any newly built unsigned executable (cargo
  build scripts, test binaries, a trivial MSVC `hello.exe`); binaries that
  existed before the switch still run. Turning it off is permanent and is
  the owner's decision, so no later build was executed on this machine. The
  later Rust changes are validated by `cargo check`, `cargo clippy
--release --all-targets -D warnings` and `cargo fmt`, and must be run on
  Linux (native tests) and on the Windows CI runner (MSI build and smoke)
  before they count as verified. See the defect register for each item's
  verification state.
- The display was locked for part of the night; runs during the lock are
  excluded from timing.

## Results

| Area          | Case                                                                                             | Result  | Notes                                                                                                                                                       |
| ------------- | ------------------------------------------------------------------------------------------------ | ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Desktop shell | Window placement on 1366x768 (W-02)                                                              | PASS    | client 1000x681, outer (175,0)-(1191,720): centred, entirely above the taskbar                                                                              |
| Models        | Open Models with an installed model (W-01)                                                       | PASS    | lists "SmolLM2 360M Instruct, Downloaded and verified"; no exit                                                                                             |
| Models        | Download Qwen2.5 1.5B Instruct Q4_K_M (1.1 GB), completion with a model already installed (W-01) | PASS    | 10-12 MB/s; "downloaded and verified"; no exit                                                                                                              |
| Models        | Pause and resume                                                                                 | PASS    | paused at 18% ("A partial download is saved"), resumed and continued from 41%                                                                               |
| Models        | Free storage refresh after a download (W-05)                                                     | PASS    | 46 GB before, 45 GB after, without Recheck                                                                                                                  |
| Models        | Use as default, model switch                                                                     | PASS    | the previous `llama-server.exe` was terminated and a new one started for the new model                                                                      |
| Chat          | First message with SmolLM2 360M, bundled runtime, no Ollama                                      | PASS    | "The capital of France is Paris."; `llama-server.exe` 658 MB working set                                                                                    |
| Chat          | Second turn continuity                                                                           | PASS    | "Paris has a population of approximately 2.2 million people." (context kept, no re-introduction)                                                            |
| Chat          | Qwen2.5 1.5B after the switch                                                                    | PASS    | "Three planets in our solar system are Mercury, Venus, and Earth."; about 90 s from send to answer on this CPU (hash, load, prompt processing)              |
| Local runtime | Console window on runtime start (W-06)                                                           | FAIL    | reproduced twice: `llama-server.exe` gets a `conhost.exe` child and a pseudo-console window; a terminal window opens in front of Juniper                    |
| Local runtime | Runtime ends when Juniper is killed (W-07)                                                       | FAIL    | reproduced: after `Stop-Process -Force` on `juniper.exe`, `llama-server.exe` (1.96 GB) stayed alive and answered `/health`; WebView2 processes ended        |
| Local runtime | Re-hash on every listing or start (W-08)                                                         | FAIL    | by code and by measurement: SHA-256 of 258 MB takes 1.0 s on this drive; no cache on Windows in this binary                                                 |
| Agent         | Capabilities disclosure                                                                          | PASS    | Sandboxed code unavailable with the Linux-only reason; project folders, web, local MCP, scheduling available                                                |
| Agent         | Unqualified model gate                                                                           | PASS    | "Not qualified for tools"; task start refused until "Try this model anyway" is enabled in the agent settings                                                |
| Agent         | Task with SmolLM2 360M on the fixture project                                                    | PASS\*  | queued, running, completed; the model answered with a template and used no tool (model limitation)                                                          |
| Agent         | Task with Qwen2.5 1.5B on the fixture project                                                    | PASS\*  | used the document tool ("Saved 'Total Quantity Report'", stored under `%APPDATA%\com.cinqic.juniper\agent\artifacts\`), answered "120" (file totals 55)     |
| Agent         | Steering ("Add to this task") and continuation                                                   | PASS    | the follow-up was queued and run; the model then printed a fake `artifact.read` call as text, which the host displayed and did not execute                  |
| Agent         | Project folder untouched by tasks                                                                | PASS    | all five fixture files unchanged (hashes compared before and after)                                                                                         |
| Agent         | Workspace file read and diff-previewed write on Windows                                          | NOT RUN | neither model called the workspace tools, so the Windows file-read and approval paths were not exercised; no tool-qualified catalog model fits this machine |
| Agent         | "Needs review" heuristic                                                                         | NOTE    | both wrong answers ended "Completed"; the documented heuristic limitation                                                                                   |
| Agent         | Task history persisted                                                                           | PASS    | both tasks listed under Recent after navigation                                                                                                             |
| Lifecycle     | Second instance while one runs                                                                   | NOTE    | reports `stage agent-runtime-lock failed` on stderr and exits; no window or message for the user                                                            |

PASS\* means the Windows runtime behaved correctly while the small model's
answer was wrong; the model is not qualified for tools and the interface said
so.

## Not verified on CINQTOP

- The packaged MSI of the candidate (needs `pnpm tauri build`, blocked by
  Smart App Control), its installation, uninstall and reinstall.
- W-06, W-07 and W-08 fixes at run time (the binary with them cannot be
  executed here); they compile and pass clippy.
- Native unit tests (`cargo test`): the debug profile's build scripts are
  blocked; this also prevented diagnosing W-10 at run time.
- MCP stdio servers (W-09): not reached before the lock-out; the
  `JuniperTestProject/mcp/echo-server.mjs` fixture is ready for the reviewer.
- Backup and restore into a fresh profile; schema upgrade from an older
  database (the profile was created fresh by 0.4.0-alpha.1 at schema v6).
- Display scaling other than 100%.

## Packaged build run (later on 2026-10-23, after Smart App Control was turned off)

The owner turned Smart App Control off in the morning, so a packaged build
could be executed on CINQTOP after all. `pnpm tauri build --bundles msi --
--locked` at `4646ed3` produced `Juniper_0.4.0-alpha.1_x64_en-US.msi`
(16,257,024 bytes) in 14 minutes. The binary from that build
(`src-tauri/target/release/juniper.exe`, embedded frontend, same bytes the
MSI installs) was run directly while the native test suite compiled in the
background, so the timings below carry CPU contention.

| Case                                          | Result  | Notes                                                                                                                                                                                 |
| --------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Baseline MSI uninstall (UAC consent)          | PASS    | `C:\Program Files\Juniper`, shortcuts and the Apps entry removed; database and both model files kept                                                                                  |
| Launch, embedded frontend                     | PASS    | window 1.2 s, `frontend ready` 2.7 s, 398 MB working set, window (175,0)-(1191,720), 0 console windows                                                                                |
| Chat with Qwen2.5 1.5B on the bundled runtime | PASS    | "Ready."                                                                                                                                                                              |
| W-06: console window while the runtime runs   | PASS    | process tree: `juniper.exe`, six WebView2 processes, `llama-server.exe` (2.1 GB) with a hidden `conhost.exe`; no console or pseudo-console window (`w06-candidate-console-check.txt`) |
| W-07: runtime ends with a killed Juniper      | PASS    | `llama-server.exe` gone within 1.4 s of `Stop-Process -Force` (`w07-candidate-kill-test.txt`)                                                                                         |
| W-08: second Models listing uses the cache    | PASS    | first open: device line still "Checking" at 1.5 s, listed by 9.5 s (1.4 GB hashed, machine busy); second open: listed within 1.5 s                                                    |
| Candidate MSI install                         | NOT YET | two attempts ended with status 1602: the UAC prompt was not approved within its two-minute window                                                                                     |

A regression was found in this run and fixed before any review: the
failure-text change had been written back through a shell that read the file
as Windows-1252, so every "·", "…", "’" and "–" in the Models screen was
double-encoded and the candidate showed "Â·" between model facts (W-15).

### Installed candidate MSI (12:05 onwards)

The rebuilt MSI (`Juniper_0.4.0-alpha.1_x64_en-US.msi` from `89b6d6e` plus
the restored Models screen) installed after UAC consent in 10 s. It carries
a `juniper_lib.dll` beside `juniper.exe` that the CI-built 0.4.0-alpha.1 MSI
did not; the local build directory holds the crate's cdylib output and the
bundler picked it up. Harmless, 2.5 MB, noted for the reviewer.

| Case                                                        | Result | Notes                                                                                                         |
| ----------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------------- |
| First launch after install (machine busy with `cargo test`) | PASS   | window 1.6 s, `frontend ready` 3.0 s, 406 MB working set, 0 console windows, window fitted above the taskbar  |
| W-03: newly granted root shows the plain path               | PASS   | `C:\Users\…\JuniperTestProject\src` (read only); the root granted by the baseline keeps its stored `\?\` form |
| Hidden-folder root refusal (`.cache` in the profile)        | PASS   | "Juniper does not grant a hidden configuration folder in your home folder to a project."                      |
| W-15: separators in the Models screen                       | PASS   | "·" renders correctly                                                                                         |
| License acceptance for Nemotron 3 Nano 4B                   | FAIL   | "Access is denied. (os error 5)" after Accept and download (W-16, fixed on the branch after this build)       |

### Native test suite on Windows

With the loader fix (W-10) the suite runs: the serialized run reports 287
passed, 6 failed, 9 ignored in 63 s. One failure is W-16; four are
test-only Windows assumptions (fixed on the branch); one needs the symlink
privilege. The parallel run additionally ended abnormally once
(`0xffffffff`) because the proxy fixture's server thread panicked under
Winsock's non-blocking inheritance, which the same fix removes.

With the fixes on the branch the suite passes on Windows: 293 passed, 0
failed, 9 ignored in 16 s on an idle machine (`cargo test --locked`). Under
heavy load (the application, clippy and the tests at once on four efficiency
cores) one wall-clock assertion,
`juniper1::tests::slow_runtimes_time_out_and_cancellation_stops_the_request`
(cancellation under 1.2 s), can exceed its budget; it passes alone every
time and is not Windows-specific.

### Final build timings (binary of the MSI built from `812300c`, on battery, 12:31)

| Launch                       | Window visible | `frontend ready` | Process-tree working set | Console windows | Children left after exit |
| ---------------------------- | -------------- | ---------------- | ------------------------ | --------------- | ------------------------ |
| Cold (first after the build) | 1.7 s          | 4.3 s            | 371 MB                   | 0               | 0                        |
| Warm 1                       | 0.25 s         | 1.7 s            | 367 MB                   | 0               | 0                        |
| Warm 2                       | 0.21 s         | 1.6 s            | 367 MB                   | 0               | 0                        |

Baseline MSI for comparison (same machine, mains power, first run busy with a
toolchain install): cold 5.2 s / 6.8 s, warm 0.4-0.6 s / 1.5-2.4 s, 380-401
MB. The candidate is not slower at launch and uses no more memory.
