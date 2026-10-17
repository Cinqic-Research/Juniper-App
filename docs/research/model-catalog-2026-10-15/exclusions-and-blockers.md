# Exclusions and blockers (2026-10-15 snapshot)

All facts come from public HF API metadata fetched 2026-10-15 01:53 AM EDT. Sizes are exact `size` values from the tree API, or `safetensors.total` param counts. Machine-readable detail is in `candidate-artifacts.json` (`blockers`, `notes`) and `popularity-snapshot.json` (`exclusions`).

## Not includable: size (not consumer-desktop feasible)

- **Kimi (Moonshot AI)**: Kimi-K3 has 2,779.9B params (image-text-to-text, `other:kimi-k3`). Kimi-K2-Instruct has 1,026.4B (`other:modified-mit`). The smallest open-weight text Kimi is **Kimi-Linear-48B-A3B-Instruct** (49.1B total, MIT). Its only GGUF from a declared quantizer is bartowski's, and Q4_K_M is a single 30,061,058,720-byte file. It also needs llama.cpp `kimi-linear` support. Kimi-VL-A3B-Instruct (16.4B) is multimodal and has only community (mradermacher) GGUFs. Blocker: size, plus no official GGUF.
- **GLM-5 / 5.3 (Z.ai)**: about 753B params. Blocker: size.
- **GLM-4.7-Flash**: 31.2B MoE. The ggml-org Q4_K file is 18,244,193,920 bytes and Q8_0 is 31,842,799,232 bytes. The manifest is complete, but it fits only a large/advanced tier with 24 GB or more of RAM. There is no license file or link; the license is MIT in cardData only. No smaller current text GLM appeared: `glm-edge-4b-chat-gguf` exists but is old (GLM-Edge), and GLM-4.6V-Flash is multimodal.
- **DeepSeek V3 / V3-0324 / V3.2 / R1** (684-685B) and **DeepSeek-V4-Flash-0731** (304.2B). Blocker: size. Use the R1-Distill-Qwen 1.5B/7B instead (manifests complete).
- **gpt-oss-120b** (116.8B) and the **Nemotron 3 Super 120B / Ultra 550B** models. Blocker: size.
- **Qwen3-32B, Qwen2.5-32B/Coder-32B, Qwen3-30B-A3B family, Nemotron-3-Nano-30B-A3B**: 30-33B. Q4 GGUFs are about 18-20 GB. These are large-tier only.
- **gpt-oss-20b**: its manifest is complete, but the only artifact is MXFP4 at 12,109,566,624 bytes. It needs about 16 GB of RAM. Treat it as large-tier.

## Not includable: license / gating

- **Qwen2.5-3B-Instruct** (rank 13) uses `other:qwen-research` (Qwen Research License, LICENSE file in repo). That license restricts commercial use, so it is excluded from a shipped catalog unless the owner accepts the terms. The 0.5B, 1.5B and 7B models and the Coder variants are Apache-2.0.
- **Llama 3.1 / 3.2** have gated originals (`manual`) under the custom Llama Community License plus AUP. The unsloth GGUF mirrors are ungated, but the license still applies, including the "Built with Llama" attribution. This is not a hard blocker, but it needs a license-acceptance UI.
- **Gemma 3** has a gated original under the Gemma Terms of Use (https://ai.google.dev/gemma/terms), which is not SPDX. The repo has no license file. The ggml-org GGUF is ungated. Same handling as Llama.
- **LFM2 (Liquid AI)** uses `other:lfm1.0`, the LFM Open License v1.0 (LICENSE file present). It is not OSI-approved and has a commercial-use threshold, so it needs owner review before shipping. Newer **LFM2.5** official GGUFs exist and are more downloaded.
- **Nemotron 3 Nano 4B** uses `other:nvidia-nemotron-open-model-license` (LICENSE file present). It is a custom license that needs review. The official GGUF ships only Q4_K_M.
- **Mistral-7B-Instruct-v0.3, Ministral-3-3B-Instruct-2512, MiniCPM5-2B, SmolLM3-3B, Granite 4.0**: each declares Apache-2.0 in cardData but has no LICENSE file in the HF repo. The licenseUrl is null for both Mistral models and is a README-linked generic or GitHub URL for the others. This is acceptable for SPDX Apache-2.0, but the app should bundle the canonical Apache-2.0 text itself.

## Not includable: provenance / integrity

- **Ornith (ornith-ai/Ornith-1.0/1.5, 9B/35B/397B)** is highly downloaded but declares no `base_model` (its architecture is Qwen3.5-family). Its `license_link` points to another org (`deepreinforce-ai`), and there is no license file in the repo. The lineage is not verifiable, so it fails the lineage and rule-2 checks.
- **CMSManhattan/JiRack\***: unknown lineage (Qwen2 architecture, no base_model) and no license file.
- **RadixArk/Kimi-K3-DSpark, openbmb/MiniCPM5-2B-DSpark**: speculative-decoding draft models, not chat models. Kimi-K3-DSpark also declares no license.
- **dolphin-2.9.1-yi-1.5-34b, Mistral-7B-Instruct-v0.2, TinyLlama-1.1B-Chat-v1.0**: no name-matched GGUF from a declared reputable quantizer. Only TheBloke, mradermacher and second-state publish them (rule 3).

## Absent from the popularity listing by construction

Models tagged `image-text-to-text` cannot appear in a `pipeline_tag=text-generation` query. That covers Gemma 3 4B+, Gemma 4, Ministral 3, Qwen3.5/3.6/3.8, Kimi-K3 and Kimi-VL. Gemma-3-4B-IT and Ministral-3-3B-Instruct-2512 still have complete text-only manifests (mmproj omitted).

## No unsupported-architecture or missing-sha256 blockers among chosen artifacts

Every chosen GGUF file in `candidate-artifacts.json` has a tree-API `lfs.oid` (sha256) and `size` at a pinned commit. Architectures that need a recent llama.cpp build are: `granitehybrid`, `lfm2`, `nemotron_h`, `smollm3`, `mistral3`, `gpt-oss`, `deepseek2` (GLM-4.7-Flash) and `kimi-linear`. Pin a llama.cpp version that loads each one before enabling it.
