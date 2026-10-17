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
