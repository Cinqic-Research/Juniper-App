import { useEffect, useMemo, useRef, useState } from 'react'
import type { RefObject } from 'react'
import { buildContext, type ContextSummary } from '../lib/context'
import { builtinTools } from '../lib/defaults'
import {
  cancelChat,
  pickAttachment,
  readAttachment,
  resolvePermission,
  runningInTauri,
  runningOnAndroid,
  streamChat,
} from '../lib/runtime'
import type {
  AppData,
  Assistant,
  AttachmentRecord,
  ChatMessage,
  ChatStreamEvent,
  Conversation,
  HostToolResult,
  MessagePart,
  ModelProfile,
  PermissionDecision,
  PermissionRequest,
  RuntimeActivity,
} from '../types'
import { AssistantAvatar } from './branding'
import { Composer, isSupportedBrowserAttachmentName, type StagedAttachment } from './Composer'
import { Icon } from './icons'
import { MessageBubble, textPart } from './MessageBubble'
import { assistantFor, defaultAssistantFor, isChatSelectable, resolveRoute } from './model-labels'
import { Menu, Modal, useDialogs } from './overlays'
import { LocationBadge, download, labelExecutionLocation, now, uid } from './ui'

type Update = (change: (current: AppData) => AppData) => void

const MAX_ATTACHMENT_BYTES = 1024 * 1024

/** A readable title from the first message, cut at a word boundary. */
export function chatTitle(content: string): string {
  const text = content
    .replace(/\[Attached: [^\]]+\]/g, '')
    .replace(/\s+/g, ' ')
    .trim()
  if (!text) return content.trim() ? 'Attached files' : 'New conversation'
  if (text.length <= 48) return text
  const cut = text.slice(0, 48)
  const space = cut.lastIndexOf(' ')
  return `${(space > 24 ? cut.slice(0, space) : cut).replace(/[\s,.;:!?-]+$/, '')}…`
}

function errorCodeFromMessage(message: string): string | undefined {
  return message.match(/^[A-Z][A-Z0-9_]+:/)?.[0].slice(0, -1)
}

export function applyStreamEvent(message: ChatMessage, event: ChatStreamEvent): ChatMessage {
  const parts = [...message.parts]
  if (event.delta) {
    const textIndex = parts.findIndex((part) => part.type === 'text')
    const text: MessagePart = {
      id: textIndex >= 0 ? parts[textIndex]!.id : uid('part'),
      type: 'text',
      text: `${textIndex >= 0 ? (parts[textIndex]!.text ?? '') : ''}${event.delta}`,
    }
    if (textIndex >= 0) parts[textIndex] = text
    else parts.unshift(text)
  }
  for (const call of event.toolCalls ?? []) {
    const existingIndex = parts.findIndex(
      (part) => part.type === 'tool-call' && part.metadata?.callId === call.id,
    )
    if (existingIndex >= 0) {
      const existing = parts[existingIndex]!
      parts[existingIndex] = {
        ...existing,
        metadata: { ...existing.metadata, arguments: JSON.stringify(call.arguments) },
      }
      continue
    }
    parts.push({
      id: uid('part'),
      type: 'tool-call',
      name: call.name,
      text: `Requested ${call.name}`,
      status: 'unavailable',
      metadata: { callId: call.id, arguments: JSON.stringify(call.arguments) },
    })
  }
  for (const result of event.toolResults ?? []) {
    if (
      parts.some((part) => part.type === 'tool-result' && part.metadata?.callId === result.callId)
    )
      continue
    parts.push({
      id: uid('part'),
      type: 'tool-result',
      name: result.name,
      text: result.error?.message ?? JSON.stringify(result.result ?? {}),
      status: result.status,
      metadata: { callId: result.callId },
    })
  }
  return {
    ...message,
    parts,
    usage: event.usage ? { ...message.usage, ...event.usage } : message.usage,
    ...(event.provenance ? { provenance: event.provenance } : {}),
  }
}

/** Status text for host activity before answer text arrives. */
export function activityLabel(activity: RuntimeActivity, assistantName: string): string {
  switch (activity) {
    case 'loading-model':
      return 'Loading the model into memory. A large model can take a few minutes the first time.'
    case 'warming-up':
      return 'Warming up the model…'
    case 'restarting-model':
      return 'The local model stopped. Restarting it…'
    case 'reasoning':
      return `${assistantName} is reasoning…`
  }
}

/**
 * A truncated answer stays visible but marked incomplete; any other failure
 * replaces partial output, which may be malformed.
 */
export function applyStreamError(
  message: ChatMessage,
  error: { code: string; message: string },
): ChatMessage {
  const errorPart: MessagePart = {
    id: uid('part'),
    type: 'error',
    text: error.message,
    metadata: error.code ? { errorCode: error.code } : undefined,
  }
  const keep =
    error.code === 'GENERATION_TRUNCATED'
      ? message.parts.filter((part) => part.type === 'text' && part.text)
      : []
  return { ...message, isStreaming: false, parts: [...keep, errorPart] }
}

function applyHostToolResult(data: AppData, result: HostToolResult): AppData {
  if (result.status !== 'success' || !result.result) return data
  const payload = result.result
  if (result.name === 'memory.save' && payload.memory && typeof payload.memory === 'object') {
    const memory = payload.memory as AppData['memories'][number]
    if (!memory.id || typeof memory.content !== 'string') return data
    return {
      ...data,
      memories: [...data.memories.filter((item) => item.id !== memory.id), memory],
    }
  }
  if (result.name === 'memory.delete' && typeof payload.deletedId === 'string') {
    return {
      ...data,
      memories: data.memories.filter((memory) => memory.id !== payload.deletedId),
    }
  }
  return data
}

// ---------------------------------------------------------------------------
// Chat history
// ---------------------------------------------------------------------------

function dayGroup(value: string, today: Date): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return 'Older'
  const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime()
  const day = 24 * 60 * 60 * 1000
  if (date.getTime() >= startOfToday) return 'Today'
  if (date.getTime() >= startOfToday - day) return 'Yesterday'
  if (date.getTime() >= startOfToday - 7 * day) return 'Previous 7 days'
  return 'Older'
}

export function ChatHistory({
  data,
  selectedChatId,
  onSelect,
  searchRef,
}: {
  data: AppData
  selectedChatId: string | null
  onSelect: (id: string) => void
  searchRef?: RefObject<HTMLInputElement | null>
}) {
  const [query, setQuery] = useState('')
  const groups = useMemo(() => {
    const normalized = query.trim().toLowerCase()
    const matches = data.conversations.filter(
      (chat) =>
        !normalized ||
        [chat.title, ...chat.messages.map(textPart)].join(' ').toLowerCase().includes(normalized),
    )
    const sorted = matches
      .map((chat, index) => ({ chat, index, time: new Date(chat.updatedAt).getTime() }))
      .sort((a, b) => {
        const aTime = Number.isNaN(a.time) ? -Infinity : a.time
        const bTime = Number.isNaN(b.time) ? -Infinity : b.time
        return bTime === aTime ? a.index - b.index : bTime - aTime
      })
    const today = new Date()
    const result: Array<{ label: string; chats: Conversation[] }> = []
    for (const { chat } of sorted) {
      const label = dayGroup(chat.updatedAt, today)
      const group = result.find((item) => item.label === label)
      if (group) group.chats.push(chat)
      else result.push({ label, chats: [chat] })
    }
    return result
  }, [data.conversations, query])

  return (
    <div className="chat-history">
      <label className="search-field">
        <Icon name="search" size={18} />
        <input
          ref={searchRef}
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search chats"
          aria-label="Search chats"
        />
      </label>
      {groups.length === 0 ? (
        <p className="history-empty">
          {query ? 'No chats match that search.' : 'Your chats will appear here.'}
        </p>
      ) : (
        groups.map((group) => (
          <div className="history-group" key={group.label}>
            <h3>{group.label}</h3>
            <ul>
              {group.chats.map((chat) => {
                const assistant = assistantFor(data, chat.assistantId)
                return (
                  <li key={chat.id}>
                    <button
                      className={`history-item ${selectedChatId === chat.id ? 'selected' : ''}`}
                      aria-current={selectedChatId === chat.id ? 'page' : undefined}
                      onClick={() => onSelect(chat.id)}
                    >
                      <AssistantAvatar assistant={assistant} className="history-avatar" />
                      <span className="history-title">{chat.title}</span>
                      {chat.privateChat && (
                        <span className="history-private" title="Private chat">
                          <Icon name="lock" size={14} />
                          <span className="visually-hidden">Private chat</span>
                        </span>
                      )}
                    </button>
                  </li>
                )
              })}
            </ul>
          </div>
        ))
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Model picker
// ---------------------------------------------------------------------------

function ModelPicker({
  open,
  onClose,
  data,
  assistant,
  selectedModelId,
  unavailableModel,
  onSelect,
  onBrowse,
  sheet,
}: {
  open: boolean
  onClose: () => void
  data: AppData
  assistant: Assistant
  selectedModelId: string | null
  unavailableModel?: ModelProfile
  onSelect: (id: string | null) => void
  onBrowse: () => void
  sheet: boolean
}) {
  const assistantModel = data.models.find(
    (model) => model.id === assistant.modelProfileId && isChatSelectable(model),
  )
  const selectable = data.models.filter(isChatSelectable)
  const providerName = (model: ModelProfile) =>
    data.providers.find((provider) => provider.id === model.providerId)?.name ?? 'Unknown provider'
  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Choose a model"
      variant={sheet ? 'sheet' : 'dialog'}
      footer={
        <button className="button ghost" onClick={onBrowse}>
          <Icon name="models" size={18} />
          Browse and manage models
        </button>
      }
    >
      <fieldset className="model-options">
        <legend className="visually-hidden">Model for this chat</legend>
        <label className={`model-option ${selectedModelId === null ? 'selected' : ''}`}>
          <input
            type="radio"
            name="conversation-model"
            checked={selectedModelId === null}
            data-autofocus={selectedModelId === null ? true : undefined}
            onChange={() => onSelect(null)}
          />
          <span className="model-option-text">
            <strong>{assistant.name}’s default</strong>
            <small>
              {assistantModel
                ? `${assistantModel.displayName} · ${labelExecutionLocation(assistantModel.executionLocation)}`
                : 'No default model is set for this assistant'}
            </small>
          </span>
        </label>
        {unavailableModel && (
          <label className="model-option unavailable">
            <input type="radio" name="conversation-model" checked disabled readOnly />
            <span className="model-option-text">
              <strong>{unavailableModel.displayName}</strong>
              <small>Unavailable — choose another model</small>
            </span>
          </label>
        )}
        {selectable.map((model) => (
          <label
            key={model.id}
            className={`model-option ${selectedModelId === model.id ? 'selected' : ''}`}
          >
            <input
              type="radio"
              name="conversation-model"
              checked={selectedModelId === model.id}
              data-autofocus={selectedModelId === model.id ? true : undefined}
              onChange={() => onSelect(model.id)}
            />
            <span className="model-option-text">
              <strong>{model.displayName}</strong>
              <small>{providerName(model)}</small>
            </span>
            <LocationBadge location={model.executionLocation} />
          </label>
        ))}
      </fieldset>
      {selectable.length === 0 && (
        <p className="muted">No models are ready yet. Download one in Models to start chatting.</p>
      )}
    </Modal>
  )
}

// ---------------------------------------------------------------------------
// Chat screen
// ---------------------------------------------------------------------------

export interface NewChatState {
  /** `null` follows the default assistant for new chats. */
  assistantId: string | null
  privateChat: boolean
  modelProfileId: string | null
}

export function ChatScreen({
  data,
  update,
  conversation,
  newChat,
  setNewChat,
  onMaterialize,
  onDeleted,
  onOpenHistory,
  onBrowseModels,
  mobile,
  touchPrimary,
  onComposerFocus,
}: {
  data: AppData
  update: Update
  /** The open chat, or undefined for a new, unsaved chat. */
  conversation: Conversation | undefined
  newChat: NewChatState
  setNewChat: (next: NewChatState) => void
  onMaterialize: (id: string) => void
  onDeleted: () => void
  onOpenHistory?: () => void
  onBrowseModels: () => void
  mobile: boolean
  touchPrimary: boolean
  onComposerFocus: (focused: boolean) => void
}) {
  const dialogs = useDialogs()
  const fallbackAssistant = newChat.assistantId
    ? assistantFor(data, newChat.assistantId)
    : defaultAssistantFor(data)
  const route = resolveRoute(data, conversation, fallbackAssistant, newChat.modelProfileId)
  const { assistant, model, provider, modelUnavailable } = route
  const privateMode = conversation ? conversation.privateChat === true : newChat.privateChat
  const messages = useMemo(() => conversation?.messages ?? [], [conversation])

  const [draft, setDraft] = useState('')
  const [attachments, setAttachments] = useState<StagedAttachment[]>([])
  const [isGenerating, setIsGenerating] = useState(false)
  const [phase, setPhase] = useState<string | null>(null)
  const [permissionRequest, setPermissionRequest] = useState<PermissionRequest | null>(null)
  const [lastContext, setLastContext] = useState<ContextSummary | null>(null)
  const [pickerOpen, setPickerOpen] = useState(false)
  const [detailsOpen, setDetailsOpen] = useState(false)
  const [inspectorOpen, setInspectorOpen] = useState(false)
  const [announcement, setAnnouncement] = useState('')
  const controller = useRef<AbortController | null>(null)
  const attachmentPickerBusy = useRef(false)
  const requestId = useRef<string | null>(null)
  const composer = useRef<HTMLTextAreaElement>(null)
  const scroller = useRef<HTMLDivElement>(null)
  const pinnedToBottom = useRef(true)
  const conversationId = useRef<string | null>(conversation?.id ?? null)

  useEffect(() => {
    // A fresh chat on a keyboard device is ready to type into.
    if (!conversation && !touchPrimary) composer.current?.focus({ preventScroll: true })
    // Only on mount: the view is remounted for every chat.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const element = scroller.current
    if (element && pinnedToBottom.current) element.scrollTop = element.scrollHeight
  }, [messages])

  function updateConversation(id: string, change: (chat: Conversation) => Conversation) {
    update((current) => ({
      ...current,
      conversations: current.conversations.map((chat) => (chat.id === id ? change(chat) : chat)),
    }))
  }

  function recordAttachment(chatId: string, attachment: StagedAttachment) {
    const metadata: AttachmentRecord = {
      id: attachment.id,
      conversationId: chatId,
      name: attachment.name,
      sizeBytes: attachment.sizeBytes ?? attachment.content.length,
      contentType: attachment.contentType ?? 'text/plain',
    }
    update((current) => ({
      ...current,
      attachments: [...current.attachments.filter((item) => item.id !== metadata.id), metadata],
    }))
  }

  async function runGeneration({
    content,
    history,
    userMessage,
    requestAttachments,
  }: {
    content: string
    history: ChatMessage[]
    userMessage: ChatMessage
    requestAttachments: StagedAttachment[]
  }) {
    if (!model || !provider) return
    let chatId = conversationId.current
    const assistantMessage: ChatMessage = {
      id: uid('message'),
      conversationId: chatId ?? '',
      role: 'assistant',
      parts: [{ id: uid('part'), type: 'text', text: '' }],
      createdAt: now(),
      modelId: model.modelId,
      providerId: provider.id,
      isStreaming: true,
    }
    const title = chatTitle(content)
    if (!chatId) {
      chatId = uid('chat')
      conversationId.current = chatId
      const created: Conversation = {
        id: chatId,
        title,
        assistantId: assistant.id,
        createdAt: now(),
        updatedAt: now(),
        privateChat: newChat.privateChat,
        ...(newChat.modelProfileId ? { modelProfileId: newChat.modelProfileId } : {}),
        messages: [],
      }
      update((current) => ({ ...current, conversations: [created, ...current.conversations] }))
      onMaterialize(chatId)
    }
    const id = chatId
    const pendingReply = { ...assistantMessage, conversationId: id }
    for (const attachment of requestAttachments) recordAttachment(id, attachment)
    const nextMessages = [...history, { ...userMessage, conversationId: id }, pendingReply]
    // The native host applies the same private-chat isolation; filtering here
    // keeps the context estimate and inspector honest.
    const enabledTools =
      assistant.toolPolicy !== 'disabled' && model.capabilities.tools === 'supported'
        ? builtinTools.filter(
            (tool) =>
              tool.enabled &&
              (tool.risk === 'automatic-safe' || assistant.toolPolicy === 'ask') &&
              !(privateMode && (tool.name.startsWith('memory.') || tool.name === 'chat.search')),
          )
        : []
    const context = buildContext(
      assistant,
      data.memories,
      nextMessages,
      enabledTools,
      model.contextLength,
      content,
      requestAttachments,
      { privateChat: privateMode, reservedOutputTokens: assistant.generation.maxOutput },
    )
    setLastContext(context)
    if (context.overflow) {
      updateConversation(id, (chat) => ({
        ...chat,
        updatedAt: now(),
        messages: [
          ...history,
          { ...userMessage, conversationId: id },
          applyStreamError(pendingReply, {
            code: 'CONTEXT_OVERFLOW',
            message: `This message and its attachments do not fit in ${model.displayName}'s ${context.contextLimit.toLocaleString()}-token context with room for an answer. Shorten it or attach less.`,
          }),
        ],
      }))
      setAnnouncement('The response did not complete.')
      return
    }
    setIsGenerating(true)
    setAnnouncement('')
    setPhase(
      provider.kind === 'juniper-local'
        ? 'Preparing the local model…'
        : `${assistant.name} is thinking…`,
    )
    pinnedToBottom.current = true
    controller.current = new AbortController()
    updateConversation(id, (chat) => ({
      ...chat,
      title: chat.title === 'New conversation' ? title : chat.title,
      updatedAt: now(),
      messages: nextMessages,
    }))
    const currentRequestId = uid('request')
    requestId.current = currentRequestId
    let failed = false
    try {
      await streamChat(
        {
          requestId: currentRequestId,
          assistantId: assistant.id,
          assistantName: assistant.name,
          conversationId: id,
          privateChat: privateMode,
          provider,
          model,
          messages: [
            { role: 'system', content: context.profile },
            ...context.conversation,
            { role: 'user', content: context.currentUserMessage },
          ],
          tools: enabledTools,
          generation: assistant.generation,
          permissionGrants: data.permissions.filter(
            (grant) =>
              grant.assistantId === assistant.id &&
              (grant.scope === 'assistant' || grant.conversationId === id),
          ),
          hostContext: {
            memories: data.memories,
            conversations: data.conversations.filter((chat) => !chat.privateChat),
          },
          attachments: requestAttachments,
          contextMemoryIds: context.memoryIds,
        },
        (streamEvent) => {
          if (streamEvent.delta) setPhase(null)
          else if (streamEvent.activity)
            setPhase(activityLabel(streamEvent.activity, assistant.name))
          if (streamEvent.permissionRequest) setPermissionRequest(streamEvent.permissionRequest)
          if (
            streamEvent.delta ||
            streamEvent.provenance ||
            streamEvent.toolCalls?.length ||
            streamEvent.toolResults?.length
          )
            updateConversation(id, (chat) => ({
              ...chat,
              messages: chat.messages.map((message) =>
                message.id === assistantMessage.id
                  ? applyStreamEvent(message, streamEvent)
                  : message,
              ),
            }))
          for (const result of streamEvent.toolResults ?? []) {
            update((current) => applyHostToolResult(current, result))
          }
          const streamError = streamEvent.error
          if (streamError) {
            failed = true
            updateConversation(id, (chat) => ({
              ...chat,
              messages: chat.messages.map((message) =>
                message.id === assistantMessage.id
                  ? applyStreamError(message, streamError)
                  : message,
              ),
            }))
          }
          if (streamEvent.done)
            updateConversation(id, (chat) => ({
              ...chat,
              messages: chat.messages.map((message) =>
                message.id === assistantMessage.id ? { ...message, isStreaming: false } : message,
              ),
            }))
        },
        controller.current.signal,
      )
    } catch (error) {
      failed = true
      const message =
        error instanceof DOMException && error.name === 'AbortError'
          ? 'Generation cancelled.'
          : error instanceof Error
            ? error.message
            : 'The provider stopped responding.'
      updateConversation(id, (chat) => ({
        ...chat,
        messages: chat.messages.map((item) =>
          item.id === assistantMessage.id
            ? {
                ...item,
                isStreaming: false,
                parts: [
                  {
                    id: uid('part'),
                    type: 'error',
                    text: message,
                    metadata: errorCodeFromMessage(message)
                      ? { errorCode: errorCodeFromMessage(message)! }
                      : undefined,
                  },
                ],
              }
            : item,
        ),
      }))
    } finally {
      setIsGenerating(false)
      setPhase(null)
      setPermissionRequest(null)
      setAnnouncement(failed ? 'The response did not complete.' : `${assistant.name} replied.`)
      controller.current = null
      requestId.current = null
      if (!touchPrimary) composer.current?.focus()
    }
  }

  function send() {
    const typed = draft.trim()
    if ((!typed && attachments.length === 0) || isGenerating || !model || !provider) return
    const content = [typed, ...attachments.map((item) => `[Attached: ${item.name}]`)]
      .filter(Boolean)
      .join('\n\n')
    const userMessage: ChatMessage = {
      id: uid('message'),
      conversationId: conversationId.current ?? '',
      role: 'user',
      parts: [{ id: uid('part'), type: 'text', text: content }],
      createdAt: now(),
    }
    const staged = attachments
    setDraft('')
    setAttachments([])
    void runGeneration({ content, history: messages, userMessage, requestAttachments: staged })
  }

  function regenerate() {
    if (isGenerating) return
    let lastUserIndex = -1
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (messages[index]!.role === 'user') {
        lastUserIndex = index
        break
      }
    }
    if (lastUserIndex < 0) return
    const lastUser = messages[lastUserIndex]!
    const content = textPart(lastUser)
    if (/\[Attached: [^\]]+\]/.test(content)) {
      // Attachment contents are never stored, so a faithful retry needs the files again.
      setDraft(content.replace(/\n*\[Attached: [^\]]+\]/g, '').trim())
      void dialogs.notify(
        'Attached file contents are not saved with the chat. Your message is back in the composer — attach the files again, then send.',
        'Attach the files again',
      )
      composer.current?.focus()
      return
    }
    void runGeneration({
      content,
      history: messages.slice(0, lastUserIndex),
      userMessage: lastUser,
      requestAttachments: [],
    })
  }

  function stop() {
    if (requestId.current) void cancelChat(requestId.current)
    controller.current?.abort()
  }

  async function attachFromHost() {
    if (attachmentPickerBusy.current) return
    attachmentPickerBusy.current = true
    try {
      const attachment = await pickAttachment()
      if (!attachment) return
      const content = await readAttachment(attachment.id)
      setAttachments((current) => [
        ...current.filter((item) => item.id !== attachment.id),
        {
          id: attachment.id,
          name: attachment.name,
          content,
          sizeBytes: attachment.sizeBytes,
          contentType: attachment.contentType,
        },
      ])
    } catch (error) {
      void dialogs.notify(
        error instanceof Error ? error.message : 'Could not attach that file.',
        'Could not attach the file',
      )
    } finally {
      attachmentPickerBusy.current = false
    }
  }

  function attachBrowserFile(file: File) {
    if (!isSupportedBrowserAttachmentName(file.name)) {
      void dialogs.notify(
        `${file.name} is not a supported text file. Choose a file with a supported text extension.`,
        'Unsupported attachment',
      )
      return
    }
    if (file.size > MAX_ATTACHMENT_BYTES) {
      void dialogs.notify(
        `${file.name} is larger than 1 MB. Juniper attaches text files up to 1 MB.`,
        'File too large',
      )
      return
    }
    void file.arrayBuffer().then(
      (bytes) => {
        if (bytes.byteLength > MAX_ATTACHMENT_BYTES) {
          void dialogs.notify(
            `${file.name} is larger than 1 MB. Juniper attaches text files up to 1 MB.`,
            'File too large',
          )
          return
        }
        let content: string
        try {
          content = new TextDecoder('utf-8', { fatal: true }).decode(bytes)
        } catch {
          void dialogs.notify(
            `${file.name} is not valid UTF-8 text. Choose a UTF-8 encoded text file.`,
            'Invalid text encoding',
          )
          return
        }
        setAttachments((current) => [
          ...current,
          {
            id: uid('attachment'),
            name: file.name,
            content,
            sizeBytes: bytes.byteLength,
            contentType: 'text/plain',
          },
        ])
      },
      () =>
        void dialogs.notify(
          `Juniper could not read ${file.name}. Choose the file again or use another copy.`,
          'Could not attach the file',
        ),
    )
  }

  async function decidePermission(requested: PermissionDecision) {
    const pending = permissionRequest
    if (!pending) return
    const decision: PermissionDecision =
      pending.standingGrantAllowed || requested === 'deny' ? requested : 'allow-once'
    try {
      await resolvePermission(pending.requestId, pending.callId, decision)
      if (decision === 'allow-chat' || decision === 'allow-assistant') {
        const timestamp = now()
        update((current) => ({
          ...current,
          permissions: [
            ...current.permissions.filter(
              (grant) =>
                !(
                  grant.toolName === pending.toolName &&
                  grant.assistantId === pending.assistantId &&
                  grant.scope === (decision === 'allow-chat' ? 'chat' : 'assistant') &&
                  (decision === 'allow-assistant' ||
                    grant.conversationId === pending.conversationId)
                ),
            ),
            {
              id: uid('permission'),
              toolName: pending.toolName,
              scope: decision === 'allow-chat' ? 'chat' : 'assistant',
              assistantId: pending.assistantId,
              ...(decision === 'allow-chat' ? { conversationId: pending.conversationId } : {}),
              createdAt: timestamp,
              updatedAt: timestamp,
            },
          ],
        }))
      }
      setPermissionRequest(null)
    } catch (error) {
      void dialogs.notify(
        error instanceof Error ? error.message : 'Could not record that permission.',
      )
    }
  }

  async function rename() {
    if (!conversation) return
    const title = await dialogs.prompt({
      title: 'Rename chat',
      label: 'Chat name',
      initialValue: conversation.title,
      maxLength: 120,
      confirmLabel: 'Rename',
    })
    if (!title) return
    updateConversation(conversation.id, (chat) => ({ ...chat, title, updatedAt: now() }))
  }

  async function remove() {
    if (!conversation) return
    const confirmed = await dialogs.confirm({
      title: 'Delete this chat?',
      message: `“${conversation.title}” and its attachment records will be permanently removed from Juniper.`,
      confirmLabel: 'Delete chat',
      danger: true,
    })
    if (!confirmed) return
    if (isGenerating) stop()
    update((current) => ({
      ...current,
      conversations: current.conversations.filter((chat) => chat.id !== conversation.id),
      attachments: current.attachments.filter(
        (attachment) => attachment.conversationId !== conversation.id,
      ),
      permissions: current.permissions.filter(
        (grant) => grant.scope !== 'chat' || grant.conversationId !== conversation.id,
      ),
    }))
    onDeleted()
  }

  function exportMarkdown() {
    if (!conversation || privateMode) return
    const body = messages
      .map(
        (message) =>
          `## ${message.role === 'user' ? 'You' : assistant.name}\n\n${textPart(message)}`,
      )
      .join('\n\n')
    download(`${conversation.title}.md`, body, 'text/markdown')
  }

  function selectModel(modelId: string | null) {
    setPickerOpen(false)
    if (conversation) {
      updateConversation(conversation.id, (chat) => ({ ...chat, modelProfileId: modelId }))
    } else {
      setNewChat({ ...newChat, modelProfileId: modelId })
    }
  }

  const lastAssistantIndex = (() => {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (messages[index]!.role === 'assistant') return index
    }
    return -1
  })()
  const canRetry =
    !isGenerating &&
    Boolean(model && provider) &&
    lastAssistantIndex >= 0 &&
    messages.slice(0, lastAssistantIndex).some((message) => message.role === 'user')

  const modelLabel = modelUnavailable ? 'Model unavailable' : (model?.displayName ?? 'Choose model')
  const statusText = modelUnavailable
    ? 'This chat’s model is unavailable. Choose another model to continue.'
    : !model
      ? 'No model selected · choose one in Models to start chatting.'
      : null

  const menuItems = conversation
    ? [
        { id: 'rename', label: 'Rename', icon: 'pencil' as const, onSelect: () => void rename() },
        {
          id: 'details',
          label: 'Chat details',
          icon: 'info' as const,
          onSelect: () => setDetailsOpen(true),
        },
        {
          id: 'export',
          label: 'Export as Markdown',
          icon: 'download' as const,
          onSelect: exportMarkdown,
          disabled: privateMode,
          hint: privateMode ? 'Private chats can’t be exported' : undefined,
        },
        ...(data.settings.developerMode
          ? [
              {
                id: 'inspector',
                label: 'Context inspector',
                icon: 'code' as const,
                onSelect: () => setInspectorOpen(true),
                disabled: !lastContext,
                hint: lastContext ? undefined : 'Send a message to inspect its context',
              },
            ]
          : []),
        {
          id: 'delete',
          label: 'Delete chat',
          icon: 'trash' as const,
          onSelect: () => void remove(),
          danger: true,
          separatorBefore: true,
        },
      ]
    : []

  const assistants = data.assistants

  return (
    <div className="chat-screen">
      <header className="chat-header">
        {onOpenHistory && (
          <button className="icon-button" onClick={onOpenHistory} aria-label="Show chats">
            <Icon name="menu" />
          </button>
        )}
        <div className="chat-title">
          {conversation ? (
            <>
              <AssistantAvatar assistant={assistant} className="chat-title-avatar" />
              <h1>{conversation.title}</h1>
            </>
          ) : assistants.length > 1 ? (
            <Menu
              label={`Assistant: ${assistant.name}`}
              className="assistant-switch"
              align="start"
              trigger={
                <>
                  <AssistantAvatar assistant={assistant} className="chat-title-avatar" />
                  <span>{assistant.name}</span>
                  <Icon name="chevronDown" size={16} />
                </>
              }
              items={assistants.map((item) => ({
                id: item.id,
                label: item.id === assistant.id ? `${item.name} ✓` : item.name,
                onSelect: () =>
                  setNewChat({
                    ...newChat,
                    assistantId: item.id,
                    modelProfileId: newChat.modelProfileId,
                  }),
              }))}
            />
          ) : (
            <>
              <AssistantAvatar assistant={assistant} className="chat-title-avatar" />
              <h1>{mobile ? assistant.name : 'New chat'}</h1>
            </>
          )}
          {privateMode && (
            <span className="private-chip">
              <Icon name="lock" size={14} />
              Private
            </span>
          )}
        </div>
        <div className="chat-header-actions">
          <button
            className={`model-pill ${modelUnavailable ? 'warning' : ''}`}
            onClick={() => setPickerOpen(true)}
            aria-haspopup="dialog"
            aria-label={`Model: ${modelLabel}${model ? `, ${labelExecutionLocation(model.executionLocation)}` : ''}. Change model`}
          >
            {model && (
              <i className={`location-dot ${model.executionLocation}`} aria-hidden="true" />
            )}
            <span>{modelLabel}</span>
            <Icon name="chevronDown" size={16} />
          </button>
          {conversation ? (
            <Menu label="Chat options" items={menuItems} trigger={<Icon name="more" />} />
          ) : (
            <button
              className={`icon-button ${newChat.privateChat ? 'active' : ''}`}
              aria-pressed={newChat.privateChat}
              aria-label="Private chat"
              title={newChat.privateChat ? 'Private chat is on' : 'Start a private chat'}
              onClick={() => setNewChat({ ...newChat, privateChat: !newChat.privateChat })}
            >
              <Icon name="lock" />
            </button>
          )}
        </div>
      </header>
      <div
        ref={scroller}
        className="chat-scroll"
        id="chat-content"
        tabIndex={-1}
        onScroll={(event) => {
          const element = event.currentTarget
          pinnedToBottom.current =
            element.scrollHeight - element.scrollTop - element.clientHeight < 120
        }}
      >
        <div className="chat-column">
          {messages.length === 0 ? (
            <div className="chat-welcome">
              <AssistantAvatar assistant={assistant} className="welcome-avatar" />
              <h2>{assistant.welcomeMessage || `What can ${assistant.name} help with?`}</h2>
              {privateMode ? (
                <p className="welcome-note">
                  <Icon name="lock" size={16} /> Private chat — it isn’t saved after this session,
                  can’t be exported, and is never included in chat search tools.
                </p>
              ) : statusText ? (
                <p className="welcome-note">{statusText}</p>
              ) : null}
              {!model && (
                <button className="button primary" onClick={onBrowseModels}>
                  Get a model
                </button>
              )}
              {model && assistant.suggestedPrompts.length > 0 && (
                <div className="suggestions">
                  {assistant.suggestedPrompts.map((prompt) => (
                    <button
                      key={prompt}
                      className="suggestion"
                      onClick={() => {
                        setDraft(prompt)
                        composer.current?.focus()
                      }}
                    >
                      {prompt}
                    </button>
                  ))}
                </div>
              )}
            </div>
          ) : (
            messages.map((message, index) => {
              const producedBy = message.modelId
                ? data.models.find(
                    (item) =>
                      (item.modelId === message.modelId || item.id === message.modelId) &&
                      (!message.providerId || item.providerId === message.providerId),
                  )
                : undefined
              return (
                <MessageBubble
                  key={message.id}
                  message={message}
                  assistant={assistant}
                  developerMode={data.settings.developerMode}
                  showTimestamp={data.settings.showTimestamps}
                  showDetails={data.settings.showMessageDetails}
                  modelName={producedBy?.displayName ?? message.modelId}
                  statusText={message.isStreaming ? (phase ?? undefined) : undefined}
                  onRetry={index === lastAssistantIndex && canRetry ? regenerate : undefined}
                />
              )
            })
          )}
        </div>
      </div>
      <div className="visually-hidden" role="status" aria-live="polite">
        {announcement}
      </div>
      <div className="chat-composer-dock">
        {/* Android document pickers return content URIs; use the WebView File API instead. */}
        <Composer
          value={draft}
          onChange={setDraft}
          onSubmit={send}
          onStop={stop}
          generating={isGenerating}
          canSend={Boolean((draft.trim() || attachments.length) && model && provider)}
          assistantName={assistant.name}
          privateChat={privateMode}
          touchPrimary={touchPrimary}
          attachments={attachments}
          onRemoveAttachment={(id) =>
            setAttachments((current) => current.filter((item) => item.id !== id))
          }
          onAttachHost={() => void attachFromHost()}
          onAttachBrowserFile={attachBrowserFile}
          useHostPicker={runningInTauri && !runningOnAndroid}
          textareaRef={composer}
          onFocusChange={onComposerFocus}
          status={
            messages.length > 0 && statusText ? (
              <>
                <span>{statusText}</span>
                <button className="link-button" onClick={() => setPickerOpen(true)}>
                  Choose a model
                </button>
              </>
            ) : undefined
          }
        />
      </div>

      <ModelPicker
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        data={data}
        assistant={assistant}
        selectedModelId={
          conversation
            ? modelUnavailable
              ? '__unavailable__'
              : (conversation.modelProfileId ?? null)
            : newChat.modelProfileId
        }
        unavailableModel={modelUnavailable ? route.pinnedModel : undefined}
        onSelect={selectModel}
        onBrowse={() => {
          setPickerOpen(false)
          onBrowseModels()
        }}
        sheet={mobile}
      />

      {conversation && (
        <Modal
          open={detailsOpen}
          onClose={() => setDetailsOpen(false)}
          title="Chat details"
          variant={mobile ? 'sheet' : 'dialog'}
        >
          <dl className="detail-list">
            <div>
              <dt>Assistant</dt>
              <dd>{assistant.name}</dd>
            </div>
            <div>
              <dt>Model</dt>
              <dd>{modelUnavailable ? 'Unavailable' : (model?.displayName ?? 'Not selected')}</dd>
            </div>
            <div>
              <dt>Provider</dt>
              <dd>{provider?.name ?? 'None'}</dd>
            </div>
            <div>
              <dt>Runs</dt>
              <dd>
                <LocationBadge location={model?.executionLocation} />
              </dd>
            </div>
            <div>
              <dt>Saved</dt>
              <dd>{privateMode ? 'Private · this session only' : 'Saved on this device'}</dd>
            </div>
            <div>
              <dt>Memory</dt>
              <dd>{assistant.memoryPolicy === 'curated' ? 'Curated memories' : 'Off'}</dd>
            </div>
            <div>
              <dt>Messages</dt>
              <dd>{messages.length}</dd>
            </div>
          </dl>
        </Modal>
      )}

      {data.settings.developerMode && lastContext && (
        <Modal
          open={inspectorOpen}
          onClose={() => setInspectorOpen(false)}
          title="Context inspector"
          description={`Estimated ${lastContext.estimatedTokens.toLocaleString()} / ${lastContext.contextLimit.toLocaleString()} tokens, including ${lastContext.reservedOutputTokens.toLocaleString()} reserved for the answer · ${lastContext.contextLimitAssumed ? 'context limit assumed' : 'runtime limit'} · ${lastContext.truncated ? 'older history truncated' : 'no truncation'}`}
        >
          <div className="context-inspector">
            <pre>{`[Added by the native host]\n${lastContext.constitutionId}, then a runtime section listing the model, where it runs, and these tools:\n${lastContext.tools.join('\n') || 'None'}`}</pre>
            <pre>{`[Assistant profile]\n${lastContext.profile}`}</pre>
            <pre>{`[Memories, sent as data]\n${lastContext.memory.join('\n') || 'None'}`}</pre>
            <pre>{`[Conversation]\n${lastContext.conversation.map((item) => `${item.role}: ${item.content}`).join('\n\n')}`}</pre>
            <pre>{`[Files]\n${lastContext.attachments.join('\n') || 'None'}\n\n[Current user]\n${lastContext.currentUserMessage}`}</pre>
          </div>
        </Modal>
      )}

      {permissionRequest && (
        <Modal
          open
          role="alertdialog"
          onClose={() => void decidePermission('deny')}
          title={`Allow ${permissionRequest.displayName || permissionRequest.toolName}?`}
          description={
            <>
              <p>
                {assistantFor(data, permissionRequest.assistantId).name} asked to use{' '}
                <code>{permissionRequest.toolName}</code>. Juniper runs it only if you allow it.
                Closing this dialog denies the request.
              </p>
              {permissionRequest.preview && (
                <p className="permission-preview">{permissionRequest.preview}</p>
              )}
              <p className="muted">
                {builtinTools.find((tool) => tool.name === permissionRequest.toolName)?.description}
                {!permissionRequest.standingGrantAllowed &&
                  ' Changes to saved data are approved one at a time.'}
              </p>
            </>
          }
          footer={
            <div className="permission-actions">
              <button
                className="button primary"
                onClick={() => void decidePermission('allow-once')}
              >
                Allow once
              </button>
              {permissionRequest.standingGrantAllowed && (
                <>
                  <button
                    className="button secondary"
                    onClick={() => void decidePermission('allow-chat')}
                  >
                    Allow for this chat
                  </button>
                  <button
                    className="button secondary"
                    onClick={() => void decidePermission('allow-assistant')}
                  >
                    Always allow for this assistant
                  </button>
                </>
              )}
              <button
                className="button ghost"
                data-autofocus
                onClick={() => void decidePermission('deny')}
              >
                Deny
              </button>
            </div>
          }
        />
      )}
    </div>
  )
}
