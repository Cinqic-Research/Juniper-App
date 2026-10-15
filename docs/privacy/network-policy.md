# Privacy and network policy

Juniper v0.3 contains no telemetry, analytics, advertising, automatic
conversation upload, silent cloud fallback, or Cinqic network requirement.

The Juniper local profile targets a loopback `llama-server` process that Juniper
starts and stops for a chat request. A remote endpoint receives the prompts the
user sends through it. Model Market downloads, MCP calls, and web tools must be
explicit actions; they are not part of startup or automatic fallback. The local
model catalog is HTTPS-only and every download is checked against a checked-in
SHA-256 pin before installation. The host derives ON-DEVICE, LOCAL-NETWORK,
REMOTE, or UNKNOWN from the provider endpoint when a chat or Agent task starts,
so stale model metadata cannot label a remote endpoint as local. Non-local
hostnames are disclosed as REMOTE; the request path independently resolves,
validates, and pins its destination before sending prompts. Literal reserved
addresses and malformed routes remain UNKNOWN or are refused. Provider
hostnames are resolved and their answers vetted before requests; direct requests
pin the vetted addresses. Public hostnames must resolve to public addresses;
private DNS endpoints must use a literal private address or a `.local` name.
Mixed public and private answers, known cloud metadata destinations, and other
reserved addresses are refused. All provider requests bypass inherited system
proxies, so provider names use the vetted, pinned route. Direct connectivity is
required. Provider requests do not follow redirects; users must configure the
final provider URL directly.

Ollama remains an optional external provider for users who already run it. The
Juniper local provider never probes Ollama and never silently falls back to it.

Provider credentials are referenced by opaque IDs. Desktop uses the OS
credential store; Android uses Android Keystore. Exports intentionally omit
credential values. Editing a saved API key assigns a new reference, and a
successful route snapshot save retires the replaced reference after signaling
affected Agent tasks to pause.

Device Link and the Juniper Network provider are disabled in this candidate.
The repository retains bounded framing, replay, least-privilege scope, and
private-chat policy as a non-networked preview, but there is no listener,
discovery, usable pairing flow, peer connection, or remote-control route. A
future implementation must exchange the exact TLS certificate or SPKI pin used
by transport, store the private key in a platform secure facility, and validate
resolved destinations as private or link-local before any network I/O.

## Agent (Beta) network use (draft PR #49)

Agent tasks make no network request unless a tool that uses the network is
enabled for that agent and then allowed:

- **Web search** sends the query to one provider the user chose: Wikipedia
  (the default, no account), Brave Search (the user's key), or the user's own
  SearXNG instance. Nothing is sent to Cinqic.
- **Web pages** are fetched directly from public addresses only (see the threat
  model). Fetched text is stored as a task source.
- **Remote MCP servers** receive the tool arguments the user approves. Local
  MCP servers are programs on this computer; they can use the network
  themselves, which the interface states.
- **Remote models** used by an agent receive the task's prompts, tool results,
  and file passages, as in chat; the model's execution location is shown in
  the task.

A task run with a local model and only local tools stays on the device.
Choosing a local model does not make web or plugin tools offline. Model
downloads go only to the pinned Hugging Face URLs in the catalog; there is no
automatic catalog refresh.
