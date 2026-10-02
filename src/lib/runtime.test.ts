import { describe, expect, it } from 'vitest'
import { defaultAssistant, defaultProvider, modelProfileFromDiscovery } from './defaults'
import { browserPreviewEnabled, modelFromInspection, streamChat } from './runtime'

describe('deterministic fake provider', () => {
  it('streams a truthful Juniper identity response without a provider', async () => {
    expect(browserPreviewEnabled).toBe(true)
    const events: string[] = []
    await streamChat(
      {
        requestId: 'test',
        assistantId: defaultAssistant.id,
        assistantName: defaultAssistant.name,
        conversationId: 'conversation-test',
        privateChat: false,
        provider: defaultProvider,
        model: modelProfileFromDiscovery(defaultProvider, 'preview-unknown-model:1b'),
        messages: [{ role: 'user', content: 'Who are you?' }],
        tools: [],
        generation: defaultAssistant.generation,
        permissionGrants: [],
        hostContext: { memories: [], conversations: [] },
        contextMemoryIds: [],
      },
      (event) => {
        if (event.delta) events.push(event.delta)
      },
      new AbortController().signal,
    )
    expect(events.join('')).toContain('I’m Juniper')
  })

  it('does not qualify a discovered model without a chat capability', () => {
    const model = modelFromInspection(
      { ...defaultProvider, transportLocation: 'local-network' },
      {
        modelId: 'embedding-model',
        displayName: 'Embedding model',
        capabilities: ['embedding'],
        metadataSource: 'test-fixture',
      },
    )
    expect(model.compatibilityStatus).toBe('not-chat-compatible')
    expect(model.executionLocation).toBe('local-network')
  })
})
