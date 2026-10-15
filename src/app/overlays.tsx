import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode, RefObject } from 'react'
import { createPortal } from 'react-dom'
import { currentHistoryState, onHistoryPop, popHistorySilently, pushHistory } from './history'
import { Icon } from './icons'
import type { IconName } from './icons'

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])'

function focusableWithin(element: HTMLElement): HTMLElement[] {
  return Array.from(element.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (item) => !item.closest('[hidden]') && !item.closest('[inert]'),
  )
}

// ---------------------------------------------------------------------------
// History-backed dismissal
//
// On Android the system back gesture calls WebView.goBack() whenever the page
// has history. Each open overlay pushes one entry so back closes the overlay
// instead of leaving the app; closing it any other way pops that entry again.
// ---------------------------------------------------------------------------

const overlayStack: string[] = []

export function useBackDismiss(open: boolean, onDismiss: () => void) {
  const id = useId()
  const dismiss = useRef(onDismiss)
  useLayoutEffect(() => {
    dismiss.current = onDismiss
  })
  useEffect(() => {
    if (!open) return
    overlayStack.push(id)
    pushHistory({ ...currentHistoryState(), juniperOverlay: id })
    const stop = onHistoryPop(() => {
      if (overlayStack[overlayStack.length - 1] !== id) return
      overlayStack.pop()
      dismiss.current()
    })
    return () => {
      stop()
      const index = overlayStack.indexOf(id)
      if (index >= 0) {
        overlayStack.splice(index, 1)
        popHistorySilently()
      }
    }
  }, [open, id])
}

// ---------------------------------------------------------------------------
// Modal dialogs and sheets
// ---------------------------------------------------------------------------

let openModals = 0

function setAppInert(inert: boolean) {
  const app = document.querySelector<HTMLElement>('.app-frame')
  if (!app) return
  if (inert) app.setAttribute('inert', '')
  else app.removeAttribute('inert')
}

export function Modal({
  open,
  onClose,
  title,
  description,
  children,
  variant = 'dialog',
  role = 'dialog',
  hideTitle = false,
  footer,
  initialFocus,
}: {
  open: boolean
  onClose: () => void
  title: string
  description?: ReactNode
  children?: ReactNode
  variant?: 'dialog' | 'sheet' | 'drawer'
  role?: 'dialog' | 'alertdialog'
  hideTitle?: boolean
  footer?: ReactNode
  initialFocus?: RefObject<HTMLElement | null>
}) {
  const panel = useRef<HTMLDivElement>(null)
  const titleId = useId()
  const descriptionId = useId()
  const close = useRef(onClose)
  useLayoutEffect(() => {
    close.current = onClose
  })
  useBackDismiss(open, onClose)

  useEffect(() => {
    if (!open) return
    const previous = document.activeElement as HTMLElement | null
    openModals += 1
    setAppInert(true)
    const target =
      initialFocus?.current ??
      panel.current?.querySelector<HTMLElement>('[data-autofocus]') ??
      (panel.current ? focusableWithin(panel.current)[0] : undefined) ??
      panel.current
    target?.focus()
    return () => {
      openModals -= 1
      if (openModals === 0) setAppInert(false)
      if (previous && document.contains(previous)) previous.focus()
    }
  }, [open, initialFocus])

  if (!open) return null

  function onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key === 'Escape') {
      event.stopPropagation()
      close.current()
      return
    }
    if (event.key !== 'Tab' || !panel.current) return
    const items = focusableWithin(panel.current)
    if (items.length === 0) {
      event.preventDefault()
      return
    }
    const first = items[0]!
    const last = items[items.length - 1]!
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault()
      first.focus()
    }
  }

  return createPortal(
    <div
      className={`overlay overlay-${variant}`}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) close.current()
      }}
    >
      <div
        ref={panel}
        className={`overlay-panel overlay-panel-${variant}`}
        role={role}
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descriptionId : undefined}
        tabIndex={-1}
        onKeyDown={onKeyDown}
      >
        <div className={`overlay-header ${hideTitle ? 'visually-hidden-title' : ''}`}>
          <h2 id={titleId} className={hideTitle ? 'visually-hidden' : undefined}>
            {title}
          </h2>
          {!hideTitle && (
            <button className="icon-button" aria-label="Close" onClick={() => close.current()}>
              <Icon name="close" />
            </button>
          )}
        </div>
        {description && (
          <div id={descriptionId} className="overlay-description">
            {description}
          </div>
        )}
        {children && <div className="overlay-body">{children}</div>}
        {footer && <div className="overlay-footer">{footer}</div>}
      </div>
    </div>,
    document.body,
  )
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

export interface MenuItem {
  id: string
  label: string
  icon?: IconName
  onSelect: () => void
  disabled?: boolean
  /** Shown to explain why an item is unavailable. */
  hint?: string
  danger?: boolean
  separatorBefore?: boolean
}

export function Menu({
  label,
  items,
  trigger,
  align = 'end',
  className = 'icon-button',
}: {
  label: string
  items: MenuItem[]
  trigger: ReactNode
  align?: 'start' | 'end'
  className?: string
}) {
  const [open, setOpen] = useState(false)
  const button = useRef<HTMLButtonElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const menuId = useId()
  const close = useCallback((restoreFocus = true) => {
    setOpen(false)
    if (restoreFocus) button.current?.focus()
  }, [])
  useBackDismiss(open, () => setOpen(false))

  useEffect(() => {
    if (!open) return
    list.current
      ?.querySelector<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])')
      ?.focus()
    function onPointer(event: MouseEvent) {
      const target = event.target as Node
      if (!list.current?.contains(target) && !button.current?.contains(target)) close(false)
    }
    document.addEventListener('mousedown', onPointer)
    return () => document.removeEventListener('mousedown', onPointer)
  }, [open, close])

  function onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    const entries = Array.from(
      list.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [],
    )
    const index = entries.indexOf(document.activeElement as HTMLElement)
    const move = (next: number) => {
      event.preventDefault()
      entries[(next + entries.length) % entries.length]?.focus()
    }
    if (event.key === 'ArrowDown') move(index + 1)
    else if (event.key === 'ArrowUp') move(index - 1)
    else if (event.key === 'Home') move(0)
    else if (event.key === 'End') move(entries.length - 1)
    else if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      close()
    } else if (event.key === 'Tab') close(false)
  }

  return (
    <div className="menu-anchor">
      <button
        ref={button}
        className={className}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={() => setOpen((value) => !value)}
      >
        {trigger}
      </button>
      {open && (
        <div
          ref={list}
          id={menuId}
          className={`menu menu-${align}`}
          role="menu"
          aria-label={label}
          onKeyDown={onKeyDown}
        >
          {items.map((item) => (
            <div key={item.id} role="none" className={item.separatorBefore ? 'menu-separated' : ''}>
              <button
                role="menuitem"
                tabIndex={-1}
                className={`menu-item ${item.danger ? 'danger' : ''}`}
                aria-disabled={item.disabled || undefined}
                onClick={() => {
                  if (item.disabled) return
                  close(false)
                  item.onSelect()
                }}
              >
                {item.icon && <Icon name={item.icon} size={18} />}
                <span>
                  {item.label}
                  {item.hint && <small>{item.hint}</small>}
                </span>
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// In-app confirm, prompt, and notice dialogs
// ---------------------------------------------------------------------------

interface ConfirmOptions {
  title: string
  message?: ReactNode
  confirmLabel?: string
  cancelLabel?: string
  danger?: boolean
}

interface PromptOptions {
  title: string
  label: string
  initialValue?: string
  confirmLabel?: string
  maxLength?: number
}

type Pending = { key: number } & (
  | { kind: 'confirm'; options: ConfirmOptions; resolve: (value: boolean) => void }
  | { kind: 'prompt'; options: PromptOptions; resolve: (value: string | null) => void }
  | { kind: 'notice'; options: { title: string; message: string }; resolve: () => void }
)

let nextDialogKey = 0

interface Dialogs {
  confirm: (options: ConfirmOptions) => Promise<boolean>
  prompt: (options: PromptOptions) => Promise<string | null>
  notify: (message: string, title?: string) => Promise<void>
}

const DialogContext = createContext<Dialogs | null>(null)

export function useDialogs(): Dialogs {
  const context = useContext(DialogContext)
  if (!context) throw new Error('useDialogs must be used inside DialogProvider')
  return context
}

export function DialogProvider({ children }: { children: ReactNode }) {
  const [queue, setQueue] = useState<Pending[]>([])
  const [value, setValue] = useState('')
  const current = queue[0]

  const dialogs = useRef<Dialogs>({
    confirm: (options) =>
      new Promise((resolve) =>
        setQueue((items) => [
          ...items,
          { key: ++nextDialogKey, kind: 'confirm', options, resolve },
        ]),
      ),
    prompt: (options) =>
      new Promise((resolve) =>
        setQueue((items) => [...items, { key: ++nextDialogKey, kind: 'prompt', options, resolve }]),
      ),
    notify: (message, title = 'Something went wrong') =>
      new Promise((resolve) =>
        setQueue((items) => [
          ...items,
          { key: ++nextDialogKey, kind: 'notice', options: { title, message }, resolve },
        ]),
      ),
  })

  useEffect(() => {
    if (current?.kind === 'prompt') setValue(current.options.initialValue ?? '')
  }, [current])

  function finish(result: boolean) {
    if (!current) return
    if (current.kind === 'confirm') current.resolve(result)
    else if (current.kind === 'prompt') current.resolve(result ? value.trim() || null : null)
    else current.resolve()
    setQueue((items) => items.slice(1))
  }

  return (
    <DialogContext.Provider value={dialogs.current}>
      {children}
      {current && (
        <Modal
          key={current.key}
          open
          onClose={() => finish(false)}
          role={current.kind === 'prompt' ? 'dialog' : 'alertdialog'}
          title={current.options.title}
          description={
            current.kind === 'confirm'
              ? current.options.message
              : current.kind === 'notice'
                ? current.options.message
                : undefined
          }
          footer={
            current.kind === 'notice' ? (
              <button className="button primary" data-autofocus onClick={() => finish(true)}>
                OK
              </button>
            ) : (
              <>
                <button
                  className="button ghost"
                  data-autofocus={
                    current.kind === 'confirm' && current.options.danger ? true : undefined
                  }
                  onClick={() => finish(false)}
                >
                  {current.kind === 'confirm'
                    ? (current.options.cancelLabel ?? 'Cancel')
                    : 'Cancel'}
                </button>
                <button
                  className={`button ${current.kind === 'confirm' && current.options.danger ? 'danger' : 'primary'}`}
                  data-autofocus={
                    current.kind === 'confirm' && !current.options.danger ? true : undefined
                  }
                  disabled={current.kind === 'prompt' && !value.trim()}
                  onClick={() => finish(true)}
                >
                  {current.options.confirmLabel ?? (current.kind === 'prompt' ? 'Save' : 'Confirm')}
                </button>
              </>
            )
          }
        >
          {current.kind === 'prompt' && (
            <form
              onSubmit={(event) => {
                event.preventDefault()
                if (value.trim()) finish(true)
              }}
            >
              <label className="field">
                <span>{current.options.label}</span>
                <input
                  data-autofocus
                  value={value}
                  maxLength={current.options.maxLength}
                  onChange={(event) => setValue(event.target.value)}
                />
              </label>
            </form>
          )}
        </Modal>
      )}
    </DialogContext.Provider>
  )
}
