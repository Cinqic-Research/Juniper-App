import { useEffect, useId, useLayoutEffect, useRef } from 'react'
import type { FormEvent, ReactNode, RefObject } from 'react'
import { Icon } from './icons'

export interface StagedAttachment {
  id: string
  name: string
  content: string
  sizeBytes?: number
  contentType?: string
}

export const BROWSER_ATTACHMENT_EXTENSIONS = [
  'txt',
  'md',
  'json',
  'csv',
  'toml',
  'yaml',
  'yml',
  'rs',
  'ts',
  'tsx',
  'js',
  'jsx',
  'py',
  'css',
  'html',
] as const

export const BROWSER_ATTACHMENT_ACCEPT = [
  ...BROWSER_ATTACHMENT_EXTENSIONS.map((extension) => `.${extension}`),
  'text/plain',
  'application/json',
  'text/markdown',
].join(',')

export function isSupportedBrowserAttachmentName(name: string): boolean {
  const extensionStart = name.lastIndexOf('.')
  if (extensionStart <= 0) return false
  const extension = name.slice(extensionStart + 1).toLowerCase()
  return BROWSER_ATTACHMENT_EXTENSIONS.some((supported) => supported === extension)
}

const MAX_ROWS_HEIGHT = 224

function fitTextarea(textarea: HTMLTextAreaElement | null) {
  if (!textarea) return
  textarea.style.height = 'auto'
  textarea.style.height = `${Math.min(textarea.scrollHeight, MAX_ROWS_HEIGHT)}px`
}

export function Composer({
  value,
  onChange,
  onSubmit,
  onStop,
  generating,
  canSend,
  assistantName,
  privateChat,
  touchPrimary,
  attachments,
  onRemoveAttachment,
  onAttachHost,
  onAttachBrowserFile,
  useHostPicker,
  status,
  textareaRef,
  onFocusChange,
}: {
  value: string
  onChange: (value: string) => void
  onSubmit: () => void
  onStop: () => void
  generating: boolean
  canSend: boolean
  assistantName: string
  privateChat: boolean
  touchPrimary: boolean
  attachments: StagedAttachment[]
  onRemoveAttachment: (id: string) => void
  onAttachHost: () => void
  onAttachBrowserFile: (file: File) => void
  useHostPicker: boolean
  /** A short reason or notice shown under the composer, if any. */
  status?: ReactNode
  textareaRef: RefObject<HTMLTextAreaElement | null>
  onFocusChange?: (focused: boolean) => void
}) {
  const fileInput = useRef<HTMLInputElement>(null)
  const hintId = useId()

  useLayoutEffect(() => fitTextarea(textareaRef.current), [value, textareaRef])

  // Width changes (sidebar, window, rotation) and late font loads change wrapping.
  useEffect(() => {
    const textarea = textareaRef.current
    if (!textarea) return
    const refit = () => fitTextarea(textarea)
    let lastWidth = textarea.clientWidth
    const observer =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(() => {
            if (textarea.clientWidth === lastWidth) return
            lastWidth = textarea.clientWidth
            refit()
          })
    observer?.observe(textarea)
    void document.fonts?.ready.then(refit)
    return () => observer?.disconnect()
  }, [textareaRef])

  useEffect(() => () => onFocusChange?.(false), [onFocusChange])

  function submit(event?: FormEvent) {
    event?.preventDefault()
    if (canSend && !generating) onSubmit()
  }

  const hint = touchPrimary ? undefined : 'Enter to send, Shift+Enter for a new line'

  return (
    <form className="composer-area" onSubmit={submit}>
      <div className={`composer ${privateChat ? 'private' : ''}`}>
        {attachments.length > 0 && (
          <ul className="attachment-chips" aria-label="Attached files">
            {attachments.map((attachment) => (
              <li key={attachment.id} className="attachment-chip">
                <Icon name="attach" size={15} />
                <span>{attachment.name}</span>
                <button
                  type="button"
                  className="chip-remove"
                  aria-label={`Remove ${attachment.name}`}
                  onClick={() => onRemoveAttachment(attachment.id)}
                >
                  <Icon name="close" size={14} />
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="composer-row">
          <button
            type="button"
            className="composer-button"
            aria-label="Attach a file"
            title="Attach a file"
            onClick={() => (useHostPicker ? onAttachHost() : fileInput.current?.click())}
          >
            <Icon name="attach" />
          </button>
          <input
            ref={fileInput}
            type="file"
            hidden
            tabIndex={-1}
            accept={BROWSER_ATTACHMENT_ACCEPT}
            onChange={(event) => {
              const file = event.target.files?.[0]
              event.target.value = ''
              if (file) onAttachBrowserFile(file)
            }}
          />
          <textarea
            ref={textareaRef}
            value={value}
            rows={1}
            onChange={(event) => onChange(event.target.value)}
            onFocus={() => onFocusChange?.(true)}
            onBlur={() => onFocusChange?.(false)}
            onKeyDown={(event) => {
              if (
                event.key === 'Enter' &&
                !event.shiftKey &&
                !touchPrimary &&
                !event.nativeEvent.isComposing
              ) {
                event.preventDefault()
                submit()
              }
            }}
            placeholder={
              privateChat ? `Message ${assistantName} privately` : `Message ${assistantName}`
            }
            aria-label={`Message ${assistantName}`}
            aria-describedby={hint ? hintId : undefined}
            enterKeyHint={touchPrimary ? 'enter' : 'send'}
          />
          <div className="composer-send-slot">
            {generating ? (
              <button
                type="button"
                className="composer-send stop"
                onClick={onStop}
                aria-label="Stop generating"
                title="Stop"
              >
                <Icon name="stop" />
              </button>
            ) : (
              <button
                type="submit"
                className="composer-send"
                disabled={!canSend}
                aria-label="Send message"
                title="Send"
              >
                <Icon name="send" />
              </button>
            )}
          </div>
        </div>
      </div>
      {hint && (
        <span id={hintId} className="visually-hidden">
          {hint}
        </span>
      )}
      {status && <div className="composer-status">{status}</div>}
    </form>
  )
}
