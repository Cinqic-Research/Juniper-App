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
