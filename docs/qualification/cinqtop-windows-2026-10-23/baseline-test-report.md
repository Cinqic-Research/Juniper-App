# Windows baseline test report: published 0.4.0-alpha.1 MSI on CINQTOP

| Field          | Value                                                                                                                                                                                                                        |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Artifact       | `Juniper-0.4.0-alpha.1-windows-x86_64.msi`, 14,917,632 bytes, SHA-256 `9477aecbac413310c7e52c6a7e456e0b943ba9f811a4eaccd517a1ca5847391d` (matches `SHA256SUMS.txt` and `RELEASE-PROVENANCE.json`, release commit `96ee95d3`) |
| Signing        | `Get-AuthenticodeSignature`: NotSigned, as `SIGNING-windows.txt` states                                                                                                                                                      |
| MSI properties | ProductVersion 0.4.0.1, Manufacturer Cinqic, ALLUSERS=1, x64, ProductCode `{5D2B5D46-18DB-4ACC-B86A-41DDE26652B1}`                                                                                                           |
| Machine        | CINQTOP, [hardware-qualification.md](hardware-qualification.md); mains power from 01:46, battery before                                                                                                                      |
| Profile        | Fresh: no `%APPDATA%\com.cinqic.juniper` existed before the first launch                                                                                                                                                     |
| When           | 2026-10-23, 01:46 to 02:14 local time (US Eastern)                                                                                                                                                                           |
| Who            | Claude Fable 5.1 as implementer (engineering self-check, not an independent review)                                                                                                                                          |
| Method         | Real installation, real window interaction through desktop automation on the same display, PowerShell probes for process, window and event-log facts. No IPC commands, page scripts or database writes were used to act.     |

The MSI copy under test had no Mark of the Web (it was fetched with
PowerShell rather than a browser), so SmartScreen did not appear; the UAC
consent prompt did, naming an unknown publisher.

## Results

| Area          | Case                                                                         | Result  | Notes                                                                                                                                                                      |
| ------------- | ---------------------------------------------------------------------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Installation  | Silent `msiexec /qn` from a non-elevated prompt                              | FAIL    | 1603 / 1925 (no elevation possible when the UI level is silent). Documented behaviour, not a product defect.                                                               |
| Installation  | `msiexec /passive` with UAC consent                                          | PASS    | 10 s; `C:\Program Files\Juniper` (35 MB: `juniper.exe`, `runtime\llama-server.exe`, LICENSE, notices); Start-menu and public-desktop shortcuts; Apps entry Juniper 0.4.0.1 |
| Installation  | Installed executable subsystem                                               | PASS    | `juniper.exe` PE subsystem 2 (GUI); `llama-server.exe` subsystem 3                                                                                                         |
| Launch        | First launch, empty profile                                                  | PASS    | window visible 5.2 s, `frontend ready` 6.8 s (cold, toolchain install running concurrently); 401 MB working set across 7 processes; 0 console windows                      |
| Launch        | Warm relaunch                                                                | PASS    | window 0.4-0.6 s, ready 1.5-2.4 s                                                                                                                                          |
| Launch        | Startup diagnostics                                                          | PASS    | the four `[juniper-startup]` lines on stderr; invisible without redirection (documented in `docs/release/windows-troubleshooting.md`)                                      |
| Desktop shell | Window placement on 1366x768                                                 | FAIL    | W-02: bottom 28-48 px under the taskbar, not centred                                                                                                                       |
| Desktop shell | Minimum-size layout (client forced to 360x601)                               | PASS    | phone layout with bottom navigation; readable                                                                                                                              |
| Desktop shell | Keyboard shortcut Ctrl+Shift+S                                               | PASS    | sidebar toggles                                                                                                                                                            |
| Onboarding    | Five-step welcome, Continue/Back/Enter Juniper                               | PASS    |                                                                                                                                                                            |
| Appearance    | Light, Dark, accent swatch, persistence across restart                       | PASS    | Dark plus blue accent survived close and relaunch                                                                                                                          |
| Accessibility | High contrast, back to System                                                | PASS    |                                                                                                                                                                            |
| Settings      | General, Appearance, Accessibility, Advanced, Diagnostics                    | PASS    | Diagnostics: SQLite schema v6, os windows, x86_64; no memory line on Windows (`system_info` memory is Linux-only)                                                          |
| Models        | Device line                                                                  | PASS    | "CINQTOP, Memory: 8.4 GB available, Free storage: 49 GB, Local engine: Loads on first chat"                                                                                |
| Models        | Recommended tab, filters, download of SmolLM2 360M (271 MB)                  | PASS    | 9 MB/s, progress and Pause control shown; file complete and SHA-256 matches the catalog                                                                                    |
| Models        | App survives download completion                                             | FAIL    | W-01: exits at completion (stack overflow, event log `0xc00000fd`)                                                                                                         |
| Models        | Open Models with an installed model                                          | FAIL    | W-01: exits every time                                                                                                                                                     |
| Chat          | Model picker with an installed model, after relaunch                         | BLOCKED | "No models ready yet": the installed model is never registered because W-01 ends the process first                                                                         |
| Chat          | Send, stream, cancel, continuity, export, private chat                       | BLOCKED | no usable local model (W-01); no external provider configured                                                                                                              |
| Agent         | Permissions tab, "Where agent work runs"                                     | PASS    | "Sandboxed code: unavailable. Sandboxed code runs on Linux in this beta. This platform has no verified sandbox yet." Web and MCP shown available                           |
| Agent         | New project, grant a folder (native picker), read-only and read-write grants | PASS    | picker is the Windows common dialog; grant stored and shown as `\\?\C:\...` (W-03)                                                                                         |
| Agent         | Refuse the user profile folder as a root                                     | PASS    | "Juniper does not grant your home folder (choose a project folder inside it) to a project."                                                                                |
| Agent         | Remove a grant (confirmation dialog)                                         | PASS    |                                                                                                                                                                            |
| Agent         | Project persists across restart                                              | PASS    |                                                                                                                                                                            |
| Agent         | Task execution, approvals, Needs review                                      | BLOCKED | needs a model (W-01)                                                                                                                                                       |
| Data          | Database created (`juniper.db`, 344 KB), `agent-runtime.lock`, `models\`     | PASS    |                                                                                                                                                                            |
| Data          | Recovery after the crash                                                     | PASS    | relaunch opens cleanly; settings and project intact                                                                                                                        |
| Lifecycle     | Close from the title bar                                                     | PASS    | exit within 0.6 s; no `juniper.exe`, `llama-server.exe` or Juniper WebView2 processes left                                                                                 |
| Lifecycle     | Uninstall / reinstall                                                        | NOT RUN | needs a UAC consent while the owner was away; see the candidate report                                                                                                     |

Passes, failures and blocked cases are what the installed application did on
this machine. "Blocked" means the case could not be reached because of an
earlier failure, not that it failed.

## Evidence

Under the session's evidence folder (not committed; file names listed so a
reviewer can ask for them): `install-0.4.0-alpha.1.log` (verbose MSI log),
`baseline-fresh-launch1-*` (stderr, report, render), `baseline-relaunch-*`,
`baseline-crash-events.txt` (Application Error and WER events),
`wer-*` (the Windows Error Reporting folders), `baseline-window-*.png`
(resize captures), and the desktop screenshots referenced from the defect
register.
