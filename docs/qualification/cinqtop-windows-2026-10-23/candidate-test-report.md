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
