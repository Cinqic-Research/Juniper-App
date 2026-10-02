import { describe, expect, it } from 'vitest'
import { defaultAssistant } from './defaults'
import {
  MAX_ASSISTANT_IMPORT_BYTES,
  parseAssistant,
  serializeAssistant,
  validateAssistant,
} from './assistant'

describe('assistant format', () => {
  it('round-trips the portable versioned format', () => {
    const parsed = parseAssistant(serializeAssistant(defaultAssistant))
    expect(parsed.id).toBe(defaultAssistant.id)
    expect(parsed.systemPrompt).toBe(defaultAssistant.systemPrompt)
  })

  it('rejects executable-looking or malformed imports', () => {
    expect(
      validateAssistant({ format: 'juniper-assistant', version: 1, assistant: { id: 'x' } }),
    ).toBe(false)
    expect(() => parseAssistant('{"format":"juniper-assistant","version":1}')).toThrow()
  })

  it('migrates the v1 thinking flag to the v2 enum', () => {
    const legacy = JSON.parse(serializeAssistant(defaultAssistant))
    legacy.version = 1
    legacy.assistant.schemaVersion = 1
    legacy.assistant.modelProfileId = 'legacy-model'
    legacy.assistant.generation.thinking = true
    const migrated = parseAssistant(JSON.stringify(legacy))
    expect(migrated.schemaVersion).toBe(2)
    expect(migrated.generation.thinking).toBe('on')
  })

  it('rejects unknown fields, mismatched versions, invalid bounds, and invalid dates', () => {
    const valid = JSON.parse(serializeAssistant(defaultAssistant))
    expect(validateAssistant({ ...valid, unexpected: true })).toBe(false)
    expect(validateAssistant({ ...valid, version: 1 })).toBe(false)

    const oversizedPrompt = structuredClone(valid)
    oversizedPrompt.assistant.systemPrompt = 'x'.repeat(20_001)
    expect(validateAssistant(oversizedPrompt)).toBe(false)

    const unsafeGeneration = structuredClone(valid)
    unsafeGeneration.assistant.generation.maxOutput = Number.MAX_SAFE_INTEGER
    expect(validateAssistant(unsafeGeneration)).toBe(false)

    const invalidDate = structuredClone(valid)
    invalidDate.assistant.createdAt = 'eventually'
    expect(validateAssistant(invalidDate)).toBe(false)
  })

  it('rejects imports larger than the bounded portable format', () => {
    expect(() => parseAssistant(' '.repeat(MAX_ASSISTANT_IMPORT_BYTES + 1))).toThrow(
      'maximum size is 128 KiB',
    )
  })
})
