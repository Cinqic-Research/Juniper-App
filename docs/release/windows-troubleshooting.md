# Windows setup and troubleshooting

This page covers the Windows desktop build, `Juniper-<version>-windows-x86_64.msi`.

## What is tested

| Environment                                                              | Install and launch | Interface | Local model | Agent                               | Evidence                                                                                   |
| ------------------------------------------------------------------------ | ------------------ | --------- | ----------- | ----------------------------------- | ------------------------------------------------------------------------------------------ |
| GitHub `windows-latest` runner (CI)                                      | tested             | pixels    | not run     | not run                             | validation and release workflows (`scripts/verify-windows-msi.ps1`)                        |
| CINQTOP: HP 14-dq6xxx, Intel N150, 16 GB, Windows 11 Home 25H2, 1366x768 | tested             | tested    | tested      | folders and permissions, no sandbox | [CINQTOP qualification, 2026-10-23](../qualification/cinqtop-windows-2026-10-23/README.md) |

"Tested" on CINQTOP means a person-style run through the installed application:
installation, onboarding, Chat, Models, Agent and Settings, a model download,
local inference, restarts, and uninstall. The CI smoke only installs, checks the
executable's GUI subsystem, launches twice, checks that the window is visible,
responsive and not a uniform surface, and uninstalls.

## Installing

The MSI is not Authenticode-signed (`SIGNING-windows.txt` on the release page
says `AuthenticodeStatus=NotSigned`). Expect:

- A SmartScreen "Windows protected your PC" notice when the file was
  downloaded with a browser. "More info" then "Run anyway" continues; nothing
  needs to be disabled.
- A User Account Control prompt naming "Juniper" with an unknown publisher. The
  package installs for all users under `C:\Program Files\Juniper`, which needs
  that consent. A silent `msiexec /qn` from a non-elevated prompt fails with
  error 1603/1925 for the same reason; use `/passive` or run elevated.

Verify the download first:

```powershell
(Get-FileHash .\Juniper-<version>-windows-x86_64.msi).Hash
```

and compare it with `SHA256SUMS.txt` from the same release.

Uninstall from Settings, Apps, Installed apps. User data is kept.

## Where Juniper keeps its data

| Item                          | Location                                                       |
| ----------------------------- | -------------------------------------------------------------- |
| Application                   | `C:\Program Files\Juniper\juniper.exe`                         |
| Bundled local runtime         | `C:\Program Files\Juniper\runtime\llama-server.exe`            |
| Database, backups, agent data | `%APPDATA%\com.cinqic.juniper\juniper.db` and that folder      |
| Downloaded models             | `%APPDATA%\com.cinqic.juniper\models\`                         |
| Provider credentials          | Windows Credential Manager, entries named `com.cinqic.juniper` |

## Start Juniper with diagnostics

Juniper is a graphical application, so its `[juniper-startup]` lines are not
shown in a console. Capture them by redirecting its standard error:

```powershell
Start-Process "C:\Program Files\Juniper\juniper.exe" -RedirectStandardError "$env:TEMP\juniper-startup.log"
```

A healthy start writes:

```text
[juniper-startup] Juniper <version> starting on windows/x86_64
[juniper-startup] local runtime: \\?\C:\Program Files\Juniper\runtime\llama-server.exe
[juniper-startup] stage native-setup: ok
[juniper-startup] frontend ready
```

The diagnostics table in
[linux-troubleshooting.md](linux-troubleshooting.md#start-juniper-from-a-terminal)
applies unchanged. A crash after startup is recorded by Windows in Event
Viewer, Windows Logs, Application, as an "Application Error" event for
`juniper.exe`; the exception code there is the first thing to include in a
report.

## Known Windows problems

### 0.4.0-alpha.1 closes when the Models screen lists an installed model

In `0.4.0-alpha.1`, the application exits without a message as soon as a model
download completes or the Models screen is opened with at least one model
installed. The event log shows exception `0xC00000FD` (stack overflow) in
`juniper.exe`. The verification step hashed the model file with a 1 MiB
buffer on the stack, and the `managed_models` command runs on the main thread,
whose stack on Windows is exactly 1 MiB. Linux and Android main threads are
larger, so only Windows was affected. Fixed after `0.4.0-alpha.1`
(`docs/qualification/cinqtop-windows-2026-10-23/defect-register.md`, W-01).
There is no workaround in `0.4.0-alpha.1` other than not installing a model.

### 0.4.0-alpha.1 cannot record a model license acceptance

Accepting a license in the Models screen (Nemotron 3 Nano 4B in this
catalog) ends with "Access is denied. (os error 5)" and the download does
not start. The acceptance file was flushed through a read-only handle,
which Windows refuses. Fixed after `0.4.0-alpha.1` (W-16). There is no
workaround in `0.4.0-alpha.1`; models whose license needs no acceptance
are unaffected.

### The window opens under the taskbar on small displays

On a 1366x768 display the configured 1000x700 window plus its frame is taller
than the area above the taskbar, and Windows placed it at the top-left corner
instead of centring it. Fixed after `0.4.0-alpha.1` by fitting the window to
the monitor's work area at startup (W-02). Until then, maximise the window.

### Smart App Control refuses the installer or the application

Windows 11 Home and Pro installations that pass Smart App Control's
evaluation period turn it on by default. It then blocks executables that are
neither signed nor known to Microsoft's reputation service, with "An
Application Control policy has blocked this file" (error 4551, Code Integrity
event 3033). Juniper's MSI and `juniper.exe` are unsigned, so on such a
machine the installer or the application may be refused. Smart App Control
can be turned off in Windows Security, App & browser control, but not turned
back on without reinstalling Windows; Juniper does not ask anyone to do
that. Signing the Windows build is the remedy and is tracked as a release
item.

## Building on Windows

Beyond the [Tauri prerequisites](https://tauri.app/start/prerequisites/)
(Visual Studio Build Tools with the "Desktop development with C++" workload,
WebView2, Rust via `rustup` with the MSVC host), the repository needs:

- Node.js 22 or newer, pnpm 11.19 (`npm install -g pnpm@11.19.0`), CMake 3.31
  or newer, and Git. `winget` can install all of them.
- VBScript, which WiX uses to build the MSI. Windows 11 24H2 and later list it
  under Settings, Apps, Optional features; it is normally present.
- Long path support for Git when building the bundled runtime: the pinned
  llama.cpp checkout contains paths longer than 260 characters and
  `scripts/build-llama-runtime.sh` fails with `Filename too long` without it.
  GitHub's runners enable it machine-wide. Locally, either run
  `git config --global core.longpaths true` or scope it to the build:

  ```bash
  GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.longpaths GIT_CONFIG_VALUE_0=true \
    JUNIPER_LLAMA_BUILD_DIR="$LOCALAPPDATA/juniper-build/llama" \
    scripts/build-llama-runtime.sh
  ```

  Run it from Git Bash with CMake and Node on `PATH`. A short build directory
  keeps the generated object paths under the limit as well.

`cargo test` on Windows: one test creates a file symbolic link, which needs
`SeCreateSymbolicLinkPrivilege`. Without Developer Mode or an elevated
prompt that test reports that it was not run; the CI runner is elevated and
runs it. The suite itself runs on Windows since the test harness gained an
application manifest (see the CHANGELOG); before that it aborted at load.

On CINQTOP (four efficiency cores) the runtime build took 13 minutes with
`JUNIPER_LLAMA_BUILD_PARALLEL=2`; the first `cargo build --release` is
longer. Set `CARGO_BUILD_JOBS=2` if the machine must stay usable meanwhile.
