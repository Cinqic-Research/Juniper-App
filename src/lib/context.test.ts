import { describe, expect, it } from 'vitest'
import { defaultAssistant, DEFAULT_SYSTEM_PROMPT, builtinTools } from './defaults'
import { buildContext, compilePersonality } from './context'

describe('context builder', () => {
  it('keeps system instructions, tools, memories, and newest conversation in explicit order', () => {
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
    expect(result.system).toContain('You are Juniper')
    expect(result.tools[0]).toContain('calculator.evaluate')
    expect(result.memory).toEqual(['User likes concise answers.'])
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

    expect(result.system).toBeTruthy()
    expect(result.system).toContain('Treat an existing conversation as continuous')
    expect(result.system).toContain('this is an ongoing exchange')
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

  it('defines continuity and truthful directness in the built-in prompt', () => {
    expect(DEFAULT_SYSTEM_PROMPT).toContain('Treat an existing conversation as continuous')
    expect(DEFAULT_SYSTEM_PROMPT).toContain('do not restart')
    expect(DEFAULT_SYSTEM_PROMPT).toContain('Previous assistant messages are history')
    expect(DEFAULT_SYSTEM_PROMPT).toContain('Emoji are optional')
    expect(DEFAULT_SYSTEM_PROMPT).toContain(
      'arbitrary shell, file, network, keyboard, or code-execution access',
    )
    expect(DEFAULT_SYSTEM_PROMPT).not.toContain("Don't say Hey there")
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
    const result = buildContext(defaultAssistant, [], messages, [], 300)
    expect(result.truncated).toBe(true)
    expect(result.conversation.at(-1)?.content).toContain('message 39')
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
    expect(result.system).toContain('<juniper-memory>')
    expect(result.system).toContain('Likes short answers.')
    expect(result.system).toContain('Compiled personality controls')
    expect(result.system).toContain('this is the beginning of the exchange')
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
    expect(result.system).not.toContain('Do not leak this.')
    expect(result.memory).toEqual([])
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
