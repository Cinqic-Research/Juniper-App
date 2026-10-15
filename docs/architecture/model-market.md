# Models and the managed model lifecycle

The screen was called Models Market through `0.3.0-rc.32`; it is now simply
**Models** (ADR 0021).

`config/models/catalog.json` is the single checked-in source for the model
catalog. Each entry includes the model family, parameter count, use cases, chat
template, context length, supported CPU architectures, license and attribution,
and concrete runtime artifacts. Each artifact carries its runtime, format,
source revision, HTTPS file manifest, exact byte size, SHA-256 digest, maturity,
and qualification. The 2026-10-15 research snapshot adds 28 model records and
33 artifacts; live smoke evidence is summarized separately and does not change
their `package-only` qualification.

The frontend and Rust native layer both validate the catalog shape before using
it. The frontend combines architecture, logical cores, available/total memory,
memory pressure, and free storage to produce an explainable recommendation.
Unknown measurements reduce confidence; they never become an optimistic
compatibility claim. Downloads require the model size plus a 1 GiB safety
headroom when free storage is known. Memory and storage fit are calculated for
the selected artifact, so a larger quantization does not inherit the estimate
for the smallest file. Upstream context is shown separately from the local
llama.cpp cap of 16,384 tokens. Model entries are tagged for agent tools only
when that artifact has an explicit tool-use qualification.

Terms that require acceptance are tied to an immutable model-repository license
revision where one is available. Juniper remembers that acceptance against the
license name, URL, and revision. Gemma's terms are hosted at a mutable external
URL, and the historical response body is not available, so its catalog hash
cannot establish which exact text a user saw. Juniper links to the current
terms, explains this limit, and asks again before each Gemma download without
saving acceptance for future downloads.

## Download state machine

```text
absent -> downloading -> ready
             |              |
             v              v
          partial        corrupt
             |
             +------ resume
```

Weights are written to an app-data `models/` directory as a `.gguf.part` file.
An existing partial file is resumed only when the server returns `206 Partial
Content`; otherwise it is safely restarted. The stream is bounded by the
catalogue's expected byte count. The completed file is hashed, compared with
the catalog digest, and atomically renamed into its final name only after the
check passes. A mismatch is removed and surfaced as a structured error.

The final and partial paths are derived from trusted artifact IDs and reject
unsafe names. Symlinks are not accepted for verification or resume. Remove
cleans both final and partial files for the selected trusted catalog entry.

The Models screen exposes Recommended, All, and Installed views and a one-line
device summary. Each card leads with the name, purpose, size, fit, and one
primary action (download, pause, resume, use as default, or remove). Warnings
that decide whether a model can run — insufficient storage, no compatible
engine on this device, a non-stable engine maturity, or a partial or corrupt
file — stay on the card. The fit reason, parameters, format and quantization,
engines, qualification, upstream and local context limits, license terms,
source, revision, file, and SHA-256 sit behind a Details disclosure.
