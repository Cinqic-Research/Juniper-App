#!/usr/bin/env node
// Builds catalog entries from a dated research snapshot. Every file name,
// size, SHA-256, and revision comes from the snapshot's Hugging Face API
// records; this script adds only Juniper's descriptions, license labels,
// and memory estimates. Run it when the research is refreshed:
//
//   node scripts/catalog-from-research.mjs docs/research/model-catalog-2026-10-15
//
// It rewrites config/models/catalog.json, keeping entries it does not
// generate (including import-only and research-only entries) unchanged.
/* global console, process */

import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const research = process.argv[2]
if (!research) {
  console.error('usage: node scripts/catalog-from-research.mjs <research folder>')
  process.exit(2)
}
const candidates = JSON.parse(readFileSync(join(research, 'candidate-artifacts.json'), 'utf8'))
const popularity = JSON.parse(readFileSync(join(research, 'popularity-snapshot.json'), 'utf8'))
const licenseSnapshot = JSON.parse(
  readFileSync(join(research, 'license-terms-snapshot.json'), 'utf8'),
)
const gemmaTerms = licenseSnapshot.licenses?.gemma
if (
  !gemmaTerms ||
  typeof gemmaTerms.lastModified !== 'string' ||
  !/^[a-f0-9]{64}$/.test(gemmaTerms.sha256)
) {
  throw new Error('Gemma terms need dated fingerprint metadata with a SHA-256 hash.')
}
const gemmaTermsRevision = `gemma-terms-${gemmaTerms.lastModified}-sha256-${gemmaTerms.sha256}`
const catalogPath = 'config/models/catalog.json'
const catalog = JSON.parse(readFileSync(catalogPath, 'utf8'))

const GIB = 1024 ** 3
const APACHE = 'Apache-2.0'
const MIT = 'MIT'

// Juniper's curated layer. `alternative` adds the research's second
// quantization; `acceptance` means the license must be accepted in Juniper
// before download (the upstream repository is gated or the terms require it).
const SELECTION = [
  {
    release: 'Qwen/Qwen3-1.7B',
    id: 'qwen3-1.7b',
    name: 'Qwen3 1.7B',
    description:
      'A small Qwen3 model with a thinking mode for step-by-step answers; a step up from 0.6B on modest machines.',
    useCases: ['Everyday chat', 'Reasoning'],
    tags: ['general', 'reasoning', 'small'],
    license: APACHE,
  },
  {
    release: 'Qwen/Qwen3-4B',
    id: 'qwen3-4b',
    name: 'Qwen3 4B',
    description:
      'A capable mid-size Qwen3 model for everyday chat and reasoning on laptops with 8 GB or more.',
    useCases: ['Everyday chat', 'Reasoning'],
    tags: ['general', 'reasoning'],
    license: APACHE,
    alternative: true,
  },
  {
    release: 'Qwen/Qwen3-4B-Instruct-2507',
    id: 'qwen3-4b-instruct-2507',
    name: 'Qwen3 4B Instruct 2507',
    description:
      'The July 2025 non-thinking Qwen3 4B update: direct answers without a reasoning phase, with long-context support.',
    useCases: ['Everyday chat', 'Writing'],
    tags: ['general'],
    license: APACHE,
  },
  {
    release: 'Qwen/Qwen3-8B',
    id: 'qwen3-8b',
    name: 'Qwen3 8B',
    description:
      'A strong general Qwen3 model for chat, reasoning, and coding help on machines with 16 GB.',
    useCases: ['Everyday chat', 'Reasoning', 'Coding'],
    tags: ['general', 'reasoning', 'coding'],
    license: APACHE,
  },
  {
    release: 'Qwen/Qwen3-14B',
    id: 'qwen3-14b',
    name: 'Qwen3 14B',
    description:
      'The largest Qwen3 dense model in the catalog; better answers at the cost of memory and speed.',
    useCases: ['Reasoning', 'Writing', 'Coding'],
    tags: ['general', 'reasoning', 'large'],
    license: APACHE,
  },
  {
    release: 'Qwen/Qwen2.5-1.5B-Instruct',
    id: 'qwen2.5-1.5b-instruct',
    name: 'Qwen2.5 1.5B Instruct',
    description: 'A fast small instruct model for everyday chat and drafting.',
    useCases: ['Everyday chat', 'Writing'],
    tags: ['general', 'fast', 'small'],
    license: APACHE,
    alternative: true,
  },
  {
    release: 'Qwen/Qwen2.5-7B-Instruct',
    id: 'qwen2.5-7b-instruct',
    name: 'Qwen2.5 7B Instruct',
    description:
      'A widely used 7B instruct model for writing, analysis, and structured answers. Published as a two-part download.',
    useCases: ['Everyday chat', 'Writing', 'Analysis'],
    tags: ['general', 'writing'],
    license: APACHE,
  },
  {
    release: 'Qwen/Qwen2.5-Coder-7B-Instruct',
    id: 'qwen2.5-coder-7b-instruct',
    name: 'Qwen2.5 Coder 7B Instruct',
    description: 'A coding-focused Qwen2.5 model for explaining, writing, and reviewing code.',
    useCases: ['Coding', 'Code review'],
    tags: ['coding'],
    license: APACHE,
  },
  {
    release: 'meta-llama/Llama-3.2-1B-Instruct',
    id: 'llama-3.2-1b-instruct',
    name: 'Llama 3.2 1B Instruct',
    description: 'Meta’s small Llama 3.2 instruct model for quick everyday chat.',
    useCases: ['Everyday chat', 'Fast chat'],
    tags: ['general', 'fast', 'small'],
    license: 'Llama 3.2 Community License',
    licenseUrl: 'https://huggingface.co/meta-llama/Llama-3.2-1B-Instruct/blob/main/LICENSE.txt',
    acceptance: true,
    alternative: true,
  },
  {
    release: 'meta-llama/Llama-3.2-3B-Instruct',
    id: 'llama-3.2-3b-instruct',
    name: 'Llama 3.2 3B Instruct',
    description: 'Meta’s 3B Llama 3.2 instruct model; a balanced choice for chat and summaries.',
    useCases: ['Everyday chat', 'Summaries'],
    tags: ['general'],
    license: 'Llama 3.2 Community License',
    licenseUrl: 'https://huggingface.co/meta-llama/Llama-3.2-3B-Instruct/blob/main/LICENSE.txt',
    acceptance: true,
  },
  {
    release: 'meta-llama/Llama-3.1-8B-Instruct',
    id: 'llama-3.1-8b-instruct',
    name: 'Llama 3.1 8B Instruct',
    description:
      'Meta’s popular 8B Llama 3.1 instruct model for general chat, writing, and analysis.',
    useCases: ['Everyday chat', 'Writing', 'Analysis'],
    tags: ['general'],
    license: 'Llama 3.1 Community License',
    licenseUrl: 'https://huggingface.co/meta-llama/Llama-3.1-8B-Instruct/blob/main/LICENSE',
    acceptance: true,
  },
  {
    release: 'google/gemma-3-1b-it',
    id: 'gemma-3-1b-it',
    name: 'Gemma 3 1B IT',
    description: 'Google’s small Gemma 3 instruct model for quick text chat.',
    useCases: ['Everyday chat', 'Fast chat'],
    tags: ['general', 'fast', 'small'],
    license: 'Gemma Terms of Use',
    licenseUrl: 'https://ai.google.dev/gemma/terms',
    licenseTermsRevision: gemmaTermsRevision,
    acceptance: true,
    alternative: true,
  },
  {
    release: 'google/gemma-3-4b-it',
    id: 'gemma-3-4b-it',
    name: 'Gemma 3 4B IT (text)',
    description:
      'Google’s Gemma 3 4B instruct model, used for text only: Juniper does not load its image component.',
    useCases: ['Everyday chat', 'Writing'],
    tags: ['general'],
    license: 'Gemma Terms of Use',
    licenseUrl: 'https://ai.google.dev/gemma/terms',
    licenseTermsRevision: gemmaTermsRevision,
    acceptance: true,
  },
  {
    release: 'deepseek-ai/DeepSeek-R1-Distill-Qwen-1.5B',
    id: 'deepseek-r1-distill-qwen-1.5b',
    name: 'DeepSeek R1 Distill Qwen 1.5B',
    description:
      'A small Qwen2.5 model distilled from DeepSeek-R1 reasoning traces; it reasons at length before answering. Not the original DeepSeek-R1.',
    useCases: ['Reasoning', 'Math'],
    tags: ['reasoning', 'small'],
    license: MIT,
  },
  {
    release: 'deepseek-ai/DeepSeek-R1-Distill-Qwen-7B',
    id: 'deepseek-r1-distill-qwen-7b',
    name: 'DeepSeek R1 Distill Qwen 7B',
    description:
      'A 7B Qwen2.5 model distilled from DeepSeek-R1 reasoning traces, for math and step-by-step problems. Not the original DeepSeek-R1.',
    useCases: ['Reasoning', 'Math'],
    tags: ['reasoning'],
    license: MIT,
  },
  {
    release: 'mistralai/Mistral-7B-Instruct-v0.3',
    id: 'mistral-7b-instruct-v0.3',
    name: 'Mistral 7B Instruct v0.3',
    description: 'Mistral AI’s Apache-licensed 7B instruct model for general chat and writing.',
    useCases: ['Everyday chat', 'Writing'],
    tags: ['general'],
    license: APACHE,
    licenseUrl: 'https://www.apache.org/licenses/LICENSE-2.0',
  },
  {
    release: 'mistralai/Ministral-3-3B-Instruct-2512',
    id: 'ministral-3-3b-instruct-2512',
    name: 'Ministral 3 3B Instruct',
    description: 'Mistral AI’s small December 2025 instruct model, used for text only.',
    useCases: ['Everyday chat', 'Writing'],
    tags: ['general', 'small'],
    license: APACHE,
    licenseUrl: 'https://www.apache.org/licenses/LICENSE-2.0',
  },
  {
    release: 'microsoft/Phi-4-mini-instruct',
    id: 'phi-4-mini-instruct',
    name: 'Phi-4 mini Instruct',
    description: 'Microsoft’s 3.8B Phi-4 mini model for reasoning, math, and concise answers.',
    useCases: ['Reasoning', 'Math', 'Everyday chat'],
    tags: ['reasoning'],
    license: MIT,
    alternative: true,
  },
  {
    release: 'HuggingFaceTB/SmolLM3-3B',
    id: 'smollm3-3b',
    name: 'SmolLM3 3B',
    description:
      'Hugging Face’s fully open 3B model with optional reasoning; the next step up from SmolLM2.',
    useCases: ['Everyday chat', 'Reasoning'],
    tags: ['general', 'small', 'offline'],
    license: APACHE,
  },
  {
    release: 'ibm-granite/granite-4.0-micro',
    id: 'granite-4.0-micro',
    name: 'Granite 4.0 Micro',
    description: 'IBM’s 3B Granite 4.0 instruct model for business writing and summaries.',
    useCases: ['Writing', 'Summaries'],
    tags: ['general'],
    license: APACHE,
  },
  {
    release: 'ibm-granite/granite-4.0-h-tiny',
    id: 'granite-4.0-h-tiny',
    name: 'Granite 4.0 H Tiny',
    description:
      'IBM’s hybrid Mamba-transformer mixture-of-experts model (7B total, about 1B active) with a light memory footprint for long inputs.',
    useCases: ['Summaries', 'Long documents'],
    tags: ['general'],
    license: APACHE,
  },
  {
    release: 'openbmb/MiniCPM5-2B',
    id: 'minicpm5-2b',
    name: 'MiniCPM5 2B',
    description: 'OpenBMB’s compact 2B model tuned for capable on-device chat.',
    useCases: ['Everyday chat', 'Fast chat'],
    tags: ['general', 'small'],
    license: APACHE,
  },
  {
    release: 'nvidia/NVIDIA-Nemotron-3-Nano-4B-BF16',
    id: 'nemotron-3-nano-4b',
    name: 'Nemotron 3 Nano 4B',
    description: 'NVIDIA’s small hybrid Mamba-transformer reasoning model.',
    useCases: ['Reasoning', 'Everyday chat'],
    tags: ['reasoning'],
    license: 'NVIDIA Open Model License',
    acceptance: true,
  },
]

const popular = new Map(
  popularity.ranked.map((item, index) => [item.upstreamId, { ...item, eligibleRank: index + 1 }]),
)

function slug(value) {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9.]+/g, '-')
    .replace(/^-|-$/g, '')
}

/** Weights plus an estimated KV cache: about 0.25 GiB per billion parameters
 * at 16k tokens for standard attention (less for hybrid models), plus runtime
 * overhead. These are estimates and the interface says so. */
function memory(fileBytes, parameterCount, architecture) {
  const billions = parameterCount / 1e9
  const hybrid = ['granitehybrid', 'nemotron_h', 'lfm2'].includes(architecture)
  const kvAtFullContext = Math.max(0.1, billions * (hybrid ? 0.06 : 0.25)) * GIB
  return {
    minimum: Math.round(fileBytes + kvAtFullContext / 4 + 0.5 * GIB),
    recommended: Math.round(fileBytes + kvAtFullContext + 1.5 * GIB),
  }
}

const generated = []
for (const choice of SELECTION) {
  const research = candidates.find((candidate) => candidate.releaseId === choice.release)
  if (!research || !research.complete)
    throw new Error(`No complete research record for ${choice.release}`)
  const artifacts = (
    choice.alternative ? research.artifacts.slice(0, 2) : research.artifacts.slice(0, 1)
  ).map((artifact) => ({
    id: `${choice.id}-${slug(artifact.quantization)}`,
    runtimeId: 'llama.cpp',
    format: 'GGUF',
    platforms: ['linux', 'windows'],
    architectures: ['x86_64'],
    quantization: artifact.quantization,
    sizeBytes: artifact.totalBytes,
    ...(artifact.files.length === 1 ? { sha256: artifact.files[0].sha256 } : {}),
    sourceRevision: research.ggufRevision,
    files: [...artifact.files]
      .sort((left, right) => left.path.localeCompare(right.path))
      .map((file) => ({
        path: file.path,
        sizeBytes: file.sizeBytes,
        sha256: file.sha256,
        url: file.url,
      })),
    maturity: 'beta',
    qualification: 'package-only',
  }))
  const largest = Math.max(...artifacts.map((artifact) => artifact.sizeBytes))
  const smallest = Math.min(...artifacts.map((artifact) => artifact.sizeBytes))
  const estimate = memory(smallest, research.parameterCount, research.architecture)
  const rank = popular.get(choice.release)
  const licenseUrl = choice.licenseUrl ?? research.licenseUrl
  const originalLicensePrefix = `https://huggingface.co/${research.originalRepo}/blob/main/`
  const pinnedLicenseUrl = licenseUrl.startsWith(originalLicensePrefix)
    ? `${originalLicensePrefix.replace('/blob/main/', `/blob/${research.originalRevision}/`)}${licenseUrl.slice(originalLicensePrefix.length)}`
    : licenseUrl
  generated.push({
    id: choice.id,
    displayName: choice.name,
    organization: research.organization,
    family: research.family,
    parameterCount: research.parameterCount,
    description: choice.description,
    useCases: choice.useCases,
    instructionTuned: true,
    architecture: research.architecture,
    sourceRepository: `https://huggingface.co/${research.ggufRepo}`,
    sourceRevision: research.ggufRevision,
    originalModel: `https://huggingface.co/${research.originalRepo}`,
    originalRevision: research.originalRevision,
    license: choice.license,
    licenseUrl: pinnedLicenseUrl,
    ...(choice.acceptance
      ? {
          licenseAcceptance: 'required',
          licenseTermsRevision: choice.licenseTermsRevision ?? research.originalRevision,
        }
      : {}),
    attribution: `${research.displayName} by ${research.organization}; GGUF ${
      research.quantizer === research.organization
        ? 'published by the developer'
        : `conversion by ${research.quantizer}`
    } (${research.ggufRepo}).`,
    chatTemplate: 'gguf-embedded',
    contextLength: research.contextLength,
    minimumRecommendedRamBytes: estimate.minimum,
    recommendedRamBytes: estimate.recommended,
    minimumStorageBytes: largest + GIB,
    supportedArchitectures: ['x86_64'],
    tags: choice.tags,
    releaseStatus: 'available',
    ...(rank
      ? {
          popularity: {
            source: 'huggingface-downloads-30d',
            snapshot: popularity.snapshotUtc,
            eligibleRank: rank.eligibleRank,
            downloads: rank.downloads,
          },
        }
      : {}),
    artifacts,
  })
}

const ids = new Set(generated.map((entry) => entry.id))
// Existing entries keep their identities; add popularity evidence to those
// the snapshot ranks, without changing their artifacts.
const RELEASE_OF = {
  'qwen3-0.6b-q8': 'Qwen/Qwen3-0.6B',
  'qwen2.5-0.5b-instruct-q4': 'Qwen/Qwen2.5-0.5B-Instruct',
  'gpt-oss-20b': 'openai/gpt-oss-20b',
}
const kept = catalog.models
  .filter((entry) => !ids.has(entry.id))
  .map((entry) => {
    const rank = popular.get(RELEASE_OF[entry.id])
    return rank
      ? {
          ...entry,
          popularity: {
            source: 'huggingface-downloads-30d',
            snapshot: popularity.snapshotUtc,
            eligibleRank: rank.eligibleRank,
            downloads: rank.downloads,
          },
        }
      : entry
  })
catalog.models = [...kept, ...generated]
writeFileSync(catalogPath, `${JSON.stringify(catalog, null, 2)}\n`)
console.log(
  `catalog: ${kept.length} kept, ${generated.length} generated, ${catalog.models.length} total`,
)
