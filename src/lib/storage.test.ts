import { beforeEach, describe, expect, it } from 'vitest'
import {
  DEFAULT_SYSTEM_PROMPT,
  HISTORICAL_STOCK_JUNIPER_SYSTEM_PROMPTS,
  initialAppData,
} from './defaults'
import { rc32StoredState } from '../test/fixtures'
import {
  inferTransportLocation,
  loadAppData,
  normalizeAppData,
  saveAppData,
  withoutPrivateChats,
} from './storage'

describe('browser-preview storage', () => {
  beforeEach(() => localStorage.clear())

  it('migrates exact historical built-in prompts and preserves customized assistants', () => {
    const stored = rc32StoredState()
    const rc32Assistant = stored.assistants[0]!
    const currentMainStockAssistant = {
      ...rc32Assistant,
      systemPrompt: HISTORICAL_STOCK_JUNIPER_SYSTEM_PROMPTS[1],
      updatedAt: '2026-09-18T09:00:00.000Z',
    }
    const customizedBuiltin = {
      ...rc32Assistant,
      systemPrompt: `${HISTORICAL_STOCK_JUNIPER_SYSTEM_PROMPTS[1]}\nKeep answers concise.`,
      welcomeMessage: 'Use my own opening.',
    }
    const customAssistantWithStockText = {
      ...stored.assistants[1]!,
      systemPrompt: HISTORICAL_STOCK_JUNIPER_SYSTEM_PROMPTS[0],
    }

    const normalized = normalizeAppData({
      ...stored,
      assistants: [
        rc32Assistant,
        currentMainStockAssistant,
        customizedBuiltin,
        customAssistantWithStockText,
      ],
    })

    expect(normalized.assistants[0]!.systemPrompt).toBe(DEFAULT_SYSTEM_PROMPT)
    expect(normalized.assistants[1]!.systemPrompt).toBe(DEFAULT_SYSTEM_PROMPT)
    expect(normalized.assistants[1]!.updatedAt).toBe(currentMainStockAssistant.updatedAt)
    expect(normalized.assistants[2]!.systemPrompt).toBe(customizedBuiltin.systemPrompt)
    expect(normalized.assistants[2]!.welcomeMessage).toBe(customizedBuiltin.welcomeMessage)
    expect(normalized.assistants[3]!.systemPrompt).toBe(customAssistantWithStockText.systemPrompt)
    expect(normalized.assistants[0]!.modelProfileId).toBe(rc32Assistant.modelProfileId)
    expect(normalized.assistants[0]!.suggestedPrompts).toEqual(rc32Assistant.suggestedPrompts)

    saveAppData(normalized)
    expect(loadAppData().assistants.map((assistant) => assistant.systemPrompt)).toEqual(
      normalized.assistants.map((assistant) => assistant.systemPrompt),
    )
  })
  it('does not persist private chats', () => {
    const data = initialAppData()
    data.conversations = [
      {
        id: 'private',
        title: 'Private',
        assistantId: data.assistants[0]!.id,
        createdAt: '',
        updatedAt: '',
        privateChat: true,
        messages: [],
      },
    ]
    saveAppData(data)
    expect(loadAppData().conversations.length).toBe(0)
  })

  it('does not persist chat-scoped grants for private chats', () => {
    const data = initialAppData()
    data.conversations = [
      {
        id: 'private',
        title: 'Private',
        assistantId: data.assistants[0]!.id,
        createdAt: '',
        updatedAt: '',
        privateChat: true,
        messages: [],
      },
    ]
    data.permissions = [
      {
        id: 'grant-private',
        toolName: 'file.read',
        scope: 'chat',
        assistantId: data.assistants[0]!.id,
        conversationId: 'private',
        createdAt: '',
        updatedAt: '',
      },
    ]
    saveAppData(data)
    expect(loadAppData().permissions).toHaveLength(0)
  })

  it('does not persist attachment metadata for private chats', () => {
    const data = initialAppData()
    data.conversations = [
      {
        id: 'private',
        title: 'Private',
        assistantId: data.assistants[0]!.id,
        createdAt: '',
        updatedAt: '',
        privateChat: true,
        messages: [],
      },
      {
        id: 'saved',
        title: 'Saved',
        assistantId: data.assistants[0]!.id,
        createdAt: '',
        updatedAt: '',
        messages: [],
      },
    ]
    data.attachments = [
      {
        id: 'private-file',
        conversationId: 'private',
        name: 'private.txt',
        sizeBytes: 12,
        contentType: 'text/plain',
      },
      {
        id: 'saved-file',
        conversationId: 'saved',
        name: 'saved.txt',
        sizeBytes: 10,
        contentType: 'text/plain',
      },
    ]
    saveAppData(data)
    expect(loadAppData().attachments.map((item) => item.id)).toEqual(['saved-file'])
  })

  it('excludes private chats and their scoped records from user exports', () => {
    const data = initialAppData()
    data.conversations = [
      {
        id: 'private',
        title: 'Private',
        assistantId: data.assistants[0]!.id,
        createdAt: '',
        updatedAt: '',
        privateChat: true,
        messages: [],
      },
      {
        id: 'saved',
        title: 'Saved',
        assistantId: data.assistants[0]!.id,
        createdAt: '',
        updatedAt: '',
        messages: [],
      },
    ]
    data.attachments = [
      {
        id: 'private-file',
        conversationId: 'private',
        name: 'private.txt',
        sizeBytes: 12,
        contentType: 'text/plain',
      },
    ]
    data.permissions = [
      {
        id: 'grant-private',
        toolName: 'file.read',
        scope: 'chat',
        assistantId: data.assistants[0]!.id,
        conversationId: 'private',
        createdAt: '',
        updatedAt: '',
      },
    ]
    const exported = withoutPrivateChats(data)
    expect(exported.conversations.map((chat) => chat.id)).toEqual(['saved'])
    expect(exported.attachments).toHaveLength(0)
    expect(exported.permissions).toHaveLength(0)
    expect(JSON.stringify(exported)).not.toContain('private.txt')
  })

  it.each([
    ['http://localhost:11434', 'on-device'],
    ['http://model.localhost:11434', 'on-device'],
    ['http://127.8.4.2:11434', 'on-device'],
    ['http://[::1]:11434', 'on-device'],
    ['http://192.168.1.2:11434', 'local-network'],
    ['http://169.254.3.4:11434', 'local-network'],
    ['http://[fd00::1234]:11434', 'local-network'],
    ['http://[fe80::1234]:11434', 'local-network'],
    ['https://api.example.test', 'remote'],
    ['not a URL', 'unknown'],
  ])('classifies provider route %s as %s', (url, expected) => {
    expect(inferTransportLocation(url)).toBe(expected)
  })
})
