import { describe, expect, it } from 'vitest'
import {
  MODEL_CATALOG,
  parseCatalog,
  recommendModel,
  recommendModels,
  type DeviceCapabilities,
  isImportOnly,
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
  it('ships four under-1B downloads with verified HTTPS variants', () => {
    const downloads = MODEL_CATALOG.models.filter((model) => !isImportOnly(model.artifacts[0]!))
    expect(downloads).toHaveLength(4)
    expect(downloads.every((model) => model.parameterCount < 1_000_000_000)).toBe(true)
    for (const model of MODEL_CATALOG.models) {
      expect(model.variants[0]?.sha256).toMatch(/^[a-f0-9]{64}$/)
      expect(model.license).toBe('Apache-2.0')
    }
    for (const model of downloads) expect(model.variants[0]?.url).toMatch(/^https:\/\//)
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
        .filter((model) => !isImportOnly(model.artifacts[0]!))
        .map(({ artifacts, ...model }) => ({
          ...model,
          variants: artifacts.map((artifact) => ({
            id: artifact.id,
            fileName: artifact.files[0]!.path,
            quantization: artifact.quantization,
            sizeBytes: artifact.sizeBytes,
            sha256: artifact.sha256,
            url: artifact.sourceUrl,
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

  it('ranks an appropriate catalog for a device rather than returning fixed order', () => {
    const recommendations = recommendModels(MODEL_CATALOG, {
      ...device,
      availableMemoryBytes: 3 * 1024 ** 3,
    })
    expect(recommendations[0]?.model.id).not.toBe('qwen3-0.6b-q8')
    expect(recommendations.at(-1)?.fit).toBe('not-recommended')
  })
})
