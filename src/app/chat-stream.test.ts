import { describe, expect, it } from 'vitest'
import type { ChatMessage, ChatStreamEvent } from '../types'
import { activityLabel, applyStreamError, applyStreamEvent } from './ChatScreen'
import { hasUncheckedReferences } from './MessageBubble'

function reply(parts: ChatMessage['parts']): ChatMessage {
  return {
    id: 'reply',
    conversationId: 'chat',
    role: 'assistant',
    parts,
    createdAt: '2026-10-02T00:00:00.000Z',
    isStreaming: true,
  }
}

describe('chat stream handling', () => {
  it('never stores raw reasoning, even if an older host sends it', () => {
    const legacy = { requestId: 'r', reasoning: 'private plan' } as unknown as ChatStreamEvent
    const next = applyStreamEvent(reply([]), legacy)
    expect(JSON.stringify(next)).not.toContain('private plan')
    expect(next.parts).toEqual([])
  })

  it('records host provenance with the answer', () => {
    const next = applyStreamEvent(reply([{ id: 't', type: 'text', text: 'Hi.' }]), {
      requestId: 'r',
      done: true,
      provenance: {
        backend: 'gpt-oss-20b-mxfp4-flowbox.v1',
        constitution: 'juniper-constitution.v2',
        toolProtocol: 'juniper-tool-protocol-v1',
        providerKind: 'openai-compatible',
        modelId: 'gpt-oss-20b',
        executionLocation: 'on-device',
        reasoningEffort: 'medium',
      },
    })
    expect(next.provenance?.backend).toBe('gpt-oss-20b-mxfp4-flowbox.v1')
  })

  it('keeps a truncated answer visible but marked, and replaces other failed output', () => {
    const partial = reply([{ id: 't', type: 'text', text: 'The first half' }])
    const truncated = applyStreamError(partial, {
      code: 'GENERATION_TRUNCATED',
      message: 'The answer reached the output limit and is incomplete.',
    })
    expect(truncated.parts.map((part) => part.type)).toEqual(['text', 'error'])
    expect(truncated.isStreaming).toBe(false)
    const invalid = applyStreamError(
      reply([{ id: 't', type: 'text', text: '<|channel|>analysis' }]),
      { code: 'MODEL_OUTPUT_INVALID', message: 'Not accepted.' },
    )
    expect(invalid.parts.map((part) => part.type)).toEqual(['error'])
  })

  it('describes host activity without implying an answer exists', () => {
    expect(activityLabel('loading-model', 'Juniper')).toContain('Loading the model')
    expect(activityLabel('reasoning', 'Juniper')).toBe('Juniper is reasoning…')
  })

  it('flags model-written links and citations that no host result supplied', () => {
    const text = (value: string) => reply([{ id: 't', type: 'text', text: value }])
    expect(hasUncheckedReferences(text('See https://example.com/paper.'))).toBe(true)
    expect(hasUncheckedReferences(text('DOI 10.1234/abcd.5678 describes it.'))).toBe(true)
    expect(hasUncheckedReferences(text('Plain answer without references.'))).toBe(false)
    const supplied = reply([
      { id: 't', type: 'text', text: 'The file mentions https://cinqic.com.' },
      {
        id: 'r',
        type: 'tool-result',
        status: 'success',
        text: '{"content":"Visit https://cinqic.com for details"}',
      },
    ])
    expect(hasUncheckedReferences(supplied)).toBe(false)
  })
})
