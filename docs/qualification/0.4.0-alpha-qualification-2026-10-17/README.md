# Juniper 0.4.0-Alpha qualification records, 2026-10-17 (non-implementer run)

These files are the raw records from the run that closed release gates **G2**
(exact-source resumed transfer above 2 GiB and a sharded bundle) and **G3**
(native-application workflows) for Juniper 0.4.0-Alpha (`0.4.0-alpha.1`). They
were copied byte for byte from the run's evidence folder; their SHA-256 values
match `evidence-SHA256SUMS.txt`, the manifest the run wrote. The gate table and
the release decision are in
[docs/release/juniper-0.4.0-alpha-readiness-review.md](../../release/juniper-0.4.0-alpha-readiness-review.md).

## Provenance

| Field             | Value                                                                                                                                                                                 |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Who ran it        | Claude Fable 5.1 (`claude-fable-5-1`), a qualification subagent with a fresh context working for the Fable 5.1 lead session. It did not write, repair or direct any Juniper-App code. |
| Independence      | Non-implementer, separate session. The implementer is Claude Opus 5.5; both are Anthropic models, so this is same-vendor evidence, not vendor-independent assurance.                  |
| Source under test | `Cinqic-Research/Juniper-App` `main` at `ea9cbe566ec7c59036ab9601ab522a5ccb639c34` (merge of PR #49), fresh clone, clean tree before and after                                        |
| Version           | `0.4.0-alpha.1` (startup log: `Juniper 0.4.0-alpha.1 starting on linux/x86_64`)                                                                                                       |
| When              | Saturday, October 17, 2026 (EDT). G2: 2:54 AM to 3:07 AM. Debug build: 6:22 AM to 6:23 AM. G3 app session: 6:26 AM to 6:49 AM.                                                        |
| Report            | `Juniper-App-0.4.0-alpha-qualification.md` in the owner's review-report folder (`~/Documents/Fable-Review-Reports-2026-10-16/`); not in this repository                               |
| Original folder   | `/home/cinqic/j1work/app-qual-evidence/` on FLOWBOX                                                                                                                                   |

## Environment

FLOWBOX: Linux 7.0.0-38-generic x86_64, 16 cores, 14 GB RAM, Cinnamon X11
desktop on `:0`; rustc 1.90.0, Node v22.23.3, pnpm 11.19.0, bubblewrap 0.9.0;
NVMe system disk with about 12 GB free before the run (the transfer check needs
at least 6.2 GB). Bundled runtime `/usr/lib/Juniper/runtime/llama-server`
(`0.3.0-dev`, build 10788, commit `e107984bc`). Builds used
`CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`. The G3 app ran with
`XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME`
pointing into the evidence folder, so it started from an empty profile; the
owner's own Juniper data was not opened. Input was synthetic X11 (XTEST)
through the application window and the native GTK dialogs only; no IPC
command, database write or page script was used to act.

## Files

| File                          | What it is                                                                                                                                |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `transfer-check.jsonl`        | G2 raw harness record (two lines, one per artifact), written by `scripts/live-transfer-check.sh`                                          |
| `transfer-check-record.json`  | G2 annotated record: environment, commands, identity checks against `git show`, per-artifact results                                      |
| `ui-workflow-records.json`    | G3 record: build, launch, isolation, model, per-workflow steps, task ids, labels, outcomes, observations                                  |
| `db-export.json`              | Read-only export of the isolated G3 database after the run                                                                                |
| `scripted-provider.jsonl`     | Every request to the scripted OpenAI-compatible fixture provider used for G3 workflow (a), with its fixed reply                           |
| `project-root-before.sha256`  | Digests of the G3 project folder's files before the run                                                                                   |
| `screenshot-inventory.sha256` | Names and SHA-256 of the 47 window captures (`00-launch.png` to `46-task-list-final.png`, 6.3 MB); the images stay in the original folder |
| `evidence-SHA256SUMS.txt`     | The run's own manifest of every evidence file. One line names `SHA256SUMS.txt.tmp`, a temporary file that no longer exists.               |

The two scripts the run used (`tools/xdrive.py`, `tools/scripted_provider.py`),
`app.log` and `app-build.log` are listed in the manifest and kept in the
original folder.

## Key identities

- G2: `sourceCommit` `ea9cbe566ec7c59036ab9601ab522a5ccb639c34`; `harnessSha256`
  `f9795375872217afc265eceaf96903c4637f4ae656a1e0597279377283831ccd`
  (`src-tauri/src/live_catalog.rs`); `transferCodeSha256`
  `843bcc94b217976a5b37a1e62bdeaef6287615c555c719bae86dfe3f391de7a0`
  (`src-tauri/src/managed_models.rs`); test binary
  `7ae05f25f5d64af7ef049e5f84241c8b8f0251da1a0943a51e9fd414468cc0a4`.
- G3: debug binary `ddb225a08916ee733823f250f34939481a4cf1eecf064dca4d325be1781a172c`;
  tasks `task-396e30ee-cf08-4fae-9126-e23f8f9b325f` (a, Needs review),
  `task-09f30e4e-df4b-4d86-a1e5-855a65c36280` (control, Completed),
  `task-bf181ec9-f44c-4a03-b2c8-9be5bcced70c` (d1, `package.json` denied),
  `task-7138e643-fc66-44ba-b22c-abcbf5bb0a6d` (d2, `run.sh` allowed once),
  `task-f3978e76-9567-491c-89a3-1bb586b3aa9b` (d3, `.bashrc` refused); project
  `project-01ae6d92-2e81-414b-97a5-87c4bb8d4565`.

## Catalog note

G3 workflow (b) downloaded the Llama 3.2 1B and Gemma 3 1B entries to exercise
the license dialogs. Those entries were held from the 0.4.0-Alpha catalog
afterwards (release gate G7). The license-dialog code they exercised is
unchanged: the saved-acceptance path still applies to Nemotron 3 Nano 4B, the
remaining entry whose license must be accepted, and the mutable-terms path
(never saved, asked on every transfer) now has no catalog entry and is covered
by unit tests only.

Not copied: the G2 script's console output (`transfer/run.log` in the original folder on FLOWBOX, `/home/cinqic/j1work/app-qual-evidence/`); the machine-readable `transfer-check.jsonl` and `transfer-check-record.json` carry the same results.
