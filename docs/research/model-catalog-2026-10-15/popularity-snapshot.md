# Hugging Face text-generation popularity snapshot (2026-10-15)

Snapshot taken 2026-10-15 01:53:53 AM EDT (UTC 2026-10-15T05:53:53Z). Source: public Hugging Face Hub API, no auth token used.

## Method

Query URL (raw response saved to the session scratchpad as `catalog/raw/listing.json`, 300 repos):

```
https://huggingface.co/api/models?pipeline_tag=text-generation&sort=downloads&direction=-1&limit=300&full=false&expand[]=downloads&expand[]=tags&expand[]=cardData&expand[]=gated&expand[]=sha&expand[]=likes
```

Per-repo follow-ups (all unauthenticated, metadata only):

- `https://huggingface.co/api/models/{repo}` gives sha, gated, cardData.license/license_link, siblings, safetensors.total, gguf metadata
- `https://huggingface.co/api/models?filter=base_model:quantized:{upstream}&filter=gguf&sort=downloads&direction=-1&limit=100` lists GGUF mirrors
- `https://huggingface.co/api/models/{gguf_repo}/tree/{sha}?recursive=true` gives path, size and lfs.oid (sha256) per file

Normalization: a repo is a **mirror** when it carries a `base_model:quantized:` tag, or (no tag) is clearly a quant/re-upload by name (e.g. `-GGUF`, `-FP8`, `-NVFP4`, `-MLX`, `unsloth/<same name>`). Mirrors are mapped to their upstream and **never added** to upstream counts. Their downloads are reported separately: `mirrorDownloadsInSnapshot` (mirrors inside this top-300) and `ggufMirrorDownloadsTop100` (sum over the top-100 GGUF mirrors from the filter query). Ranking uses only the upstream repo's own `downloads`.

Scope: listing ranks 1-110 were evaluated in detail. Ranks below that were classified only automatically (mirror, tiny/test). 97 repos below the cutoff were not evaluated.

## Eligibility rules (declared before ranking)

1 instruction/chat tuned (base/pretrained, draft, embedding, reward, classifier, test models excluded).
2 declared license in card metadata AND license text resolvable (repo license file, cardData.license_link, standard SPDX apache-2.0/mit text, or licensor terms URL in README for Gemma).
3 name-matched GGUF repo from publisher or ggml-org/Qwen/unsloth/bartowski/lmstudio-community/ibm-granite/LiquidAI/mistralai/google/microsoft/nvidia/openbmb.
4 immutable commit sha for upstream and GGUF repo, and lfs.oid sha256 for every .gguf file in the chosen GGUF repo tree.

Exclusion classes applied first: base/pretrained-only, tiny test/random, speculative-decoding drafts, embedding/reward/classifier/non-chat, and merges/derivatives of undeclared lineage.

Hardware fit is **not** an eligibility rule. It is flagged in the `≤16B` column (total params ≤ 16e9 from `safetensors.total`). Releases with an `n` are not realistic for a local consumer desktop. See exclusions-and-blockers.md.

## Interpretation limits

- HF `downloads` counts **requests over a trailing 30 days** (see https://huggingface.co/docs/hub/models-download-stats). It is not a count of users, installs, or quality. CI systems, re-downloads and sharded files inflate it. Counts also differ in kind between library types.
- The listing is filtered on `pipeline_tag=text-generation`. Multimodal chat models tagged `image-text-to-text` (Gemma 3 4B+, Ministral 3, Qwen3.5/3.6/3.8, Gemma 4, Kimi-K3) are **absent by construction**, not because they are unpopular.
- The counts are a single point-in-time snapshot. Re-running the same URL later will give different numbers.
- Gated repos (`manual`) are still counted. Gating only affects whether the original weights can be fetched without an account.

## Ranked eligible upstream releases (top 20 of 41 eligible within cutoff)

| #   | listing rank | upstream id                          | downloads (30d) | family        | params (B) | ≤16B | license             | gated  | included GGUF repo                     |
| --- | ------------ | ------------------------------------ | --------------: | ------------- | ---------: | ---- | ------------------- | ------ | -------------------------------------- |
| 1   | 1            | `Qwen/Qwen3-0.6B`                    |      30,452,693 | Qwen3         |        0.8 | y    | apache-2.0          | no     | `Qwen/Qwen3-0.6B-GGUF`                 |
| 2   | 3            | `Qwen/Qwen3-4B`                      |      12,585,505 | Qwen3         |        4.0 | y    | apache-2.0          | no     | `Qwen/Qwen3-4B-GGUF`                   |
| 3   | 4            | `Qwen/Qwen3-8B`                      |       9,739,109 | Qwen3         |        8.2 | y    | apache-2.0          | no     | `Qwen/Qwen3-8B-GGUF`                   |
| 4   | 5            | `Qwen/Qwen2.5-7B-Instruct`           |       7,357,696 | Qwen2.5       |        7.6 | y    | apache-2.0          | no     | `Qwen/Qwen2.5-7B-Instruct-GGUF`        |
| 5   | 6            | `Qwen/Qwen3-14B`                     |       6,913,178 | Qwen3         |       14.8 | y    | apache-2.0          | no     | `Qwen/Qwen3-14B-GGUF`                  |
| 6   | 7            | `meta-llama/Llama-3.2-1B-Instruct`   |       6,380,305 | Llama 3.x     |        1.2 | y    | llama3.2            | manual | `unsloth/Llama-3.2-1B-Instruct-GGUF`   |
| 7   | 8            | `Qwen/Qwen2.5-0.5B-Instruct`         |       6,276,110 | Qwen2.5       |        0.5 | y    | apache-2.0          | no     | `Qwen/Qwen2.5-0.5B-Instruct-GGUF`      |
| 8   | 9            | `meta-llama/Llama-3.1-8B-Instruct`   |       6,034,692 | Llama 3.x     |        8.0 | y    | llama3.1            | manual | `unsloth/Llama-3.1-8B-Instruct-GGUF`   |
| 9   | 10           | `Qwen/Qwen2.5-1.5B-Instruct`         |       6,018,399 | Qwen2.5       |        1.5 | y    | apache-2.0          | no     | `Qwen/Qwen2.5-1.5B-Instruct-GGUF`      |
| 10  | 11           | `openai/gpt-oss-20b`                 |       5,982,456 | gpt-oss       |       20.9 | n    | apache-2.0          | no     | `ggml-org/gpt-oss-20b-GGUF`            |
| 11  | 16           | `deepseek-ai/DeepSeek-V3.2`          |       4,844,451 | DeepSeek-V3   |      685.4 | n    | mit                 | no     | `unsloth/DeepSeek-V3.2-GGUF`           |
| 12  | 20           | `deepseek-ai/DeepSeek-V4-Flash-0731` |       4,438,887 | DeepSeek-V4   |      304.2 | n    | mit                 | no     | `ggml-org/DeepSeek-V4-Flash-0731-GGUF` |
| 13  | 21           | `Qwen/Qwen2.5-3B-Instruct`           |       4,299,162 | Qwen2.5       |        3.1 | y    | other:qwen-research | no     | `Qwen/Qwen2.5-3B-Instruct-GGUF`        |
| 14  | 22           | `Qwen/Qwen3-1.7B`                    |       4,034,338 | Qwen3         |        2.0 | y    | apache-2.0          | no     | `Qwen/Qwen3-1.7B-GGUF`                 |
| 15  | 24           | `openai/gpt-oss-120b`                |       3,455,569 | gpt-oss       |      116.8 | n    | apache-2.0          | no     | `ggml-org/gpt-oss-120b-GGUF`           |
| 16  | 27           | `Qwen/Qwen3-4B-Instruct-2507`        |       3,293,032 | Qwen3         |        4.0 | y    | apache-2.0          | no     | `unsloth/Qwen3-4B-Instruct-2507-GGUF`  |
| 17  | 28           | `Qwen/Qwen3-32B`                     |       3,235,453 | Qwen3         |       32.8 | n    | apache-2.0          | no     | `Qwen/Qwen3-32B-GGUF`                  |
| 18  | 34           | `google/gemma-3-1b-it`               |       2,247,811 | Gemma         |        1.0 | y    | gemma               | manual | `ggml-org/gemma-3-1b-it-GGUF`          |
| 19  | 37           | `zai-org/GLM-5.3`                    |       1,779,954 | GLM-5         |      753.3 | n    | other:glm-5.3       | no     | `unsloth/GLM-5.3-GGUF`                 |
| 20  | 38           | `Qwen/Qwen2.5-Coder-14B-Instruct`    |       1,755,766 | Qwen2.5-Coder |       14.8 | y    | apache-2.0          | no     | `Qwen/Qwen2.5-Coder-14B-Instruct-GGUF` |

Eligible releases 21 and below (same rules): 21. `Qwen/Qwen2.5-32B-Instruct` (1,605,309); 22. `Qwen/Qwen2.5-Coder-7B-Instruct` (1,589,786); 23. `nvidia/NVIDIA-Nemotron-3-Nano-4B-BF16` (1,565,119); 24. `deepseek-ai/DeepSeek-V3` (1,470,043); 25. `meta-llama/Llama-3.2-3B-Instruct` (1,352,983); 26. `zai-org/GLM-4.7-Flash` (1,346,404); 27. `deepseek-ai/DeepSeek-R1-Distill-Qwen-1.5B` (1,333,486); 28. `Qwen/Qwen3-30B-A3B` (1,290,653); 29. `deepseek-ai/DeepSeek-V3-0324` (1,282,489); 30. `openbmb/MiniCPM5-2B` (1,247,513); 31. `deepseek-ai/DeepSeek-R1` (1,228,430); 32. `Qwen/Qwen2.5-14B-Instruct` (1,117,538); 33. `nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B-BF16` (1,053,158); 34. `Qwen/Qwen3-30B-A3B-Instruct-2507` (1,036,773); 35. `nvidia/NVIDIA-Nemotron-3-Super-120B-A12B-BF16` (1,025,723); 36. `nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B-BF16` (902,153); 37. `google/gemma-2-9b-it` (829,988); 38. `Qwen/Qwen2.5-Coder-32B-Instruct` (774,742); 39. `zai-org/GLM-5` (772,323); 40. `HuggingFaceTB/SmolLM3-3B` (764,561); 41. `deepseek-ai/DeepSeek-Coder-V2-Lite-Instruct` (731,256).

Notes:

- DeepSeek-V3.2 and V4-Flash-0731 have no HF chat template (they ship a custom `encoding/` script) but are chat models. DeepSeek-V3 declares no `cardData.license`; its license comes from the `LICENSE-MODEL` file.
- Gemma passes rule 2 through the Gemma Terms URL in its README (https://ai.google.dev/gemma/terms). There is no license file in the repo.
- Upstream revisions (immutable sha) and mirror lists are in `popularity-snapshot.json`.

## Exclusions (evaluated range plus auto-classified test models)

| listing rank | repo                                                |  downloads | reason                                                                                                                                                                                |
| ------------ | --------------------------------------------------- | ---------: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2            | `openai-community/gpt2`                             | 16,001,636 | base/pretrained-only (no chat tuning)                                                                                                                                                 |
| 12           | `facebook/opt-125m`                                 |  5,948,464 | base/pretrained-only                                                                                                                                                                  |
| 13           | `trl-internal-testing/tiny-Qwen2ForCausalLM-2.5`    |  5,668,842 | tiny test/random model                                                                                                                                                                |
| 14           | `RadixArk/Kimi-K3-DSpark`                           |  5,371,571 | speculative-decoding draft model (DSparkDraftModel), not a chat model; no license declared                                                                                            |
| 19           | `dphn/dolphin-2.9.1-yi-1.5-34b`                     |  4,665,833 | rule3: no name-matched GGUF from declared reputable quantizers (only community quantizers: mradermacher/dolphin-2.9.1-yi-1.5-34b-i1-GGUF, mradermacher/dolphin-2.9.1-yi-1.5-34b-GGUF) |
| 25           | `EleutherAI/pythia-160m`                            |  3,446,926 | base/pretrained-only                                                                                                                                                                  |
| 36           | `distilbert/distilgpt2`                             |  1,874,892 | base/pretrained-only                                                                                                                                                                  |
| 41           | `mistralai/Mistral-7B-Instruct-v0.2`                |  1,626,049 | rule3: no name-matched GGUF from declared reputable quantizers (only community quantizers: TheBloke/Mistral-7B-Instruct-v0.2-GGUF, mozilla-ai/Mistral-7B-Instruct-v0.2-llamafile)     |
| 47           | `HuggingFaceTB/SmolLM2-135M`                        |  1,520,896 | base/pretrained-only                                                                                                                                                                  |
| 51           | `Qwen/Qwen2.5-0.5B`                                 |  1,399,614 | base/pretrained-only                                                                                                                                                                  |
| 52           | `EleutherAI/pythia-70m-deduped`                     |  1,393,984 | base/pretrained-only                                                                                                                                                                  |
| 53           | `ornith-ai/Ornith-1.0-35B`                          |  1,379,293 | lineage: no base_model declared (Qwen3.5-arch derivative of unstated origin); license_link points to a different org (deepreinforce-ai); no license file in repo                      |
| 58           | `bigscience/bloomz-560m`                            |  1,285,515 | non-chat (multitask prompted, no chat template)                                                                                                                                       |
| 65           | `TinyLlama/TinyLlama-1.1B-Chat-v1.0`                |  1,131,294 | rule3: no name-matched GGUF from declared reputable quantizers (only community quantizers: TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF, second-state/TinyLlama-1.1B-Chat-v1.0-GGUF)        |
| 71           | `openbmb/MiniCPM5-2B-DSpark`                        |  1,084,185 | speculative-decoding draft model, not a chat model                                                                                                                                    |
| 74           | `sshleifer/tiny-gpt2`                               |  1,046,989 | tiny test/random model                                                                                                                                                                |
| 80           | `openai-community/gpt2-large`                       |    959,459 | base/pretrained-only                                                                                                                                                                  |
| 94           | `trl-internal-testing/tiny-Qwen3ForCausalLM`        |    832,502 | tiny test/random model                                                                                                                                                                |
| 96           | `ornith-ai/Ornith-1.0-9B`                           |    813,366 | lineage: no base_model declared (Qwen3.5-arch derivative of unstated origin); license_link points to a different org (deepreinforce-ai); no license file in repo                      |
| 97           | `deepseek-ai/DeepSeek-V4-Flash`                     |    806,067 | superseded by -0731 snapshot; no chat template tag (evaluate -0731 instead)                                                                                                           |
| 99           | `Qwen/Qwen2.5-7B`                                   |    781,749 | base/pretrained-only                                                                                                                                                                  |
| 103          | `Qwen/Qwen3-8B-Base`                                |    758,111 | base/pretrained-only                                                                                                                                                                  |
| 115          | `meta-llama/Llama-2-7b-hf`                          |    701,229 | base/pretrained-only                                                                                                                                                                  |
| 116          | `meta-llama/Llama-3.1-8B`                           |    680,490 | base/pretrained-only                                                                                                                                                                  |
| 117          | `EleutherAI/pythia-14m`                             |    677,445 | base/pretrained-only                                                                                                                                                                  |
| 120          | `Qwen/Qwen-72B`                                     |    667,233 | base/pretrained-only                                                                                                                                                                  |
| 123          | `Qwen/Qwen2.5-1.5B`                                 |    665,108 | base/pretrained-only                                                                                                                                                                  |
| 124          | `meta-llama/Llama-3.2-1B`                           |    650,466 | base/pretrained-only                                                                                                                                                                  |
| 135          | `peft-internal-testing/tiny-random-OPTForCausalLM`  |    587,183 | tiny test/random model                                                                                                                                                                |
| 155          | `llamafactory/tiny-random-Llama-3`                  |    529,497 | tiny test/random model                                                                                                                                                                |
| 165          | `trl-internal-testing/tiny-random-LlamaForCausalLM` |    493,502 | tiny test/random model                                                                                                                                                                |
| 206          | `hmellor/tiny-random-LlamaForCausalLM`              |    386,132 | tiny test/random model                                                                                                                                                                |
| 240          | `hmellor/tiny-random-Gemma2ForCausalLM`             |    326,767 | tiny test/random model                                                                                                                                                                |
| 248          | `trl-internal-testing/tiny-GptOssForCausalLM`       |    305,828 | tiny test/random model                                                                                                                                                                |
| 290          | `trl-internal-testing/tiny-Qwen3MoeForCausalLM`     |    249,300 | tiny test/random model                                                                                                                                                                |

Mirror/quantization repos inside the snapshot: 127 (listed with upstream mapping in the JSON). Examples: `Qwen/Qwen3-14B-AWQ`, `unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF`, and `prism-ml/Ternary-Bonsai-2-27B-gguf` (mapped to `Qwen/Qwen3.8-27B`, an image-text-to-text upstream outside this listing).
