# CINQTOP Windows qualification, 2026-10-23

An engineering run on Cinqic's secondary Windows workstation against the
published Juniper 0.4.0-Alpha MSI, followed by repairs on branch
`engineering/cinqtop-windows-qualification`. These records are the
implementer's own evidence (Claude Fable 5.1 in this session), not an
independent review. The owner has designated GPT-6.1 Sol as the independent
reviewer, working on CINQTOP itself, where the toolchain, the final MSI and
the test profile are already in place (see the handoff).

| File                                                   | What it is                                                                             |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------- |
| [hardware-qualification.md](hardware-qualification.md) | Verified CINQTOP hardware, software, resource baseline and budget                      |
| [cinqtop-inventory.json](cinqtop-inventory.json)       | The same inventory, machine-readable                                                   |
| [baseline-test-report.md](baseline-test-report.md)     | What the published 0.4.0-alpha.1 MSI did on CINQTOP, case by case                      |
| [defect-register.md](defect-register.md)               | W-01 to W-16: reproduction, root cause, fix and verification state                     |
| [candidate-test-report.md](candidate-test-report.md)   | What the branch build did on CINQTOP, and what could not be run there                  |
| [review-handoff.md](review-handoff.md)                 | Commits, changes, tests, evidence, limitations and verification steps for the reviewer |
| [independent-review.md](independent-review.md)         | The PR #54 review ledger as prepared, before exact-head CI and installed qualification |

## Summary

- The published MSI installs and renders correctly on real Windows 11
  hardware; the historical console-and-black-window report is not
  reproduced at launch. Onboarding, Settings, appearance, Agent projects and
  folder refusals work.
- The published MSI cannot use a local model on Windows: listing an installed
  model crashes the process (W-01, stack overflow on the 1 MiB main thread).
  Fixed on the branch and verified on the machine.
- With the fix, the bundled runtime serves SmolLM2 360M and Qwen2.5 1.5B
  without Ollama, chats keep context across turns, models switch, downloads
  pause and resume, and Agent tasks run, persist and can be steered. The
  Agent's workspace read and write tools were not exercised because no
  tool-qualified catalog model fits the machine's budget; sandboxed code is
  disclosed as unavailable, as designed.
- Three more Windows defects were reproduced (terminal window on runtime
  start, orphaned runtime after a kill, re-hashing on every use), fixed and
  verified on the packaged build once the owner turned Smart App Control
  off (W-13 records the interruption). Running the native suite on Windows
  for the first time (W-10 fixed) found and fixed a sixth: license
  acceptance failed on Windows (W-16), verified on the installed final MSI.

## Final disposition

The records above, including the review ledger, keep the state at the time
they were written. The final decision is the formal PR #54 review: approved at
`c793c5649ad165acd586b701d2399b42a1118ea0` and merged as
`6bbe0adf3dc50c3d697fd47e996b2f01d42ae75f` on 2026-10-24
([approval](https://github.com/Cinqic-Research/Juniper-App/pull/54#pullrequestreview-5638083041),
[post-merge verification](https://github.com/Cinqic-Research/Juniper-App/pull/54#pullrequestreview-5638154344)). The installed
CINQTOP qualification used the disclosed `0b57f735` Windows build; exact-head
CI ran on the later `c793c564` head, so these are separate build identities.
The supplemental Windows attachment symlink test ran after the merge and
closed the local privilege skip disclosed at approval. Physical Android
inference, optional and private fixtures, a genuine older owner-profile
upgrade and the other limits listed in the PR description remain unverified
and are not counted as passes. The repairs are on `main` but not in a
published release; the published `0.4.0-alpha.1` files are unchanged.

## Reproducing on another Windows machine

1. Build: `pnpm install --frozen-lockfile`, build the runtime
   (`docs/release/windows-troubleshooting.md`, "Building on Windows"), then
   `pnpm tauri build --bundles msi`. The MSI must come from `tauri build`;
   a plain `cargo build --release` produces a binary that loads the Vite dev
   URL.
2. Install the MSI (UAC consent), start from a fresh profile (move
   `%APPDATA%\com.cinqic.juniper` aside), download SmolLM2 360M, open
   Models: the application must stay up (W-01).
3. Start a chat with the model: no terminal window may appear (W-06); kill
   `juniper.exe` from Task Manager: `llama-server.exe` must end within a
   second (W-07); open Models repeatedly with a multi-GB model installed:
   the second open must be immediate (W-08).
4. On a 1366x768 display, the window must open centred above the taskbar
   (W-02).
5. Grant a folder to a project: the Projects list must show `C:\...` (W-03).

## Evidence outside the repository

Raw logs, PowerShell probe reports, window captures, Windows Error Reporting
folders and desktop screenshots stay in the session's evidence folder on
CINQTOP (`%LOCALAPPDATA%\Temp\claude\...\scratchpad\evidence\`). They
contain user paths and are listed by name in the two test reports; copy them
on request. Nothing in this folder contains credentials, serial numbers or
chat contents.
