# Permission model

Tauri capabilities grant `core:default` plus the native dialog picker. No
blanket filesystem, shell, or remote-source permissions are enabled. Native
operations are narrow Rust commands:

- attachment registration validates file type, regular-file status, and a
  1 MiB size cap before assigning an opaque grant ID;
- attachment reads accept only a previously granted ID;
- credentials use an OS secure store on desktop and are never returned to the
  webview;
- provider requests are explicit and carry a visible local/remote profile;
- cancellation is owned by the user and is checked during streaming.

User-data and filesystem tools pause at a native permission dialog. The user
can allow the current call once, grant access for the chat or assistant, or
deny it. Tools that change saved data (`memory.save`, `memory.delete`) are
approved one call at a time: the dialog shows the exact text the host will
save or delete, offers only "Allow once" or "Deny", and a stored grant never
applies to them. Calls that cannot succeed (an unknown memory, an attachment
not granted to this request, invalid arguments) are refused before the user is
asked. Private chats are offered no memory or chat-search tools at all. Chat-scoped grants are removed with private chats and are never
persisted for private conversations; assistant-scoped grants can be revoked
from Settings › Tools & permissions.

An assistant template, attachment, model output, MCP result, or provider
metadata cannot change this policy. A future shell/sidecar runtime must add a
dedicated capability and a separate review; it must not broaden `default.json`.

## Agent capabilities (Beta)

Agent tasks use one host decision point (`src-tauri/src/agent/policy.rs`):

| Risk          | Examples                                                                                                                  | Default                                                                                                                                                                                                                                                             |
| ------------- | ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Safe          | calculator, date and time, plan, ask the user, create a document                                                          | Runs                                                                                                                                                                                                                                                                |
| Project read  | list, read, search granted folders; project search; read own documents                                                    | Runs (the folder grant is the consent)                                                                                                                                                                                                                              |
| Project write | write a file in a read-write folder (never version-control, editor, CI, shell start-up, SSH, autostart, or service files) | Asks with the full diff, unless allowed for that agent and project or the "Within projects" preset is on; build manifests (`Makefile`, `package.json`, `pyproject.toml`, `Cargo.toml`, …), `*.desktop` files and other dot-files always ask, with no standing grant |
| Network       | web search, read a web page                                                                                               | Asks, unless that tool was allowed; no preset allows it, so project data cannot leave unseen                                                                                                                                                                        |
| Process       | sandboxed Python, project checks                                                                                          | Asks, unless that tool was allowed                                                                                                                                                                                                                                  |
| Plugin        | MCP server tools                                                                                                          | Asks, unless allowed; grants bind to the server's tool-set digest                                                                                                                                                                                                   |
| Memory write  | save a memory                                                                                                             | Asks every time; standing grants never apply                                                                                                                                                                                                                        |
| Automation    | create a schedule                                                                                                         | Asks every time, showing the next run times                                                                                                                                                                                                                         |

Presets (`Ask first`, `Within projects`, `Custom`) are described in the
interface before they are chosen. Standing permissions are listed under Agent ›
Permissions, where each can be revoked; revocation pauses that agent's running
tasks. Folder grants are added and removed only under Agent › Projects through
the native folder picker; the host refuses the filesystem root, the home folder
itself, Juniper's own data folder, and system folders.

A standing permission names one tool: allowing web searches does not allow page
fetches. An approval shows everything it covers; a call whose arguments or diff
are too long to show in full is refused rather than shortened.
