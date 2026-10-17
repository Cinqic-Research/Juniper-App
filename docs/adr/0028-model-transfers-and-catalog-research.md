# ADR 0028: Resumable model transfers and dated catalog research

Status: Proposed for review on the "Fixing Up A Little" branch (draft PR #49).
It extends ADR-0018.

## Context

The downloader had five limits:

- a 2 GiB ceiling;
- a 300-second total timeout;
- one file per artifact, always the first artifact;
- resume trusted any 206 response;
- the catalog listed five entries, four of them small.

## Decision

**Transfers** (`managed_models.rs`).

- Downloads name an artifact. The default is the first one listed.
- There is no size ceiling. A transfer fails only after 60 s with no data.
- A journal binds partial files to the artifact, revision and file manifest. If
  the journal does not match, the partial files are discarded.
- On resume, only a 206 with the exact `Content-Range` is accepted. A 200
  response, a different range, or a 416 restarts the file. `Content-Length`
  must match what remains, and byte counts are checked arithmetic.
- Each file's SHA-256 is verified before it is kept.
- A multi-file bundle is staged in its own folder and published with one
  rename, with a ready marker inside. An interrupted bundle is never in place.
- Only one operation per artifact runs at a time, and at most two downloads.
- Disk-full errors become a resumable error, and free space is checked with
  1 GB of headroom.
- Single-file artifacts keep their pre-v2 file names, so existing installs
  still resolve.

**Context size.** Generic local models are served with
`--ctx-size min(context length, 16,384)`. The served context is recorded in the
runtime identity, and every prompt is measured against it before generation.

**Catalog research.** The catalog is regenerated from a dated research
snapshot (`docs/research/model-catalog-<date>/`) by
`scripts/catalog-from-research.mjs`.

- Popularity is Hugging Face's 30-day download count for the upstream
  repository. Quantization mirrors are not added to the count.
- The snapshot's eligibility rules were declared before ranking.
- Every file's revision, size and SHA-256 come from the Hugging Face API, never
  from hand entry.
- Models whose terms require it carry `licenseAcceptance: required`; the
  interface asks for acceptance, and the native host refuses a download or
  import without a matching acceptance record (`model_licenses.rs`).
- There is no automatic catalog update at startup.

## Consequences

The dated snapshot includes 28 model records and 33 artifacts, and the
snapshot's ten most-downloaded eligible releases are present. A per-artifact
summary of the reported live download, verification, load, chat, and tool-call
smoke runs on the bundled llama.cpp build is in
`docs/qualification/catalog-live-2026-10-15.md`. The raw JSONL is external to
this checkout, so the checked-in report is not a standalone reproduction
package. The live-resume row also lacks the exact outgoing `Range` and received
`Content-Range` headers; exact-range handling is covered by fixture tests, not
that live record. Artifacts keep `qualification: package-only` unless
capabilities were measured.
