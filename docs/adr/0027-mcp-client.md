# ADR 0027: MCP client

Status: Proposed for review on the "Fixing Up A Little" branch (draft PR #49).
It implements the boundary ADR-0010 reserved.

## Context

The current MCP revision is 2026-07-28 (checked on 2026-10-15 at
modelcontextprotocol.io). That revision is stateless:

- every request carries `_meta` with the protocol version and client
  capabilities;
- `server/discover` is mandatory;
- the Streamable HTTP transport has no sessions and no GET stream.

Most published servers still implement the handshake-based revisions
(2025-11-25 and earlier). The reference `server-everything` answered with
2025-11-25 in our live check.

## Decision

Juniper implements its own small **dual-era** client in `agent/mcp.rs`.

**Era detection.**

- On stdio, Juniper probes with `server/discover`. A recognized modern answer
  keeps the modern era. Any other error, or a timeout, falls back to
  `initialize` with version 2025-11-25. Juniper accepts 2025-11-25, 2025-06-18,
  2025-03-26 and 2024-11-05.
- On HTTP, Juniper tries the modern request first. A response without a modern
  error body falls back to the legacy handshake, which uses `Mcp-Session-Id`.

**Supported features.**

- `tools/list` with pagination (up to 8 pages and 128 tools).
- `tools/call`.
- Cancellation: `notifications/cancelled` on stdio, closing the stream on HTTP.
- The HTTP headers `MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name`, with
  Base64 sentinel encoding.
- Validation of `x-mcp-header` annotations.
- SSE responses.
- Bounded schemas (64 KB) and results (32,000 characters).

**Not offered.** Juniper declares empty client capabilities and does not offer
sampling, roots, elicitation, resources, prompts, subscriptions, tasks, or
apps. An `input_required` result is reported as unsupported.

**Trust.**

- Tool definitions are checked: name, object schema, no remote `$ref`, and
  valid header annotations. Invalid tools are excluded and listed.
- Descriptions and results are untrusted data. Annotations never set risk:
  every plugin tool asks unless a standing grant exists, and the grant is bound
  to the server's tool-set digest.

**Local servers.**

- The program is chosen with the native file picker.
- The server runs with a cleared environment (a fixed `PATH`, and `HOME` set to
  a private folder) plus named secrets read from the keychain. Nothing else is
  inherited.
- The interface states plainly that the server is **not sandboxed**.

**Remote servers.**

- They must use https, except on loopback.
- A bearer token from the keychain is sent only to that endpoint, and redirects
  are not followed. OAuth sign-in is not implemented in this beta.

## Consequences

Normal chat never depends on MCP. Disabling or removing a server pauses the
tasks that were offered its tools, and removal revokes its grants. Tests:

- a deterministic Python fixture covering both eras, over stdio and HTTP;
- a live check against `@modelcontextprotocol/server-everything`.
