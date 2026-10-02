import { useId, useState } from 'react'
import type { Assistant, ChatMessage, MessagePart } from '../types'
import { Markdown } from '../lib/markdown'
import { AssistantAvatar } from './branding'
import { Icon } from './icons'

export function textPart(message: ChatMessage): string {
  return message.parts
    .filter((part) => part.type === 'text')
    .map((part) => part.text ?? '')
    .join('')
}

/** Copies text, falling back to a selection copy where the Clipboard API is missing. */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch {
    // Fall through to the selection-based copy.
  }
  const area = document.createElement('textarea')
  area.value = text
  area.setAttribute('readonly', '')
  area.style.position = 'fixed'
  area.style.opacity = '0'
  document.body.append(area)
  area.select()
  try {
    return document.execCommand('copy')
  } catch {
    return false
  } finally {
    area.remove()
  }
}

function formatTime(value: string): string | null {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return date.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
}

const MODEL_WRITTEN_REFERENCE =
  /\bhttps?:\/\/\S+|\bwww\.\S+\.\S+|\b10\.\d{4,9}\/\S+|\barXiv:\s?\d{4}\.\d{4,5}/i

/**
 * Juniper has no browsing tool, so a link, DOI, or arXiv ID in an answer came
 * from the model and has not been checked. A host tool result that contains
 * the same text is the only evidence the host itself supplied it.
 */
export function hasUncheckedReferences(message: ChatMessage): boolean {
  const content = textPart(message)
  const evidence = message.parts
    .filter((part) => part.type === 'tool-result' && part.status === 'success')
    .map((part) => part.text ?? '')
    .join('\n')
  const references = content.match(new RegExp(MODEL_WRITTEN_REFERENCE.source, 'gi')) ?? []
  return references.some((reference) => !evidence.includes(reference.replace(/[).,;]+$/, '')))
}

function provenanceDetails(message: ChatMessage, developerMode: boolean): string[] {
  const provenance = message.provenance
  if (!provenance) return []
  const runtime = provenance.runtime
  const details = [
    provenance.reasoningEffort ? `Reasoning effort: ${provenance.reasoningEffort}` : null,
    runtime?.runtimeBuild ? `Runtime: llama.cpp ${runtime.runtimeBuild}` : null,
  ]
  if (developerMode) {
    details.push(
      `Behavior: ${provenance.constitution} · ${provenance.backend}`,
      runtime?.artifactSha256
        ? `Model file SHA-256: ${runtime.artifactSha256.slice(0, 12)}…`
        : null,
      runtime?.templateSha256 ? `Template SHA-256: ${runtime.templateSha256.slice(0, 12)}…` : null,
    )
  }
  return details.filter(Boolean) as string[]
}

function toolLabel(part: MessagePart): string {
  return part.name ?? 'tool'
}

function statusLabel(status: MessagePart['status']): string {
  switch (status) {
    case 'success':
      return 'Completed'
    case 'denied':
      return 'Denied'
    case 'cancelled':
      return 'Cancelled'
    case 'unsupported':
      return 'Unsupported'
    case 'error':
      return 'Failed'
    default:
      return 'Waiting for Juniper'
  }
}

export function MessageBubble({
  message,
  assistant,
  developerMode = false,
  showTimestamp = false,
  showDetails = false,
  modelName,
  statusText,
  onRetry,
}: {
  message: ChatMessage
  assistant: Assistant
  developerMode?: boolean
  showTimestamp?: boolean
  showDetails?: boolean
  /** Human name for the model that produced this reply, when still known. */
  modelName?: string
  /** Progress text while waiting for the first streamed output. */
  statusText?: string
  /** Present only for the reply that can be regenerated. */
  onRetry?: () => void
}) {
  const user = message.role === 'user'
  const content = textPart(message)
  const toolCalls = message.parts.filter((part) => part.type === 'tool-call')
  const toolResults = message.parts.filter((part) => part.type === 'tool-result')
  const errorPart = message.parts.find((part) => part.type === 'error')
  const [copied, setCopied] = useState(false)
  const [detailsOpen, setDetailsOpen] = useState(false)
  const detailsId = useId()
  const time = formatTime(message.createdAt)
  const detailsVisible = showDetails || detailsOpen
  const author = user ? 'You' : assistant.name
  const waiting = !user && message.isStreaming && !content && !toolCalls.length
  const unchecked = !user && !message.isStreaming && hasUncheckedReferences(message)

  async function copy() {
    if (await copyText(content)) {
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    }
  }

  const details = [
    modelName ? `Model: ${modelName}` : null,
    message.modelId && message.modelId !== modelName ? `ID: ${message.modelId}` : null,
    message.usage?.totalTokens ? `${message.usage.totalTokens.toLocaleString()} tokens` : null,
    message.usage?.outputTokens && !message.usage.totalTokens
      ? `${message.usage.outputTokens.toLocaleString()} output tokens`
      : null,
    message.usage?.durationMs ? `${(message.usage.durationMs / 1000).toFixed(1)} s` : null,
    ...provenanceDetails(message, developerMode),
    time,
  ].filter(Boolean) as string[]

  return (
    <article
      className={`message ${user ? 'user' : 'assistant'} ${errorPart ? 'failed' : ''}`}
      aria-label={`${author}${time && showTimestamp ? ` at ${time}` : ''}`}
    >
      {!user && (
        <div className="message-author">
          <AssistantAvatar assistant={assistant} className="message-avatar" />
          <span>{assistant.name}</span>
          {showTimestamp && time && <time dateTime={message.createdAt}>{time}</time>}
        </div>
      )}
      {user && showTimestamp && time && (
        <div className="message-author user-time">
          <time dateTime={message.createdAt}>{time}</time>
        </div>
      )}
      <div className="message-body">
        {toolCalls.length > 0 && (
          <details className="disclosure">
            <summary>
              <Icon name="chevronRight" size={16} />
              {toolCalls.length === 1
                ? `Used ${toolLabel(toolCalls[0]!)}`
                : `Used ${toolCalls.length} tools`}
            </summary>
            <ul className="tool-activity">
              {toolCalls.map((call) => {
                const result = toolResults.find(
                  (item) => item.metadata?.callId === call.metadata?.callId,
                )
                const status = result?.status ?? call.status
                return (
                  <li key={call.id}>
                    <div className="tool-activity-head">
                      <strong>{toolLabel(call)}</strong>
                      <span className={`tool-status ${status === 'success' ? 'ok' : 'not-ok'}`}>
                        {result ? statusLabel(result.status) : statusLabel(call.status)}
                      </span>
                    </div>
                    {result?.text && (
                      <pre className="tool-result" aria-label="Result from Juniper">
                        {result.text}
                      </pre>
                    )}
                  </li>
                )
              })}
              {toolResults
                .filter(
                  (result) =>
                    !toolCalls.some((call) => call.metadata?.callId === result.metadata?.callId),
                )
                .map((result) => (
                  <li key={result.id}>
                    <div className="tool-activity-head">
                      <strong>{toolLabel(result)}</strong>
                      <span
                        className={`tool-status ${result.status === 'success' ? 'ok' : 'not-ok'}`}
                      >
                        {statusLabel(result.status)}
                      </span>
                    </div>
                    {result.text && <pre className="tool-result">{result.text}</pre>}
                  </li>
                ))}
            </ul>
          </details>
        )}
        {errorPart && content && <Markdown content={content} />}
        {errorPart ? (
          <div className="message-error" role="alert">
            <Icon name="info" size={18} />
            <div>
              <p>{errorPart.text || 'The response failed.'}</p>
              {developerMode && errorPart.metadata?.errorCode && (
                <small>Internal code: {String(errorPart.metadata.errorCode)}</small>
              )}
            </div>
          </div>
        ) : content ? (
          <Markdown content={content} />
        ) : waiting ? (
          <span className="typing">
            <span className="typing-dots" aria-hidden="true">
              <i />
              <i />
              <i />
            </span>
            <span className="typing-label">{statusText ?? `${assistant.name} is responding…`}</span>
          </span>
        ) : !user && !message.isStreaming && !toolCalls.length ? (
          <p className="message-empty">No text was returned.</p>
        ) : null}
        {unchecked && (
          <p className="message-note">
            Links and citations in this answer were written by the model. Juniper has not checked
            that they exist.
          </p>
        )}
      </div>
      {!message.isStreaming && (
        <div className="message-actions">
          {content && (
            <button
              className="icon-button small"
              onClick={() => void copy()}
              aria-label={copied ? 'Copied' : `Copy ${user ? 'your message' : 'response'}`}
              title={copied ? 'Copied' : 'Copy'}
            >
              <Icon name={copied ? 'check' : 'copy'} size={17} />
            </button>
          )}
          {onRetry && (
            <button
              className="icon-button small"
              onClick={onRetry}
              aria-label="Regenerate response"
              title="Regenerate"
            >
              <Icon name="retry" size={17} />
            </button>
          )}
          {!user && details.length > 0 && !showDetails && (
            <button
              className="icon-button small"
              onClick={() => setDetailsOpen((value) => !value)}
              aria-expanded={detailsOpen}
              aria-controls={detailsId}
              aria-label="Response details"
              title="Details"
            >
              <Icon name="info" size={17} />
            </button>
          )}
        </div>
      )}
      {!user && detailsVisible && details.length > 0 && (
        <p className="message-details" id={detailsId}>
          {details.join(' · ')}
        </p>
      )}
    </article>
  )
}
