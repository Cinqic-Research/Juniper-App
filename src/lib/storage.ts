import type { AppData, AttachmentRecord, ExecutionLocation, Locality } from '../types'
import {
  DEFAULT_SYSTEM_PROMPT,
  initialAppData,
  isHistoricalStockJuniperSystemPrompt,
  JUNIPER_ACCENT,
} from './defaults'
import { normalizeSettings } from './settings'

const STORAGE_KEY = 'juniper.app-data.v1'

function localityForLocation(location: ExecutionLocation): Locality {
  if (location === 'remote') return 'remote'
  if (location === 'unknown') return 'unknown'
  return 'local'
}

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
    (provider) => {
      // Saved locality is presentation metadata. Recompute it from the route
      // before showing where messages will go, including for imported data.
      const transportLocation =
        provider.kind === 'juniper-local' ? 'on-device' : inferTransportLocation(provider.baseUrl)
      return {
        ...provider,
        locality: localityForLocation(transportLocation),
        transportLocation,
        capabilities: {
          ...defaults.providers[0]!.capabilities,
          ...provider.capabilities,
          chat: provider.capabilities?.chat ?? provider.capabilities?.text ?? 'unknown',
        },
      }
    },
  )
  const models = (Array.isArray(parsed.models) ? parsed.models : defaults.models).map((model) => {
    const provider = providers.find((item) => item.id === model.providerId)
    const executionLocation = provider?.transportLocation ?? 'unknown'
    return {
      ...model,
      locality: localityForLocation(executionLocation),
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

function privateV4(a: number, b: number) {
  return (
    a === 10 ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168)
  )
}

function ipv6Groups(host: string): number[] | null {
  let value = host.toLowerCase()
  if (value.includes('.')) {
    const colon = value.lastIndexOf(':')
    const ipv4 = value.slice(colon + 1).split('.')
    if (ipv4.length !== 4 || ipv4.some((part) => !/^\d{1,3}$/.test(part))) return null
    const bytes = ipv4.map(Number)
    if (bytes.some((byte) => byte > 255)) return null
    value = `${value.slice(0, colon)}:${((bytes[0]! << 8) | bytes[1]!).toString(16)}:${((bytes[2]! << 8) | bytes[3]!).toString(16)}`
  }
  const halves = value.split('::')
  if (halves.length > 2) return null
  const left = halves[0] ? halves[0]!.split(':') : []
  const right = halves.length === 2 && halves[1] ? halves[1]!.split(':') : []
  if (halves.length === 1 && left.length !== 8) return null
  const missing = 8 - left.length - right.length
  if ((halves.length === 2 && missing < 1) || missing < 0) return null
  const groups = [...left, ...Array.from({ length: missing }, () => '0'), ...right].map((group) =>
    /^[0-9a-f]{1,4}$/.test(group) ? Number.parseInt(group, 16) : NaN,
  )
  return groups.length === 8 && groups.every(Number.isFinite) ? groups : null
}

function ipv6Route(groups: number[]): 'on-device' | 'local-network' | null {
  const mapped = groups.slice(0, 5).every((part) => part === 0) && groups[5] === 0xffff
  if (mapped) {
    const first = groups[6]! >>> 8
    const second = groups[6]! & 0xff
    if (first === 127) return 'on-device'
    return privateV4(first, second) ? 'local-network' : null
  }
  if (groups[0] === 0 && groups.slice(1).every((part) => part === 0)) return null
  if ((groups[0]! & 0xfe00) === 0xfc00) return 'local-network'
  if (groups[0]! >= 0xfe80 && groups[0]! <= 0xfebf) return 'local-network'
  return null
}

export function inferTransportLocation(baseUrl: string | undefined) {
  try {
    const hostname = baseUrl ? new URL(baseUrl).hostname.toLowerCase() : ''
    const host = hostname.replace(/^\[|\]$/g, '')
    const ipv4 = host.split('.').map(Number)
    const ipv6 = host.includes(':') ? ipv6Groups(host) : null
    if (
      host === 'localhost' ||
      host.endsWith('.localhost') ||
      host === '::1' ||
      (!host.includes(':') && ipv4.length === 4 && ipv4[0] === 127) ||
      (ipv6 !== null && ipv6Route(ipv6) === 'on-device')
    ) {
      return 'on-device' as const
    }
    if (
      host.endsWith('.local') ||
      (!host.includes(':') && ipv4.length === 4 && privateV4(ipv4[0]!, ipv4[1]!)) ||
      (ipv6 !== null && ipv6Route(ipv6) === 'local-network')
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
