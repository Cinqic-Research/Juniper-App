# Wrapper evaluation

Development cases for Juniper's wrapper around gpt-oss-20b. They are aimed at
the failure modes the Juniper LM 1.1 qualification recorded — fabricated
papers, APIs, flags, people, URLs, and repository files; instructions followed
from documents — plus capability honesty, identity, and a few ordinary tasks
that would show a cost from the added instructions.

`cases.v1.jsonl` is a development set: it may be extended, and wrapper changes
may be tuned against it. Every entity in it was written for this file and
differs from the frozen qualification suite, which is used only as a held-out
comparison: read, never edited, never tuned against.

Each case lists `messages`, optional `tools`, `attachments`, `memories`,
`conversations` (host chat-search data), `permissions` (decisions for
permission prompts; anything unlisted is denied), and `checks`
(`regex_any`, `regex_all`, `regex_none`, `tool_called`, `tool_not_called`).
Patterns run case-insensitively on text with typographic apostrophes, hyphens,
and narrow spaces normalized.

## Running

On FLOWBOX, with the qualified GGUF and CUDA server from Juniper LM 1.1:

```bash
data=/path/on/the/storage/volume/appdata
mkdir -p "$data/models"
# A hard link, not a copy: same filesystem, no second 12 GB file.
ln /path/to/gpt-oss-20b-6cee5e81ee83-MXFP4_MOE.gguf \
   "$data/models/gpt-oss-20b-6cee5e81ee83-mxfp4-moe.gguf"
export JUNIPER_LIVE_GPT_OSS=1 XDG_DATA_HOME="$data" \
  JUNIPER_LLAMA_SERVER_CUDA=/path/to/llama.cpp-b11270/build-cuda/bin/llama-server
cargo test --manifest-path src-tauri/Cargo.toml --release --lib \
  -- --ignored --nocapture live_resident_runtime_lifecycle
JUNIPER_EVAL_OUT=results.jsonl JUNIPER_EVAL_SEEDS=3 \
  JUNIPER_EVAL_HELDOUT=/path/to/Juniper-LM-1.1/evals/juniper-gptoss-qual.v1/cases.jsonl \
  cargo test --manifest-path src-tauri/Cargo.toml --release --lib \
  -- --ignored --nocapture live_wrapper_evaluation
```

`JUNIPER_EVAL_ONLY=<id prefix>` limits the run; comma-separated prefixes select
several groups in one live server session. Results append to
`JUNIPER_EVAL_OUT`, one line per case, seed, and condition (`wrapper` or
`raw`), with each check's outcome and the answer text.

## Conditions

- **wrapper**: the full host path — constitution, runtime section, framed
  memories and attachments, the host tool loop with permission decisions, and
  answer acceptance.
- **raw**: the same server, weights, sampling, and reasoning effort; no Juniper
  layers; attachments and memories pasted into the user message; tools offered
  with no host behind them (a requested call is recorded and not run).

The checks are regular expressions and fail in both directions; read the
answers before drawing a conclusion. A pass rate on 24 cases at three seeds is
a direction, not a measurement of truthfulness.
