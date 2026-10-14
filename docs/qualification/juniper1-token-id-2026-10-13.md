# Juniper 1 token-ID backend: qualification record (2026-10-13)

Machine-readable record:
[`juniper1-token-id-2026-10-13/live-evidence.json`](juniper1-token-id-2026-10-13/live-evidence.json).
Design: [ADR 0024](../adr/0024-juniper1-token-id-backend.md). This record has
not been independently reviewed.

## What ran

| item                 | identity                                                                                                                                   |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Juniper App          | branch `a02/native-token-id-adapter`, code commit `3a0ea94` on base `e3e8656`                                                              |
| Python reference     | Juniper-1-Data `f0399ed` (`jtok.py`, `protocol.py`, `runtime_client.py` identical at `b7af963`)                                            |
| tokenizer            | j1tok `176a89e2…adf1` (24,576 IDs), Python `tokenizers` 0.23.2, Rust crate `tokenizers` 0.23.2 (`onig`)                                    |
| runtime              | llama.cpp b11270 `748d4225b` (CUDA build run CPU-only: `CUDA_VISIBLE_DEVICES` empty, `-ngl 0`), FLOWBOX                                    |
| base GGUF (F16)      | `juniper1-base-v1-f16` `41e21fed…b8980`, checkpoint `ckpt-000052034-336e5967`                                                              |
| SFT pilot GGUF (F16) | `juniper1-sft-pilot-v1-f16` `1537fde2…95175`, checkpoint `ckpt-000001226-55d13f9a` (qualified pilot candidate, **not a production model**) |

## Results

**Deterministic tests** (`cargo test`, CI): 180 pass, 12 ignored (hardware,
golden, and the proxy child). The Juniper 1 tests use fake loopback servers and
a stand-in tokenizer; they are mocks of the runtime, not evidence about it.

**Cross-language golden tests** (canonical tokenizer, fixtures written by the
Python reference): 5/5 pass — all 64 control spellings alone and embedded,
48 Unicode/whitespace/indentation/NUL/combining/byte-fallback texts (plus the
`jtok` probes), 12 serializations, 8 budgeting cases including whole-turn
trimming and tool-result truncation, 31 parser outcomes, and **200/200**
regression documents with identical IDs (the A-02 selection; hashes committed,
texts kept off this public repository).

**Live, through the App request path** (`local_runtime::stream_chat` with a
mock Tauri app, real llama-server, both real GGUFs): **34/34 checks pass**.

| area                    | checks                                                                                                                                                                                                                                                                                                               |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --- | ----------------------------------------------------------------------------------------------------- |
| text paths              | the guard refuses both real exports                                                                                                                                                                                                                                                                                  |
| start-up refusals       | wrong runtime build (e107984bc) `RUNTIME_IDENTITY_MISMATCH`; missing tokenizer; tampered tokenizer; model hash mismatch; a runtime that exits (a fixture script reporting b11270) `LOCAL_RUNTIME_FAILED`; no process left after any of them                                                                          |
| runtime behavior        | self-test (9/10/11 `eos`, banned 1 never sampled); 0 banned IDs in 1,152 tokens at temperature 1.5; byte-fallback IDs returned intact; KV-cache continuation 3/3; only `/completion` called                                                                                                                          |
| greedy parity vs Python | **base 48/48, candidate 48/48** identical sequences (24 tokens, evaluation-bundle prompts) between the App's Rust client and the Python reference client on separate servers; no divergences, so no near-tie analysis was needed                                                                                     |
| lifecycle               | cancellation mid-generation (0.4 s) and recovery; 50 ms request timeout and recovery; crash (SIGKILL) and restart; base → candidate → base switching with the old process gone, one server, and no events on finished requests' topics; unload leaves no process                                                     |
| conversation            | provenance (backend, constitution, runtime build, artifact hash, checkpoint, tokenizer hash, protocol, context); deterministic repeat; 3-turn state; 40-turn history trimmed to 1,957 prompt tokens; oversized turn `CONTEXT_OVERFLOW`; all 64 control spellings in user text give only host control IDs `[3, 9, 4]` |
| analysis                | a forced analysis message is resubmitted after `<                                                                                                                                                                                                                                                                    | end | >` and never reaches the interface; a fully forced analysis-then-final turn shows only the final text |
| host tools              | forced calculator call executed by the host with a host call ID and `juniper-tool-protocol-v1` result; unknown tool `TOOL_NOT_ENABLED`; invalid arguments `INVALID_TOOL_ARGUMENT`; `memory.save` prompted and denied `PERMISSION_DENIED`                                                                             |

Prior A-02 numbers (greedy GGUF vs PyTorch fp32: base 48/48, candidate 47/48,
the one divergence an F16 near-tie) are a different comparison; this record
does not repeat it.

## What the model did (not a capability claim)

The checks above test the host. What the models wrote is recorded as observed:

- The base is not instruction-tuned. Its turns were rejected
  (`GENERATION_TRUNCATED`) and nothing was shown, as designed.
- Under Juniper's full host instructions (about 740 prompt tokens with the
  constitution), the SFT pilot answered "I don't know." to "What is the capital
  of France?", "2 + 3 = 5." correctly, "5" to "What did I say my name was?",
  "2" to "Name one primary color.", and "Italy." to "What is the capital of
  Italy?". The pilot was trained mostly
  without a system message; its quality in this App configuration has not been
  evaluated.
- Tool calls were forced one ID at a time through the real server. After the
  host result the pilot echoed the call JSON as its answer, or produced output
  the parser rejected. The pilot has no tool training; nothing here shows tool
  competence.

## Not covered

- Windows and macOS builds (the `tokenizers`/Oniguruma build there is left to
  CI), and any machine other than FLOWBOX.
- A start-up timeout and cancellation while the model is loading (the 286 MB
  model loads in about a second); a malformed GGUF that matches a pinned hash
  (impossible to construct; the metadata checks are unit-tested).
- External Ollama or OpenAI-compatible servers that serve a Juniper 1 model
  they loaded themselves; the App cannot inspect their files.
- Android: Juniper 1 stays blocked. The JNI renders a chat template and
  tokenizes text with special parsing; a token-ID entry point, an Android
  build of the tokenizer, and on-device evidence are all still needed. Desktop
  evidence does not qualify Android.

## Reproduce

```bash
# golden fixtures (Juniper-1-Data checkout, canonical tokenizer, token shards)
PYTHONPATH=<data>/src python tests/juniper1/generate_golden.py --data-repo <data> \
  --tokenizer <tokenizer.json> --shards <token_shards/j1-base-v0.2.2/cand5-j1tok-24k> \
  --private-out <outside-the-repo>/regression_docs.private.jsonl
JUNIPER1_TOKENIZER=<tokenizer.json> JUNIPER1_GOLDEN_DOCS=<regression_docs.private.jsonl> \
  cargo test --manifest-path src-tauri/Cargo.toml --lib -- --ignored juniper1::golden
# Python reference generations for the parity check
PYTHONPATH=<data>/src python tests/juniper1/python_parity.py --server <b11270 llama-server> \
  --tokenizer <tokenizer.json> --bundle <j1-sft-v1.2/eval> --gguf <base.gguf> --gguf <sft.gguf> \
  --out <outside-the-repo>/python_parity.private.json
# live (see the header of src-tauri/src/live_juniper1.rs for every variable)
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --ignored --nocapture --test-threads=1 live_juniper1
```
