# ADR-0010: MCP integration

Status: accepted. Implemented by [ADR 0027](0027-mcp-client.md) (2026-10-15, draft PR #49); the
"future work" note below is historical.

## Decision

Reserve a versioned MCP client boundary for an advanced feature. It must
support explicit server registration, tools/list, tools/call, per-server
visibility, local/remote labels, and permissions without becoming a startup
dependency.

## Consequences

Broken MCP servers cannot block normal chat. A full current-spec client is
future work and is not represented as shipped functionality in v0.1.

## Security/privacy

MCP output is untrusted. Remote servers are network calls and require visible
consent; local stdio servers require a dedicated scoped process capability.
