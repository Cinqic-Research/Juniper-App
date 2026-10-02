import constitution from '../../config/behavior/constitution.v1.json'
import type { Assistant, ChatMessage, Memory, ToolDefinition } from '../types'

export interface ContextMessage {
  role: 'user' | 'assistant'
  content: string
}

/**
 * What the interface sends for one request. The native host adds the
 * constitution and the runtime section in front of `profile`, so they are
 * not built (or removable) here; `hostLayerTokens` only budgets for them.
 */
export interface ContextSummary {
  profile: string
  constitutionId: string
  memoryIds: string[]
  memory: string[]
  conversation: ContextMessage[]
  currentUserMessage: string
  tools: string[]
  attachments: string[]
  hostLayerTokens: number
  reservedOutputTokens: number
  estimatedTokens: number
  contextLimit: number
  contextLimitAssumed: boolean
  truncated: boolean
  /** The required layers, current message, and output budget alone exceed the limit. */
  overflow: boolean
}

export interface ContextOptions {
  privateChat?: boolean
  reservedOutputTokens?: number
}

const DEFAULT_CONTEXT_LIMIT = 8192
// The identity sentence and runtime section (model, location, tools) are
// composed natively; these are generous estimates. The host measures the real
// prompt with the model's tokenizer before sending it where it can.
const IDENTITY_TOKENS = 80
const RUNTIME_SECTION_TOKENS = 160
const MAX_CONTEXT_MEMORIES = 64

function estimateTokens(value: string): number {
  return Math.ceil(value.length / 4)
}

export const CONSTITUTION_ID = constitution.id
const CONSTITUTION_TOKENS =
  IDENTITY_TOKENS + estimateTokens(constitution.rules.map((rule) => rule.text).join('\n'))

function band(value: number): 'low' | 'balanced' | 'high' {
  if (value >= 67) return 'high'
  if (value <= 33) return 'low'
  return 'balanced'
}

export function compilePersonality(personality: Assistant['personality']): string {
  const labels = Object.entries(personality).map(([key, value]) => `${key}: ${band(value)}`)
  const guidance = [
    personality.warmth >= 67
      ? 'Communicate warmth through attentive, natural wording; do not turn it into repeated greetings, habitual reassurance, pet names, or obligatory emoji.'
      : personality.warmth <= 33
        ? 'Keep emotional expressiveness restrained and professional without becoming cold.'
        : 'Use a friendly, measured tone.',
    personality.directness >= 67
      ? 'Answer the latest question promptly and give a useful first pass, including concrete options for broad advice, before any optional follow-up; skip unnecessary throat-clearing and redundant restatement of the conversation opening.'
      : personality.directness <= 33
        ? 'Use exploratory language and offer options before recommending.'
        : 'Balance clarity with appropriate nuance.',
    personality.playfulness >= 67
      ? 'Allow light, well-timed playfulness when it fits.'
      : personality.playfulness <= 33
        ? 'Stay focused and avoid playful asides.'
        : 'Use occasional lightness only when it helps.',
    personality.detail >= 67
      ? 'Explain thoroughly with useful detail.'
      : personality.detail <= 33
        ? 'Keep answers concise and focused on the essentials.'
        : 'Give enough detail to make the answer actionable.',
    personality.creativity >= 67
      ? 'Offer inventive alternatives and fresh framing.'
      : personality.creativity <= 33
        ? 'Prefer conventional, dependable approaches.'
        : 'Use creativity when it improves the result.',
    personality.formality >= 67
      ? 'Use polished, formal wording.'
      : personality.formality <= 33
        ? 'Use relaxed, natural wording rather than formal prose.'
        : 'Use clear conversational wording.',
  ]
  return `Compiled personality controls (${labels.join(', ')}):\n${guidance.map((item) => `- ${item}`).join('\n')}`
}

function messageText(message: ChatMessage): string {
  return message.parts
    .filter((part) => part.type === 'text' && part.text)
    .map((part) => part.text ?? '')
    .join('')
}

/**
 * A failed or truncated reply is not an answer, so it never becomes context
 * a later turn could build on.
 */
function usableReply(message: ChatMessage): boolean {
  return !message.parts.some((part) => part.type === 'error')
}

/** Groups history into exchanges so truncation never splits a question from its answer. */
function exchanges(messages: ContextMessage[]): ContextMessage[][] {
  const groups: ContextMessage[][] = []
  for (const message of messages) {
    const last = groups[groups.length - 1]
    if (message.role === 'user' || !last) groups.push([message])
    else last.push(message)
  }
  return groups
}

export function buildContext(
  assistant: Assistant,
  memories: Memory[],
  messages: ChatMessage[],
  tools: ToolDefinition[],
  limit?: number,
  currentUserMessage?: string,
  attachments: Array<{ name: string; content?: string }> = [],
  options: ContextOptions = {},
): ContextSummary {
  const selectedMemories =
    assistant.memoryPolicy === 'curated' && !options.privateChat
      ? memories
          .filter(
            (memory) =>
              memory.enabled && (!memory.assistantId || memory.assistantId === assistant.id),
          )
          .slice(-MAX_CONTEXT_MEMORIES)
      : []
  const toolNames = tools
    .filter((tool) => tool.enabled)
    .map((tool) => `${tool.name}: ${tool.description}`)
  const current = currentUserMessage ?? ''
  // A reverse loop rather than findLastIndex, which Android System WebView
  // builds before Chromium 97 do not implement.
  let currentIndex = -1
  if (currentUserMessage) {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index]!
      if (message.role === 'user' && messageText(message) === currentUserMessage) {
        currentIndex = index
        break
      }
    }
  }
  const candidates: ContextMessage[] = messages.flatMap((message, index) => {
    if (index === currentIndex) return []
    if (message.role !== 'user' && message.role !== 'assistant') return []
    if (message.role === 'assistant' && !usableReply(message)) return []
    const content = messageText(message)
    if (!content) return []
    return [{ role: message.role, content }]
  })
  const hasConversationHistory = candidates.length > 0
  const profile = [
    assistant.systemPrompt,
    compilePersonality(assistant.personality),
    `Response preference: ${assistant.responseLength}.`,
    hasConversationHistory
      ? 'Conversation state: this is an ongoing exchange. Start with the latest answer or a relevant acknowledgment; do not add a greeting or re-introduction unless the user explicitly greets you.'
      : 'Conversation state: this is the beginning of the exchange. A brief greeting is optional when it is natural, but answer the user directly when the request is clear.',
  ].join('\n\n')
  const contextLimitAssumed = !limit || !Number.isFinite(limit) || limit <= 0
  const contextLimit = contextLimitAssumed
    ? DEFAULT_CONTEXT_LIMIT
    : Math.max(256, Math.floor(limit))
  // A configured output budget larger than the context cannot all be
  // reserved; servers cap it, and the host checks the real figure where it can.
  const reservedOutputTokens = Math.min(
    Math.max(0, Math.floor(options.reservedOutputTokens ?? 0)),
    Math.floor(contextLimit / 2),
  )
  const hostLayerTokens =
    CONSTITUTION_TOKENS + RUNTIME_SECTION_TOKENS + estimateTokens(toolNames.join('\n'))
  const attachmentTokens = attachments.reduce(
    (sum, attachment) => sum + estimateTokens(attachment.content ?? ''),
    0,
  )
  const fixedTokens =
    hostLayerTokens +
    estimateTokens(profile) +
    estimateTokens(current) +
    attachmentTokens +
    reservedOutputTokens
  let available = contextLimit - fixedTokens
  const keptMemories: Memory[] = []
  for (let index = selectedMemories.length - 1; index >= 0; index -= 1) {
    const memory = selectedMemories[index]!
    const size = estimateTokens(memory.content) + 4
    if (size > available) break
    keptMemories.unshift(memory)
    available -= size
  }
  const kept: ContextMessage[] = []
  let used = 0
  const groups = exchanges(candidates)
  for (let index = groups.length - 1; index >= 0; index -= 1) {
    const group = groups[index]!
    const size = group.reduce((sum, item) => sum + estimateTokens(item.content) + 4, 0)
    if (used + size > available) break
    kept.unshift(...group)
    used += size
  }
  const memoryTokens = keptMemories.reduce(
    (sum, memory) => sum + estimateTokens(memory.content) + 4,
    0,
  )
  return {
    profile,
    constitutionId: CONSTITUTION_ID,
    memoryIds: keptMemories.map((memory) => memory.id),
    memory: keptMemories.map((memory) => memory.content),
    conversation: kept,
    currentUserMessage: current,
    tools: toolNames,
    attachments: attachments.map((attachment) => attachment.name),
    hostLayerTokens,
    reservedOutputTokens,
    estimatedTokens: fixedTokens + memoryTokens + used,
    contextLimit,
    contextLimitAssumed,
    truncated: kept.length < candidates.length || keptMemories.length < selectedMemories.length,
    overflow: fixedTokens > contextLimit,
  }
}
