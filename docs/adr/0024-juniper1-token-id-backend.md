# ADR 0024: Juniper 1 token-ID backend

Status: Implemented in `main` (PR #46); research-only, with the qualification
restrictions below.

Originally proposed on `a02/native-token-id-adapter` with independent review
required before merge. The historical correction below records what occurred;
implementation status does not replace that missing review.

Correction (2026-10-15): this was merged to `main` on 2026-10-14 as
`77f5da7` (PR #46) under the owner's standing delegation. GitHub records no
review on the PR. The reviews before merge were same-vendor (Claude) checks and
are not independent review.

Juniper 1 Base is a research-only base model, published as
[Juniper 1 Base](https://github.com/Cinqic-Research/Juniper-1-Base) (model card: [CinqicX/juniper1-base](https://huggingface.co/CinqicX/juniper1-base)). It is not Juniper's default
model and is not qualified for conversational use; the app offers it only as
a local research import on desktop.

## Context

Juniper 1 finding A-02 (the token-ID qualification recorded in Cinqic's
Juniper 1 development work; the public release is
[Juniper 1 Base](https://github.com/Cinqic-Research/Juniper-1-Base) (model card: [CinqicX/juniper1-base](https://huggingface.co/CinqicX/juniper1-base))): llama.cpp's own text
tokenizer for the exported Juniper 1 GGUF reproduced the canonical tokenizer
(`j1tok`) on 0 of 200 validation documents, and with special-token parsing it
turned literal `<|system|>` or `<|eot|>` text into control IDs. Juniper App
therefore refuses every Juniper 1 GGUF on its text paths (`gguf_guard`, merged
in #45): the Ollama import, the desktop llama-server launch, and the Android
JNI launch.

The Python reference implementation from the Juniper 1 data tooling
(`j1data.jtok`, `j1data.protocol`, `j1data.runtime_client`; not public) showed that llama-server b11270 (`748d4225b`) serves
the model correctly when the host does all tokenization and sends token IDs:
an all-integer prompt on `/completion` is used verbatim, generated IDs come
back in the non-streaming result, `logit_bias` works by integer ID, and IDs
9, 10, and 11 end generation.

## Decision

**Juniper 1 runs only through a native token-ID backend** (`src-tauri/src/juniper1/`).
No text endpoint, chat template, or server-side tokenizer is involved.

- `tokenizer.rs` loads `j1tok` with the `tokenizers` crate 0.23.2 (the
  release the Python reference uses; `onig` regex engine, as in its wheel).
  `tokenizer.json` does not persist `encode_special_tokens`, so the loader
  sets it, checks it took effect, and checks the SHA-256 of the file bytes,
  the 64 control IDs, the 256 byte-fallback IDs, and the vocabulary size.
- `protocol.rs` ports `juniper-chat-v1`: serialization (role and control
  tokens inserted by ID; completed-turn analysis never re-serialized),
  budgeting with whole-turn trimming and in-result tool truncation, and
  generation parsing. Golden fixtures written by the Python reference pin it
  (`tests/juniper1/`).
- `client.rs` ports the token-ID client: integer prompts to `/completion`,
  `stream: false`, `return_tokens`, every sampler set explicitly, integer
  `logit_bias` bans of {0, 1, 2, 3, 5, 12..63}, validated responses
  (in-vocabulary integers, no more than requested, prompt evaluated as sent),
  resubmission after `<|end|>` closes an analysis message within a bounded
  budget, literal loopback URLs only, no environment proxies, no redirects,
  and the listener-ownership check before the key is sent. A start-up
  self-test proves 9/10/11 end generation and a banned ID is never sampled.
- `profile.rs` and `config/backends/juniper1-token-ids.v1.json` pin
  everything that qualifies a model: artifact IDs, sizes, and SHA-256; the
  GGUF metadata (`juniper.io_contract = token-ids-only`, `juniper.protocol`,
  `juniper.tokenizer_sha256`, the source checkpoint manifest digest,
  architecture, context length, vocabulary size, EOT and BOS settings); the
  tokenizer hash; and the runtime build. A file name never qualifies a file.
- `local_runtime.rs` resolves a request's model ID against the token-ID
  profile first. A match starts the server with `start_token_ids`: verify the
  installed file (hash, then GGUF metadata), the tokenizer, and the
  executable's `--version`; launch on loopback with the per-launch key file,
  `-c 2048 -np 1 --no-context-shift --no-mmproj --fit off -ngl 0`, no chat
  template, and `CUDA_VISIBLE_DEVICES` cleared (the qualified configuration is
  CPU); after load check listener ownership, `/props` model path, build, and
  context, then run the self-test. Requests on that route go to
  `juniper1::chat` and never to the provider loop's text path.
- `chat.rs` composes the host's instruction layers as `<|system|>`, the user's
  assistant profile as `<|developer|>`, memories and attachments as framed
  `<|user|>` data, then the dialogue, with no control-token neutralization
  (the host tokenizer cannot produce control IDs from text). A
  `<|tool_call|>` goes through the existing `tool_gate`, schema validation,
  permission prompt, loop and size bounds, and host execution, with a
  host-assigned call ID; the host result returns as a `<|tool|>` message. An
  answer is emitted once, after the turn parses, only if it is non-empty
  and contains no control spelling. Analysis is never emitted, stored, or
  re-sent after its turn.

**The guard stays.** `gguf_guard::refuse_juniper_on_text_path` still runs
before the Ollama import, the generic desktop launch, and the Android JNI
launch. The token-ID route does not call it; it verifies far more instead.

**Not in the catalog.** The two qualified artifacts (the frozen base and the
SFT pilot candidate) are named only by the token-ID profile, never by
`config/models/catalog.json`, and no interface lists them. The candidate is
labeled a qualified pilot, not a production model. Installing them is a
manual developer step (files under the managed model directory with the
profile's names).

**Android stays blocked.** The JNI renders a chat template and tokenizes text
with special parsing. The tokenizer dependency and this backend are not built
for Android; a regression test checks that the Android route still calls the
guard before loading and that no Juniper 1 artifact resolves there.

## Consequences

- One more native dependency on desktop (`tokenizers`, which builds the
  Oniguruma C library). Linux and Android (unchanged) are compiled here;
  Windows and macOS builds rely on CI.
- Juniper 1 needs the b11270 build. It is found through
  `JUNIPER_LLAMA_SERVER_JUNIPER1` or the bundled CUDA runtime (run CPU-only);
  there is no packaged CPU build of 748d4225b yet, so a release without the
  CUDA runtime cannot run Juniper 1.
- The host instructions cost about 700 of the 2,048 tokens. Answers are
  limited to 512 tokens per round. The SFT pilot was trained mostly without a
  system message; its answers under the full constitution are not evaluated
  here beyond protocol validity.
- The protocol has no qualified tool-catalog serializer: tool names appear
  only in the host runtime section, and the pilot was not trained on tool
  use. Tool execution through this backend is verified with forced calls,
  which say nothing about the model's tool competence.
- Token-level streaming is not possible on this transport (llama-server's
  stream drops the IDs of incomplete UTF-8 byte tokens); answers arrive whole.

## Security and privacy

Everything in ADR 0023's resident-runtime section applies. In addition, the
client refuses any URL that is not a literal loopback address with an explicit
port, never consults proxy settings, never follows a redirect, and sends the
key only while the spawned process still owns the listener (Linux). The key is
never part of an event, log, export, or database row. Prompts and model output
are not logged.
