import { describe, expect, it } from 'vitest'
import {
  MODEL_CATALOG,
  parseCatalog,
  isRecommendedModel,
  recommendModel,
  recommendModels,
  type DeviceCapabilities,
  isImportOnly,
  artifactMemoryEstimate,
  licenseAcceptanceValue,
} from './model-catalog'

const device: DeviceCapabilities = {
  os: 'linux',
  deviceName: 'Test machine',
  architecture: 'x86_64',
  cpuArchitecture: 'x86_64',
  logicalCores: 8,
  totalMemoryBytes: 16 * 1024 ** 3,
  availableMemoryBytes: 8 * 1024 ** 3,
  memoryPressure: 'low',
  totalStorageBytes: 500 * 1024 ** 3,
  freeStorageBytes: 20 * 1024 ** 3,
  modelDirectory: '/models',
  gpu: 'unknown',
  acceleration: 'unknown',
}

describe('model catalog', () => {
  it('does not fabricate compatibility or qualification for legacy v1 artifacts', () => {
    const converted = parseCatalog({
      version: 1,
      minimumAppVersion: '0.2.0',
      models: [
        {
          id: 'legacy-model',
          displayName: 'Legacy model',
          organization: 'Example',
          family: 'Example',
          parameterCount: 1,
          description: 'Legacy fixture',
          useCases: ['test'],
          instructionTuned: true,
          architecture: 'unknown',
          sourceRepository: 'https://example.invalid/model',
          sourceRevision: 'legacy',
          license: 'unknown',
          licenseUrl: 'https://example.invalid/license',
          attribution: 'Legacy fixture',
          chatTemplate: 'unknown',
          contextLength: 1,
          minimumRecommendedRamBytes: 1,
          recommendedRamBytes: 1,
          minimumStorageBytes: 1,
          supportedArchitectures: ['unknown'],
          tags: ['legacy'],
          releaseStatus: 'available',
          variants: [
            {
              id: 'legacy-artifact',
              fileName: 'legacy.gguf',
              quantization: 'unknown',
              sizeBytes: 1,
              sha256: 'a'.repeat(64),
              url: 'https://example.invalid/legacy.gguf',
              sourceRevision: 'legacy',
            },
          ],
        },
      ],
    })
    const artifact = converted.models[0]!.artifacts[0]!
    expect(artifact.platforms).toEqual(['unknown'])
    expect(artifact.architectures).toEqual(['unknown'])
    expect(artifact.maturity).toBe('experimental')
    expect(artifact.qualification).toBe('unknown')
  })
  it('pins every download to an exact revision, size, and SHA-256 with stated license terms', () => {
    const downloads = MODEL_CATALOG.models.flatMap((model) =>
      model.artifacts
        .filter((artifact) => !isImportOnly(artifact))
        .map((artifact) => ({ model, artifact })),
    )
    expect(downloads.length).toBeGreaterThan(0)
    for (const { model, artifact } of downloads) {
      for (const file of artifact.files) {
        expect(file.sha256).toMatch(/^[a-f0-9]{64}$/)
        expect(file.url).toBe(
          `${model.sourceRepository}/resolve/${artifact.sourceRevision}/${file.path}`,
        )
      }
    }
    for (const model of MODEL_CATALOG.models) {
      expect(model.license.length).toBeGreaterThan(0)
      expect(model.licenseUrl).toMatch(/^https:\/\//)
      if (model.licenseAcceptance) {
        expect(model.license).not.toMatch(/^(Apache-2\.0|MIT)$/)
        expect(model.licenseTermsRevision).toBeTruthy()
        expect(model.licenseUrl).not.toMatch(/\/blob\/main\//)
        if (model.licenseTermsRevision === model.originalRevision) {
          expect(model.licenseUrl).toContain(`/blob/${model.originalRevision}/`)
        } else {
          expect(model.licenseTermsRevision).toMatch(/-sha256-[a-f0-9]{64}$/)
        }
      }
    }
    const toolCatalogEntries = MODEL_CATALOG.models.filter(
      (model) =>
        model.tags.includes('tools') ||
        model.useCases.some((useCase) => /tool|agent/i.test(useCase)) ||
        /tool|agent/i.test(model.description),
    )
    expect(toolCatalogEntries.length).toBeGreaterThan(0)
    expect(
      toolCatalogEntries.every((model) =>
        model.artifacts.every((artifact) => artifact.capabilities?.tools === 'supported'),
      ),
    ).toBe(true)
  })

  it('does not persist acceptance for reported hashes without bundled terms bytes', () => {
    const gemma = MODEL_CATALOG.models.find((item) => item.id === 'gemma-3-1b-it')!
    const llama = MODEL_CATALOG.models.find((item) => item.id === 'llama-3.2-1b-instruct')!

    expect(licenseAcceptanceValue(gemma)).toBeNull()
    expect(licenseAcceptanceValue(llama)).toBe(
      JSON.stringify([llama.license, llama.licenseUrl, llama.licenseTermsRevision]),
    )
  })

  it('labels popularity with its dated source and keeps quantizations under one release', () => {
    const ranked = MODEL_CATALOG.models.filter((model) => model.popularity)
    const ranks = new Set(ranked.map((model) => model.popularity!.eligibleRank))
    for (let rank = 1; rank <= 10; rank += 1) expect(ranks.has(rank)).toBe(true)
    for (const model of ranked) {
      expect(model.popularity!.source).toBe('huggingface-downloads-30d')
      expect(model.popularity!.snapshot).toMatch(/^2026-10-15/)
    }
    // One entry per release: alternative quantizations are artifacts, not models.
    const releases = MODEL_CATALOG.models.map(
      (model) => model.originalModel ?? model.sourceRepository,
    )
    expect(new Set(releases).size).toBe(releases.length)
  })

  it('ships gpt-oss-20b as an import-only artifact with qualified capabilities', () => {
    const model = MODEL_CATALOG.models.find((item) => item.id === 'gpt-oss-20b')!
    const artifact = model.artifacts[0]!
    expect(isImportOnly(artifact)).toBe(true)
    expect(artifact.sha256).toBe('9d7364f02d9952e158ab462629e72401bec844d2243cc3854b271bb35d33d23d')
    expect(artifact.backendProfile).toBe('gpt-oss-20b-mxfp4-flowbox.v1')
    expect(artifact.capabilities).toEqual({
      tools: 'supported',
      thinking: 'supported',
      generationParameters: ['maxOutput'],
    })
    expect(model.organization).toBe('OpenAI')
    expect(model.attribution).toContain('gpt-oss-20b by OpenAI')
    expect(model.contextLength).toBe(16384)
  })

  it('rejects duplicate ids, malformed hashes, and non-HTTPS variants', () => {
    const malformed = structuredClone(MODEL_CATALOG) as unknown as Record<string, unknown>
    malformed.models = [
      {
        ...MODEL_CATALOG.models[0],
        variants: [
          { ...MODEL_CATALOG.models[0]!.variants[0], sha256: 'bad', url: 'http://unsafe' },
        ],
      },
      MODEL_CATALOG.models[0],
    ]
    expect(() => parseCatalog(malformed)).toThrow(/invalid variant/)
  })

  it('converts a legacy v1 variant catalog into concrete artifacts safely', () => {
    const legacy = {
      version: 1,
      minimumAppVersion: MODEL_CATALOG.minimumAppVersion,
      // The v1 format had no way to express an import-only artifact.
      models: MODEL_CATALOG.models
        .filter(
          (model) =>
            !isImportOnly(model.artifacts[0]!) &&
            model.artifacts.some((artifact) => artifact.files.length === 1),
        )
        .map(({ artifacts, ...model }) => ({
          ...model,
          // v1 could describe only single-file downloads.
          variants: artifacts
            .filter((artifact) => artifact.files.length === 1)
            .map((artifact) => ({
              id: artifact.id,
              fileName: artifact.files[0]!.path,
              quantization: artifact.quantization,
              sizeBytes: artifact.sizeBytes,
              sha256: artifact.files[0]!.sha256,
              url: artifact.sourceUrl ?? artifact.files[0]!.url,
              sourceRevision: artifact.sourceRevision,
            })),
        })),
    }
    const parsed = parseCatalog(legacy)
    expect(parsed.version).toBe(2)
    expect(parsed.models[0]?.artifacts[0]?.runtimeId).toBe('llama.cpp')
  })

  it('explains why a model fits and protects low-storage devices', () => {
    const recommendation = recommendModel(MODEL_CATALOG.models[3]!, device)
    expect(recommendation.storageSafe).toBe(true)
    expect(recommendation.reasons.length).toBeGreaterThan(1)
    expect(
      recommendModel(MODEL_CATALOG.models[3]!, {
        ...device,
        architecture: 'arm64-v8a',
        cpuArchitecture: 'arm64-v8a',
      }).storageSafe,
    ).toBe(true)
    const lowStorage = { ...device, freeStorageBytes: 100 * 1024 ** 2 }
    const unsafe = recommendModel(MODEL_CATALOG.models[0]!, lowStorage)
    expect(unsafe.storageSafe).toBe(false)
    expect(unsafe.fit).toBe('not-recommended')
  })

  it('recommends memory and storage for the selected artifact', () => {
    const model = MODEL_CATALOG.models.find((item) => item.id === 'qwen3-4b')!
    const q4 = model.artifacts.find((artifact) => artifact.quantization === 'Q4_K_M')!
    const q8 = model.artifacts.find((artifact) => artifact.quantization === 'Q8_0')!
    const q4Memory = artifactMemoryEstimate(model, q4)
    const q8Recommendation = recommendModel(
      model,
      {
        ...device,
        availableMemoryBytes: Math.round(5.9 * 1024 ** 3),
        freeStorageBytes: q4.sizeBytes + 1100 * 1024 ** 2,
      },
      q8.id,
    )
    const q4Recommendation = recommendModel(
      model,
      {
        ...device,
        availableMemoryBytes: Math.round(5.9 * 1024 ** 3),
        freeStorageBytes: q4.sizeBytes + 1100 * 1024 ** 2,
      },
      q4.id,
    )

    expect(q8Recommendation.artifact.id).toBe(q8.id)
    expect(q8Recommendation.memory.recommended).toBeGreaterThan(q4Memory.recommended)
    expect(q8Recommendation.fit).toBe('not-recommended')
    expect(q4Recommendation.storageSafe).toBe(true)
    expect(isRecommendedModel(q4Recommendation)).toBe(true)
    expect(isRecommendedModel(q8Recommendation)).toBe(false)
  })

  it('does not treat an unknown architecture as support for an unsupported platform', () => {
    const model = MODEL_CATALOG.models.find((item) => item.id === 'qwen3-1.7b')!
    const recommendation = recommendModel(model, {
      ...device,
      os: 'android',
      architecture: 'unknown',
    })

    expect(recommendation.artifactCompatible).toBe(false)
    expect(recommendation.artifactCompatibilityKnown).toBe(true)
    expect(recommendation.fit).toBe('not-recommended')
  })

  it('does not recommend or enable a download when desktop architecture is unknown', () => {
    const model = MODEL_CATALOG.models.find((item) => item.id === 'qwen3-1.7b')!
    const recommendation = recommendModel(model, {
      ...device,
      os: 'linux',
      architecture: 'unknown',
    })

    expect(recommendation.artifactCompatible).toBe(false)
    expect(recommendation.artifactCompatibilityKnown).toBe(false)
    expect(recommendation.fit).toBe('not-recommended')
    expect(recommendation.reasons[0]).toContain('compatibility cannot be confirmed')
    expect(isRecommendedModel(recommendation)).toBe(false)
  })

  it('ranks an appropriate catalog for a device rather than returning fixed order', () => {
    const recommendations = recommendModels(MODEL_CATALOG, {
      ...device,
      availableMemoryBytes: 3 * 1024 ** 3,
    })
    expect(recommendations[0]?.model.id).not.toBe('qwen3-0.6b-q8')
    expect(recommendations.at(-1)?.fit).toBe('not-recommended')
  })
})
