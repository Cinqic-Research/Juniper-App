# Contributing to Juniper

GitHub `main` is the canonical project copy. Keep changes reviewable and do
not rewrite shared history. Material architectural changes require an ADR.

Before opening a pull request, run `pnpm validate`, document any unavailable
platform checks, and keep claims aligned with executed evidence. On Linux the
agent sandbox tests need bubblewrap (`bwrap`) and fail without it; on a
developer machine that cannot sandbox, set `JUNIPER_SKIP_SANDBOX_TESTS=1` and
say so in the pull request. CI never sets it. Do not add
telemetry or proprietary required services. Treat model output, attachments,
MCP output, imported templates, and remote metadata as untrusted.

For provider work, add a deterministic fixture and test the normalized
contract. For tool work, add schema/permission tests and ensure real results
are host-authored. For UI work, check keyboard operation, reduced motion,
focus, narrow viewports, and local/remote labeling.

## Review roles

From 2026-10-24, by the owner's direction (the 2026-10-16 assignment named
Claude Fable 5.1 as reviewer):

- **Implementer: Claude Opus 5.5.** Builds, tests, and documents changes, and
  prepares the evidence. It may fix defects, but it never certifies its own
  work as independently reviewed.
- **Independent technical reviewer: GPT-6.1 Sol,** in a separate session
  with a fresh context. It reviews the exact head and the primary evidence
  itself, and may reject work, require changes, or withhold approval. If it
  authored, repaired, or directed the exact change, it says so and the change
  stays pending review.
- **Owner: Markus Gillyard.** Decides direction, funding, releases,
  irreversible operations, and policy exceptions. A technical approval is not
  owner authorization.

A named reviewer is an assignment, not evidence that a review took place;
only a recorded review of the exact head counts. Earlier reviews (Claude
Fable 5.1, GPT-6 Luna, GPT-6 and GPT-5.6 Sol, Codex) remain historical
records under their original attribution. This section does not change any
review gate.

Release and qualification gates in `docs/` are unchanged. Where they call for
real supported-platform evidence, a review does not replace it.
