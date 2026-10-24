# ADR 0026: Project folders and sandboxed execution

Status: Implemented in `main` (PR #49, hardened in PR #51).

Originally proposed for review on the "Fixing Up A Little" branch while PR #49
was a draft. Platform-specific sandbox restrictions below remain in force.

## Context

Agents need to read and change files and to run checks. Tauri's default
capability is deliberately narrow, and a renderer must not gain filesystem or
shell access.

## Decision

- **Granting a folder.** A folder is granted to a project only through the
  native folder picker, read-only or read-write. The renderer never supplies a
  path. The host refuses (`ROOT_REFUSED`) the filesystem root, the home folder
  itself or any folder containing it, any folder whose path passes through a
  hidden (dot) folder, inside the home folder or not (`~/.ssh`, `~/.gnupg`,
  `~/.config` and its subfolders, `~/.local/share`, another user's `.ssh`, a
  disk's `.git`, …), any folder under a Windows `AppData` folder (which holds
  the per-user Startup folder) or, on macOS, under `~/Library`, Juniper's own data
  folder or anything
  containing or inside it, and system folders (`/etc`, `/usr`, `/boot`,
  `/var/lib` and similar; on Windows the system, program and program-data
  folders), comparing canonical paths (`workspace::refused_root`).
- **Protected paths.** Agents never write version-control, editor, CI and
  environment-loader configuration, shell start-up files (`.bashrc`,
  `.profile`, `.zshrc` and their relatives), `.ssh/` and `.gnupg/`,
  `.config/autostart/` and `.config/systemd/`, or `*.service`, `*.timer` and
  `*.socket` units (`PROTECTED_PATH`). Build manifests and scripts that run
  when the user builds or installs (`Makefile`, `*.mk`, `package.json`,
  `pyproject.toml`, `setup.py`, `Cargo.toml`, `build.rs`, `CMakeLists.txt`,
  `Rakefile`, `Gemfile`, Gradle and Maven files, `meson.build`, Bazel `BUILD`
  and `WORKSPACE`, `Dockerfile` and Compose files, `Taskfile`, `Jenkinsfile`,
  `configure`, `Vagrantfile`, `Procfile`, tool configuration that runs code
  (`vite.config.*`, `webpack.config.*`, `eslint.config.*` and similar),
  dependency manifests and lock files (`requirements*.txt`, `go.mod`,
  `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`,
  `poetry.lock`, `composer.json` and similar), and `*.sh`, `*.fish`, `*.csh`,
  `*.ps1`, `*.bat`, `*.nix`, `*.gradle`, `*.cmake` files), `*.desktop` entries, and every other dot-file or dot-folder at
  any depth can be written only through a per-call approval showing the diff:
  neither the "Within projects" preset nor a standing permission covers them,
  and no standing grant is offered. Residual risk: a project folder can still
  hold files that other programs run when the user opens them (for example a
  script the user later executes by hand); those writes are recorded in task
  activity and shown as a diff when an approval is asked, but the "Within
  projects" preset does not ask. Grants made before this check existed are not
  re-checked; remove and re-add them.
- **File access** goes through a `cap-std` directory handle on the granted
  folder, so a path cannot resolve outside it.
  - Every path component is also checked, and symbolic links and reparse
    points are refused rather than followed.
  - Reads are limited to 4 MB.
  - Writes are limited to 1 MB. Each write is shown as a unified diff, bound to
    the file's version (SHA-256, or `absent`), and applied atomically (a new
    file, synced, then renamed over the original). The version is checked again
    just before the rename.
- **Revoking a folder** removes its index entries at once.
- **Code** runs under bubblewrap on Linux. The sandbox has:
  - new user, mount, PID, network, IPC and UTS namespaces;
  - `/usr` read-only, a private `/work`, and an optional read-only `/project`;
  - a cleared environment;
  - CPU, address-space, file-size, open-file, process and core limits, plus a
    60-second wall limit;
  - process-group kill on timeout or cancel.

  `code.run` accepts only `python3` or `node` with argument lists. No shell is
  involved.

- **Other platforms.** Windows and Android have no verified sandbox, so
  sandboxed code is unavailable there and the interface says why.

## Consequences

Agents can do real project work without any change to `capabilities/*.json`.
The sandbox tests (`agent::sandbox::tests`) show four things on the build
machine and on CI, where the validation and release jobs install bubblewrap and
the tests fail (rather than skip) when the sandbox is unavailable; the
validation job uploads their output as `agent-sandbox-evidence`. A developer
machine that cannot sandbox may set `JUNIPER_SKIP_SANDBOX_TESTS=1`; CI refuses
that variable:

- the user's home folder is not visible inside the sandbox;
- no environment variables leak;
- `/usr` cannot be written and the network cannot be reached;
- a mounted project is read-only, and cancellation stops child processes.

## Limits

bubblewrap needs unprivileged user namespaces. Where they are disabled, the
availability probe reports it. Node.js must be installed system-wide.
