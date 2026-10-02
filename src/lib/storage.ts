import type { AppData, AttachmentRecord } from '../types'
import {
  DEFAULT_SYSTEM_PROMPT,
  initialAppData,
  isHistoricalStockJuniperSystemPrompt,
  JUNIPER_ACCENT,
} from './defaults'
import { normalizeSettings } from './settings'

const STORAGE_KEY = 'juniper.app-data.v1'

export function loadAppData(): AppData {
  if (typeof localStorage === 'undefined') return initialAppData()
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return initialAppData()
    return normalizeAppData(JSON.parse(raw))
  } catch {
    return initialAppData()
  }
}

/**
 * Drops private chats and everything scoped to them.
 *
 * Private chats are promised to the user as session-only, so this is applied to
 * every path that writes them outside memory — persistence and user export
 * alike.
 */
export function withoutPrivateChats(data: AppData): AppData {
  const privateConversationIds = new Set(
    data.conversations.filter((chat) => chat.privateChat).map((chat) => chat.id),
  )
  return {
    ...data,
    conversations: data.conversations.filter((chat) => !chat.privateChat),
    attachments: data.attachments.filter(
      (attachment) => !privateConversationIds.has(attachment.conversationId),
    ),
    permissions: data.permissions.filter(
      (grant) =>
        grant.scope !== 'chat' ||
        !grant.conversationId ||
        !privateConversationIds.has(grant.conversationId),
    ),
  }
}

export function saveAppData(data: AppData): void {
  if (typeof localStorage === 'undefined') return
  localStorage.setItem(STORAGE_KEY, JSON.stringify(withoutPrivateChats(data)))
}

/**
 * Earlier versions stored raw model reasoning as message parts. It is not
 * history and is never shown or sent again, so it is dropped on load.
 */
function withoutRawReasoning(conversation: AppData['conversations'][number]) {
  return {
    ...conversation,
    messages: (conversation.messages ?? []).map((message) => ({
      ...message,
      parts: message.parts.filter((part) => (part.type as string) !== 'reasoning'),
    })),
  }
}

function migrateBuiltinAssistantPrompt(assistant: AppData['assistants'][number]): string {
  if (
    assistant.id === 'assistant-juniper' &&
    isHistoricalStockJuniperSystemPrompt(assistant.systemPrompt)
  ) {
    return DEFAULT_SYSTEM_PROMPT
  }
  return assistant.systemPrompt
}

export function normalizeAppData(value: unknown): AppData {
  const defaults = initialAppData()
  if (!value || typeof value !== 'object') return defaults
  const parsed = value as Partial<AppData>
  const providers = (Array.isArray(parsed.providers) ? parsed.providers : defaults.providers).map(
    (provider) => ({
      ...provider,
      transportLocation:
        provider.transportLocation ?? inferTransportLocation(provider.baseUrl, provider.locality),
      capabilities: {
        ...defaults.providers[0]!.capabilities,
        ...provider.capabilities,
        chat: provider.capabilities?.chat ?? provider.capabilities?.text ?? 'unknown',
      },
    }),
  )
  const models = (Array.isArray(parsed.models) ? parsed.models : defaults.models).map((model) => {
    const provider = providers.find((item) => item.id === model.providerId)
    const executionLocation =
      model.executionLocation ??
      provider?.transportLocation ??
      inferTransportLocation('', model.locality)
    return {
      ...model,
      executionLocation,
      compatibilityStatus:
        model.compatibilityStatus ??
        (model.capabilities?.chat === 'unsupported' ? 'not-chat-compatible' : 'unknown'),
      capabilities: {
        ...defaults.providers[0]!.capabilities,
        ...model.capabilities,
        chat: model.capabilities?.chat ?? model.capabilities?.text ?? 'unknown',
      },
    }
  })
  const rawDeviceLink: Partial<AppData['deviceLink']> =
    parsed.deviceLink && typeof parsed.deviceLink === 'object' ? parsed.deviceLink : {}
  const deviceLink = {
    ...defaults.deviceLink,
    ...rawDeviceLink,
    peers: Array.isArray(rawDeviceLink.peers) ? rawDeviceLink.peers : [],
  }
  const attachments = (Array.isArray(parsed.attachments) ? parsed.attachments : []).flatMap(
    (attachment): AttachmentRecord[] => {
      if (!attachment || typeof attachment !== 'object') return []
      const item = attachment as Partial<AttachmentRecord>
      if (
        typeof item.id !== 'string' ||
        typeof item.conversationId !== 'string' ||
        typeof item.name !== 'string' ||
        typeof item.sizeBytes !== 'number' ||
        typeof item.contentType !== 'string'
      )
        return []
      return [
        {
          id: item.id,
          conversationId: item.conversationId,
          name: item.name,
          sizeBytes: item.sizeBytes,
          contentType: item.contentType,
        },
      ]
    },
  )
  return {
    ...defaults,
    assistants: (Array.isArray(parsed.assistants) ? parsed.assistants : defaults.assistants).map(
      (assistant) => ({
        ...assistant,
        schemaVersion: 2,
        systemPrompt: migrateBuiltinAssistantPrompt(assistant),
        ...(assistant.id === 'assistant-juniper' && assistant.avatar === 'J'
          ? { avatar: defaults.assistants[0]!.avatar }
          : {}),
        ...(assistant.id === 'assistant-juniper' && assistant.accent === '#6f8f72'
          ? { accent: JUNIPER_ACCENT }
          : {}),
        modelProfileId: assistant.modelProfileId ?? null,
        generation: {
          ...assistant.generation,
          thinking:
            typeof assistant.generation?.thinking === 'boolean'
              ? assistant.generation.thinking
                ? 'on'
                : 'off'
              : (assistant.generation?.thinking ?? 'auto'),
        },
      }),
    ),
    models,
    providers,
    conversations: Array.isArray(parsed.conversations)
      ? parsed.conversations.map(withoutRawReasoning)
      : defaults.conversations,
    memories: Array.isArray(parsed.memories) ? parsed.memories : defaults.memories,
    attachments,
    permissions: Array.isArray(parsed.permissions) ? parsed.permissions : defaults.permissions,
    settings: normalizeSettings(parsed.settings),
    deviceLink,
  }
}

export function inferTransportLocation(baseUrl: string | undefined, locality?: string) {
  if (locality === 'remote') return 'remote' as const
  try {
    const hostname = baseUrl ? new URL(baseUrl).hostname.toLowerCase() : ''
    const host = hostname.replace(/^\[|\]$/g, '')
    const firstIpv6Group = host.includes(':') ? Number.parseInt(host.split(':')[0] ?? '', 16) : NaN
    if (
      host === 'localhost' ||
      host.endsWith('.localhost') ||
      host === '::1' ||
      /^127\./.test(host)
    ) {
      return 'on-device' as const
    }
    if (
      host.endsWith('.local') ||
      /^(10\.|192\.168\.|169\.254\.|172\.(1[6-9]|2\d|3[01])\.)/.test(host) ||
      (firstIpv6Group >= 0xfc00 && firstIpv6Group <= 0xfdff) ||
      (firstIpv6Group >= 0xfe80 && firstIpv6Group <= 0xfebf)
    ) {
      return 'local-network' as const
    }
    if (host) return 'remote' as const
  } catch {
    // Fall through to unknown rather than guessing from a malformed endpoint.
  }
  return 'unknown' as const
}

export function clearAppData(): void {
  localStorage.removeItem(STORAGE_KEY)
}
