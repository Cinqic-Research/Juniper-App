# ADR 0026: Project folders and sandboxed execution

Status: Proposed for review on the "Fixing Up A Little" branch (draft PR #49).

## Context

Agents need to read and change files and to run checks. Tauri's default
capability is deliberately narrow, and a renderer must not gain filesystem or
shell access.

## Decision

- **Granting a folder.** A folder is granted to a project only through the
  native folder picker, read-only or read-write. The renderer never supplies a
  path.
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
machine:

- the user's home folder is not visible inside the sandbox;
- no environment variables leak;
- `/usr` cannot be written and the network cannot be reached;
- a mounted project is read-only, and cancellation stops child processes.

## Limits

bubblewrap needs unprivileged user namespaces. Where they are disabled, the
availability probe reports it. Node.js must be installed system-wide.
