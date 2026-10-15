# Catalog live verification (2026-10-15)

Evidence for every downloadable catalog artifact, produced by the ignored test
`src-tauri/src/live_catalog.rs` on FLOWBOX (Linux x86_64, 16 cores, 14 GiB RAM),
CPU-only, with the Juniper-bundled llama.cpp server
(`version: 0.3.0-dev (build 10788, commit e107984bc) | built with GNU 11.4.0 for Linux x86_64`).

For each artifact the test:

- downloaded it from the pinned Hugging Face URL through Juniper's own transfer code;
- verified every file's size and SHA-256 before publishing it;
- started llama-server with Juniper's generic arguments (`--ctx-size` = catalog context capped at 16,384);
- asked one factual question (pass = the answer names Paris);
- offered one `calculator.evaluate` tool (pass = a well-formed call whose expression contains both operands);
- deleted the files.

These are smoke checks of one request each, not qualification of answer quality, tool
selection in general, long contexts, or other platforms. Windows and Android were not
exercised. Catalog `qualification` stays `package-only`.

| Artifact                               |    Size | Download (s) | SHA-256  | Load | Chat | Tool call | Notes                                                                            |
| -------------------------------------- | ------: | -----------: | -------- | ---- | ---- | --------- | -------------------------------------------------------------------------------- |
| `qwen3-1.7b-q8-0`                      | 1.83 GB |          184 | verified | pass | pass | pass      |                                                                                  |
| `qwen3-0.6b-q8-0`                      | 0.64 GB |           63 | verified | pass | pass | pass      |                                                                                  |
| `qwen2.5-0.5b-instruct-q4-k-m`         | 0.49 GB |           48 | verified | pass | pass | pass      |                                                                                  |
| `smollm2-360m-instruct-q4-k-m`         | 0.27 GB |           31 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `smollm2-135m-instruct-q4-k-m`         | 0.11 GB |           14 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `qwen3-4b-q4-k-m`                      | 2.50 GB |          252 | verified | pass | pass | pass      | paused after 234,591,188 bytes and resumed with an exact range                   |
| `qwen3-4b-q8-0`                        | 4.28 GB |          419 | verified | pass | pass | pass      |                                                                                  |
| `qwen3-4b-instruct-2507-q4-k-m`        | 2.50 GB |          325 | verified | pass | pass | pass      |                                                                                  |
| `qwen3-8b-q4-k-m`                      | 5.03 GB |          564 | verified | pass | pass | pass      |                                                                                  |
| `qwen3-14b-q4-k-m`                     | 9.00 GB |          885 | verified | pass | pass | pass      |                                                                                  |
| `qwen2.5-1.5b-instruct-q4-k-m`         | 1.12 GB |          117 | verified | pass | pass | pass      |                                                                                  |
| `qwen2.5-1.5b-instruct-q8-0`           | 1.89 GB |          213 | verified | pass | pass | pass      |                                                                                  |
| `qwen2.5-7b-instruct-q4-k-m`           | 4.68 GB |          453 | verified | pass | pass | pass      |                                                                                  |
| `qwen2.5-coder-7b-instruct-q4-k-m`     | 4.68 GB |          446 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `llama-3.2-1b-instruct-q8-0`           | 1.32 GB |          127 | verified | pass | pass | fail      | tool: HTTP 500 Internal Server Error (output did not match the tool-call format) |
| `llama-3.2-1b-instruct-q4-k-m`         | 0.81 GB |          112 | verified | pass | pass | pass      |                                                                                  |
| `llama-3.2-3b-instruct-q4-k-m`         | 2.02 GB |          192 | verified | pass | pass | pass      |                                                                                  |
| `llama-3.1-8b-instruct-q4-k-m`         | 4.92 GB |          487 | verified | pass | pass | pass      |                                                                                  |
| `gemma-3-1b-it-q8-0`                   | 1.07 GB |          105 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `gemma-3-1b-it-q4-k-m`                 | 0.81 GB |           84 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `gemma-3-4b-it-q4-k-m`                 | 2.49 GB |          238 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `deepseek-r1-distill-qwen-1.5b-q4-k-m` | 1.12 GB |          107 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `deepseek-r1-distill-qwen-7b-q4-k-m`   | 4.68 GB |          480 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `mistral-7b-instruct-v0.3-q4-k-m`      | 4.37 GB |          413 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `ministral-3-3b-instruct-2512-q4-k-m`  | 2.15 GB |          203 | verified | pass | pass | pass      |                                                                                  |
| `phi-4-mini-instruct-q4-k-m`           | 2.49 GB |          240 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `phi-4-mini-instruct-q8-0`             | 4.08 GB |          387 | verified | pass | pass | fail      | tool: no call (finish=stop)                                                      |
| `smollm3-3b-q4-k-m`                    | 1.92 GB |          184 | verified | pass | pass | fail      | tool: no call (finish=length)                                                    |
| `granite-4.0-micro-q4-k-m`             | 2.10 GB |          199 | verified | pass | pass | pass      |                                                                                  |
| `granite-4.0-h-tiny-q4-k-m`            | 4.23 GB |          400 | verified | pass | pass | pass      |                                                                                  |
| `minicpm5-2b-q4-k-m`                   | 1.56 GB |          152 | verified | pass | pass | pass      |                                                                                  |
| `nemotron-3-nano-4b-q4-k-m`            | 2.84 GB |          269 | verified | pass | pass | pass      |                                                                                  |

Totals: 32 artifacts; downloaded and verified 32, loaded 32, chat 32, tool call 19.

Not run here: `gpt-oss-20b-6cee5e81ee83-mxfp4-moe` (12.1 GB). It predates this phase and has
its own qualification (`docs/qualification/gpt-oss-wrapper-evaluation-2026-10-02.md`); this
branch changes only its download path, which the other 32 artifacts exercise.

Raw records (one JSON object per artifact, including revisions, file digests, timings,
and the model's short answers) are kept outside the repository with the review package;
the table above is the complete summary.
