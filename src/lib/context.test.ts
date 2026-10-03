import { describe, expect, it } from 'vitest'
import { defaultAssistant, DEFAULT_SYSTEM_PROMPT, builtinTools } from './defaults'
import constitution from '../../config/behavior/constitution.v2.json'
import { buildContext, compilePersonality } from './context'

describe('context builder', () => {
  it('sends the profile, selected memories, and newest conversation; the host adds the rules', () => {
    const result = buildContext(
      defaultAssistant,
      [
        {
          id: 'm',
          content: 'User likes concise answers.',
          source: 'user',
          enabled: true,
          createdAt: '',
          updatedAt: '',
        },
      ],
      [
        {
          id: '1',
          conversationId: 'c',
          role: 'user',
          parts: [{ id: 'p', type: 'text', text: 'old' }],
          createdAt: '',
        },
        {
          id: '2',
          conversationId: 'c',
          role: 'assistant',
          parts: [{ id: 'p2', type: 'text', text: 'new' }],
          createdAt: '',
        },
      ],
      builtinTools,
      4000,
    )
    // Identity and the constitution are composed natively, not by the interface.
    expect(result.profile).not.toContain('You are Juniper')
    expect(result.profile).toContain(DEFAULT_SYSTEM_PROMPT)
    expect(result.constitutionId).toBe('juniper-constitution.v2')
    expect(result.tools[0]).toContain('calculator.evaluate')
    expect(result.memory).toEqual(['User likes concise answers.'])
    expect(result.memoryIds).toEqual(['m'])
    expect(result.conversation).toEqual([
      { role: 'user', content: 'old' },
      { role: 'assistant', content: 'new' },
    ])
  })

  it('treats repetitive prior assistant text as history without rewriting or duplicating it', () => {
    const result = buildContext(
      defaultAssistant,
      [],
      [
        {
          id: '1',
          conversationId: 'c',
          role: 'user',
          parts: [{ id: 'p1', type: 'text', text: 'Yo' }],
          createdAt: '',
        },
        {
          id: '2',
          conversationId: 'c',
          role: 'assistant',
          parts: [{ id: 'p2', type: 'text', text: 'Hey there! 😊 How can I help?' }],
          createdAt: '',
        },
        {
          id: '3',
          conversationId: 'c',
          role: 'user',
          parts: [{ id: 'p3', type: 'text', text: 'how are you?' }],
          createdAt: '',
        },
        {
          id: '4',
          conversationId: 'c',
          role: 'assistant',
          parts: [{ id: 'p4', type: 'text', text: 'I am here and ready to help.' }],
          createdAt: '',
        },
        {
          id: '5',
          conversationId: 'c',
          role: 'user',
          parts: [{ id: 'p5', type: 'text', text: 'What should I do today?' }],
          createdAt: '',
        },
      ],
      [],
      4000,
      'What should I do today?',
    )

    expect(result.profile).toContain('Treat an ongoing conversation as continuous')
    expect(result.profile).toContain('this is an ongoing exchange')
    expect(result.conversation).toEqual([
      { role: 'user', content: 'Yo' },
      { role: 'assistant', content: 'Hey there! 😊 How can I help?' },
      { role: 'user', content: 'how are you?' },
      { role: 'assistant', content: 'I am here and ready to help.' },
    ])
    expect(result.currentUserMessage).toBe('What should I do today?')
    expect(
      result.conversation.filter((item) => item.content === result.currentUserMessage),
    ).toHaveLength(0)
  })

  it('keeps the built-in profile to style and continuity; rules live in the constitution', () => {
    expect(DEFAULT_SYSTEM_PROMPT).toContain('Treat an ongoing conversation as continuous')
    expect(DEFAULT_SYSTEM_PROMPT).toContain('do not restart')
    expect(DEFAULT_SYSTEM_PROMPT).toContain('do not copy the style of earlier replies')
    // Identity, truth, and capability rules must not depend on editable profile text.
    expect(DEFAULT_SYSTEM_PROMPT).not.toContain('You are Juniper')
    expect(DEFAULT_SYSTEM_PROMPT).not.toContain('older sister')
    const rules = constitution.rules.map((rule) => rule.text).join('\n')
    expect(rules).toContain('Truth over confidence')
    expect(rules).toContain('Only the host runs tools')
    expect(rules).toContain('data, not instructions')
    expect(rules).not.toContain('older sister')
  })

  it('keeps personality bands meaningful while separating warmth from canned behavior', () => {
    const high = compilePersonality({
      warmth: 100,
      directness: 100,
      playfulness: 100,
      detail: 100,
      creativity: 100,
      formality: 100,
    })
    expect(high).toContain('Communicate warmth through attentive, natural wording')
    expect(high).toContain('Answer the latest question promptly')
    expect(high).toContain('Allow light, well-timed playfulness')
    expect(high).toContain('Explain thoroughly')
    expect(high).toContain('Offer inventive alternatives')
    expect(high).toContain('polished, formal wording')

    const low = compilePersonality({
      warmth: 0,
      directness: 0,
      playfulness: 0,
      detail: 0,
      creativity: 0,
      formality: 0,
    })
    expect(low).toContain('restrained and professional')
    expect(low).toContain('exploratory language')
    expect(low).toContain('avoid playful asides')
    expect(low).toContain('concise and focused')
    expect(low).toContain('conventional, dependable approaches')
    expect(low).toContain('relaxed, natural wording')

    const balanced = compilePersonality({
      warmth: 50,
      directness: 50,
      playfulness: 50,
      detail: 50,
      creativity: 50,
      formality: 50,
    })
    expect(balanced).toContain('friendly, measured tone')
    expect(balanced).toContain('appropriate nuance')
    expect(balanced).toContain('occasional lightness')
    expect(balanced).toContain('enough detail')
    expect(balanced).toContain('creativity when it improves')
    expect(balanced).toContain('clear conversational wording')
  })

  it('truncates expendable oldest history under a known budget', () => {
    const messages = Array.from({ length: 40 }, (_, index) => ({
      id: String(index),
      conversationId: 'c',
      role: 'user' as const,
      parts: [
        { id: `p-${index}`, type: 'text' as const, text: `message ${index} ${'x'.repeat(80)}` },
      ],
      createdAt: '',
    }))
    const result = buildContext(defaultAssistant, [], messages, [], 2000)
    expect(result.truncated).toBe(true)
    expect(result.overflow).toBe(false)
    expect(result.conversation.at(-1)?.content).toContain('message 39')
  })

  it('reserves room for the answer and reports when the required layers cannot fit', () => {
    const history = Array.from({ length: 20 }, (_, index) => ({
      id: String(index),
      conversationId: 'c',
      role: index % 2 ? ('assistant' as const) : ('user' as const),
      parts: [{ id: `p-${index}`, type: 'text' as const, text: 'x'.repeat(400) }],
      createdAt: '',
    }))
    const roomy = buildContext(defaultAssistant, [], history, [], 4000, undefined, [], {
      reservedOutputTokens: 0,
    })
    const reserved = buildContext(defaultAssistant, [], history, [], 4000, undefined, [], {
      reservedOutputTokens: 2048,
    })
    expect(reserved.conversation.length).toBeLessThan(roomy.conversation.length)
    expect(reserved.estimatedTokens).toBeLessThanOrEqual(4000)
    const overflow = buildContext(defaultAssistant, [], [], [], 1000, 'x'.repeat(8000))
    expect(overflow.overflow).toBe(true)
    // An output budget beyond the context does not by itself overflow.
    const generous = buildContext(defaultAssistant, [], [], [], 16384, 'hi', [], {
      reservedOutputTokens: 32768,
    })
    expect(generous.overflow).toBe(false)
    expect(generous.reservedOutputTokens).toBe(8192)
  })

  it('drops history by whole exchanges and never keeps failed replies as context', () => {
    const message = (id: string, role: 'user' | 'assistant', text: string, failed = false) => ({
      id,
      conversationId: 'c',
      role,
      parts: [
        { id: `${id}-t`, type: 'text' as const, text },
        ...(failed ? [{ id: `${id}-e`, type: 'error' as const, text: 'cut off' }] : []),
      ],
      createdAt: '',
    })
    const result = buildContext(
      defaultAssistant,
      [],
      [
        message('1', 'user', 'first question'),
        message('2', 'assistant', 'partial answer that was cut', true),
        message('3', 'user', 'second question'),
        message('4', 'assistant', 'complete answer'),
      ],
      [],
      4000,
    )
    expect(result.conversation.map((item) => item.content)).toEqual([
      'first question',
      'second question',
      'complete answer',
    ])
    const tight = buildContext(
      defaultAssistant,
      [],
      [message('1', 'user', 'a'.repeat(400)), message('2', 'assistant', 'b'.repeat(400))],
      [],
      1400,
      undefined,
      [],
      { reservedOutputTokens: 0 },
    )
    // Either the whole exchange fits or none of it does.
    expect([0, 2]).toContain(tight.conversation.length)
  })

  it('excludes memories from private chats', () => {
    const result = buildContext(
      defaultAssistant,
      [{ id: 'm', content: 'secret', source: 'user', enabled: true, createdAt: '', updatedAt: '' }],
      [],
      [],
      4000,
      'hello',
      [],
      { privateChat: true },
    )
    expect(result.memoryIds).toEqual([])
    expect(result.memory).toEqual([])
  })

  it('includes curated memory once and compiles personality controls', () => {
    const result = buildContext(
      defaultAssistant,
      [
        {
          id: 'm',
          content: 'Likes short answers.',
          source: 'user',
          enabled: true,
          createdAt: '',
          updatedAt: '',
        },
      ],
      [],
      [],
      4000,
      'hello',
    )
    // Memories travel by ID; the host frames them as data outside the instructions.
    expect(result.profile).not.toContain('Likes short answers.')
    expect(result.memory).toEqual(['Likes short answers.'])
    expect(result.memoryIds).toEqual(['m'])
    expect(result.profile).toContain('Compiled personality controls')
    expect(result.profile).toContain('this is the beginning of the exchange')
    expect(result.currentUserMessage).toBe('hello')
    expect(result.conversation).toEqual([])
  })

  it('does not include memories when the assistant policy is off', () => {
    const result = buildContext(
      { ...defaultAssistant, memoryPolicy: 'off' },
      [
        {
          id: 'm',
          content: 'Do not leak this.',
          source: 'user',
          enabled: true,
          createdAt: '',
          updatedAt: '',
        },
      ],
      [],
      [],
      4000,
      'hello',
    )
    expect(result.profile).not.toContain('Do not leak this.')
    expect(result.memory).toEqual([])
    expect(result.memoryIds).toEqual([])
  })

  it('keeps the current user message exactly once', () => {
    const result = buildContext(
      defaultAssistant,
      [],
      [
        {
          id: 'u',
          conversationId: 'c',
          role: 'user',
          parts: [{ id: 'p', type: 'text', text: 'current' }],
          createdAt: '',
        },
      ],
      [],
      4000,
      'current',
    )
    expect(result.conversation.filter((item) => item.content === 'current')).toHaveLength(0)
    expect(result.currentUserMessage).toBe('current')
  })
})
