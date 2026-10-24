# Independent review of PR #54

Status at preparation: corrections implemented; exact-head CI and installed qualification pending. The subsequent PR review records the final decision.

Started 2026-10-23; qualification continued on 2026-10-24 (America/New_York).
Reviewer: Codex, with focused independent source reviews. Corrections were authored
during this review; previous engineering records remain historical self-checks.

## Identity and environment

- Repository: Cinqic-Research/Juniper-App.
- Base and origin/main: fc1f1d7937995ca68d14e8df1682f6869ae90d8b.
- Initial head: d98f18c2171b5af4b48fe90bd1f27a7b709cd8d4.
- PR #54: open draft, 22 commits, 35 files, no submitted reviews, comments or
  review threads at baseline. Working tree initially clean; existing engineering
  branch retained. No historical closed experiments were restored.
- CINQTOP detected directly: Windows 11 Home 10.0.26300; Node 24.22.1,
  Rust/Cargo 1.90.0 MSVC. Use Corepack to select repository-pinned pnpm 11.19.0;
  the generic PATH pnpm is 11.25.0.
- Detailed local logs: %TEMP%/juniper-independent-audit-20261023.
  Logs cover a changing working tree until a final reviewed commit is recorded;
  they must not be represented as one qualified immutable build.

## Findings and corrections under validation

| ID   | Severity | Root cause and correction                                                                                                                                                                                                                                                                                             | Regression evidence                                                                                                                                               |
| ---- | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| I-01 | P1       | Windows child job creation/assignment failures were ignored and assignment occurred after child execution. Establish inherited parent ownership before launch; suspend child until separate job assignment; fail closed; close per-child job after root exit. Runtime probes and Ollama import use the same launcher. | Forced parent death, descendant termination, concurrent-root isolation and restrictive enclosing-job refusal.                                                     |
| I-02 | P1       | Failed MCP handshake could retain a subprocess through reader ownership; writes and cancellation notification could wait without bounds; untrusted line allocation preceded size checks. Weak reader ownership, full request deadlines, bounded framing and disconnect on failed partial writes.                      | Windows modern/legacy fixture, cancellation, backpressure, writer contention, frame bounds and dropped-session tests.                                             |
| I-03 | P1       | Stale frontend snapshots acknowledged unseen approved Agent memory; another stale save could erase it. Keep outbox entries until renderer snapshot reflects their unique change IDs.                                                                                                                                  | Repeated stale saves and deletions, rollback on acknowledgement failure, preservation of user-edited memory.                                                      |
| I-04 | P2       | NTFS timestamps and USNs can coalesce while another reader stays open. Disable Windows cache reuse, retain SHA-256 authority and reject active writers at mutation checkpoints; move installed listing off the UI thread.                                                                                             | Windows caching must remain disabled; 17 held-reader edits, same-size/mtime-restored edits, hard links, replacement and live writers must all reject wrong bytes. |
| I-05 | P2       | Frontend abort during awaited event-listener registration was missed. Check abort before registration and immediately before native invocation.                                                                                                                                                                       | Eleven IPC cancellation cases; baseline reproduced before correction.                                                                                             |
| I-06 | P2       | Agent overview dropped refresh events in flight; late task reads overwrote newer task or selected child. Coalesce refresh, order responses and isolate task identity.                                                                                                                                                 | Three deferred-response regressions failed on baseline, passed after correction.                                                                                  |
| I-07 | P2       | Logical minimum window height exceeded work area under display scaling. Cap minimum only where it cannot fit; center actual measured outer frame.                                                                                                                                                                     | Geometry matrix for scales/taskbars/monitor offsets/small work areas; native/manual validation pending.                                                           |
| I-08 | P2       | Recursive malformed GGUF arrays had no depth bound and could exhaust stack. Bound metadata recursion.                                                                                                                                                                                                                 | Deep crafted headers in a small-stack thread.                                                                                                                     |
| I-09 | P1       | Windows generic local-runtime path did not check listener PID before sending its private key. Check exact loopback listener owner through Windows TCP table before authenticated requests; retain Linux-only Juniper 1 qualification.                                                                                 | Wrong-owner and rebound-port requests must make no authenticated connection.                                                                                      |
| I-10 | P2       | Windows verbatim-prefix stripping can change trailing-dot/space or reserved-name path identity. Preserve verbatim form for those components.                                                                                                                                                                          | Distinct ordinary/verbatim directories and special-name cases.                                                                                                    |
| I-11 | P2       | Hash/cache regressions were nested in a Unix-only test module, excluding Windows. Enable platform-neutral tests on Windows, retaining Unix-only symlink fixture guards.                                                                                                                                               | Explicit Windows small-stack and NTFS test execution required.                                                                                                    |
| I-12 | P3       | ADR 0022–0028 retained Proposed/pending wording after implementation. Correct current status while preserving historical reviews, research boundaries and qualification limitations.                                                                                                                                  | Source/history and release-record comparison.                                                                                                                     |

Persistent license updates are serialized for concurrent native callers;
first/subsequent/revised/corrupt/failed store tests preserve default refusal.
This is native API hardening rather than a reproduced user-interface race.

## Review coverage and limits

Reviewed application entry points, runtime/model lifecycle, frontend IPC/chat/Agent
state, SQLite snapshot and migration ownership, Agent policy/workspace/MCP,
provider networking and credentials, Android vault bridge, generic-text Juniper 1
refusal and token-ID platform restriction, capabilities and validation/release
workflow changes. Source inspection is distinct from executed platform tests.
Windows junction refusal was independently probed using actual junction metadata.

No model research artifacts, corpora, checkpoints, security settings, release
tags, publications or unrelated repositories were modified.

## Repository context inventory

Enumerated both authenticated GitHub connections through repository and
affiliation listings. All listed repositories default to main. Read relevant
READMEs for Juniper-1-Base, Juniper-1-Data, Juniper-1-Train, AAA and cinqic.com.
Private research access was available; no private research code was changed.

| Repository                           | Access / status           | Purpose and Juniper relationship                                                          |
| ------------------------------------ | ------------------------- | ----------------------------------------------------------------------------------------- |
| Cinqic-Research/Juniper-App          | public, active            | Execution target; independent product using compatible third-party models.                |
| Cinqic-Research/Juniper-1-Base       | public, preserved release | Separate base model and canonical tokenizer; no instruction/tool/reasoning qualification. |
| Cinqic-Research/Juniper-1-Data       | private, research closed  | Frozen corpus pipeline/evidence; provenance context only.                                 |
| Cinqic-Research/Juniper-1-Train      | private, research closed  | Frozen base and negative post-training outcomes; context only.                            |
| Cinqic-Research/AAA                  | public, active            | Separate research program; no shipping App dependency.                                    |
| Cinqic-Research/cinqic.com           | public, active            | Public product/release documentation; no direct runtime dependency.                       |
| Cinqic-Research/Cinqic-Calculator    | public, active            | Separate software product; no material App dependency found.                              |
| Cinqic-Research/Cinqic-Backup        | private, listed active    | Backup records; no shipping dependency identified, not recursively audited.               |
| Cinqic-Research/Juniper-LM-1         | private, archived         | Retired research, no supported App dependency.                                            |
| Cinqic-Research/Juniper-LM-1.1       | private, archived         | Retired research, no supported App dependency.                                            |
| Cinqic-Research/Juniper-Reference-4B | private, archived         | Retired research, no supported App dependency.                                            |
| Cinqic/Juniper-Auto                  | public, archived          | Retired experiment; excluded from implementation.                                         |
| Cinqic/Juniper-Encoder               | public, archived          | Retired experiment; excluded from implementation.                                         |
| Cinqic/juniper-math-1                | public, archived          | Retired experiment; excluded from implementation.                                         |
| CinqicX/Cinqic-Notes                 | public, archived          | Historical notes; no shipping dependency identified.                                      |
| MarkusGillyard/astroverse-site       | public, listed active     | Unrelated website; not recursively audited.                                               |
| MarkusGillyard/Slopperfall-releases  | public, listed active     | Unrelated game releases; not recursively audited.                                         |

## Verification ledger

Windows hash-cache performance is an accepted limitation, not a verified optimization. The original W-08 timing attribution is withdrawn. Timestamp and USN probes both failed to distinguish every rapid edit while a reader remained open; no unqualified change token is trusted.

- Frozen pnpm install with Corepack 11.19.0: passed.
- Production frontend build: passed on working candidate.
- Frontend suite: 22 files, 171 passed, zero failures.
- Initial native suite: 306 passed, zero failures, nine ignored; this precedes
  final listener/path/window and newly enabled Windows cache regressions.
- Initial exact-head GitHub validation run 40028157803: success. Includes Windows
  MSI/native tests, Linux native/audits/build and Ubuntu 22.04/24.04 launch smoke,
  Android compile/credential instrumentation/lifecycle. Manual physical Android
  inference was skipped. These results apply to the initial head only.
- Updated native working-tree run: 322 passed, zero failures, nine ignored. Includes Windows MCP handshake cancellation, wrong listener rejection, process trees, path identity, small stacks, migrations and geometry. The symlink-privilege fixture reports an internal skip and is not counted as exercised.
- Clippy all targets with warnings denied: passed after the range-check correction.
- C++ native contract tests: passed with MSVC /utf-8 /std:c++17 /W4 /WX; the initial default-codepage run failed its Unicode-literal assertion, so UTF-8 was explicitly selected without changing the test.
- JavaScript audit after Vitest 4.1.12 and fast-uri 3.1.9 patch updates: no known vulnerabilities, no ignored advisories. The old braces advisory was withdrawn and its exception removed.
- Nine ordinary native ignores: five private Juniper 1 golden fixtures, two optional live Ollama suites, a proxy child executed by its parent test, and a deliberately invoked real-database-copy test. Canonical public tokenizer SHA-256 was independently verified; private golden payloads are unavailable and no golden pass is claimed.
- Final pinned full validation, exact final CI, packaging comparison,
  isolated profile graphical/model qualification and final merge decision: pending.

## Platform API references

[Windows job ownership and nesting](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects).
[Atomic job-list process creation](https://devblogs.microsoft.com/oldnewthing/20230209-00/)
explains why post-execution assignment is insufficient. Rust 1.90's stable
Command API does not expose job-list startup attributes; this implementation
uses inherited parent ownership plus suspended per-child assignment.

Dependency references: [Vitest mock injection](https://github.com/advisories/GHSA-qxg6-89mg-rqv8), [Windows mock traversal](https://github.com/advisories/GHSA-xp26-3454-mjgp), [fast-uri scheme parsing](https://github.com/advisories/GHSA-w74x-v6f9-j3xg), [withdrawn braces advisory](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm).

## Exact-head CI follow-up

The first correction head `0b57f735cdbaf1e19e65e0b2fd362ea5ddc113c3` passed full local validation, Linux validation/audits/build and Android CI, but exposed two qualification defects in run 40061842295:

- The Windows restrictive-job fixture resolved Python after imposing its one-process limit. Resolve the interpreter first; the launch-refusal and no-user-code assertions remain intact. The corrected fixture passed locally with the override removed, matching CI discovery.
- GTK initially reported a zero outer frame during setup. Re-centering that frame put the 1280x820 window at (720,450), leaving a 720x450 clipped capture and inaccessible picker controls. Use the predicted fitted position on non-Windows; Windows remeasurement behavior is unchanged. The Linux picker gate remains unchanged and must pass on the corrected head.

Installed CINQTOP qualification began on the `0b57f735` Windows binary (MSI SHA-256 `f27cd98b306fb143b4352379fe67ca22d0508d3471897f14204cd37c2bb3434f`). These follow-up shipping changes affect only non-Windows placement; Windows fixture changes are tests only. Do not describe that installed binary as a different source build.
