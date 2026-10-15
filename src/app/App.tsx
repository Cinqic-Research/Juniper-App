import { useCallback, useEffect, useRef, useState } from 'react'
import { applyAppearance } from '../lib/appearance'
import { initialAppData, modelProfileFromDiscovery } from '../lib/defaults'
import {
  checkProviderConnection,
  getWindowInsets,
  listenForBackButton,
  listProviderModels,
  loadNativeAppData,
  reportFrontendReady,
  runningInTauri,
  runningOnAndroid,
  saveNativeAppData,
  type WindowInsets,
} from '../lib/runtime'
import { agentApi, applyMemoryChanges, onAgentEvent } from '../lib/agent'
import { loadAppData, saveAppData } from '../lib/storage'
import type { AppData, Page, SettingsSection } from '../types'
import { JuniperMark } from './branding'
import { AgentScreen } from './AgentScreen'
import { ChatHistory, ChatScreen, type NewChatState } from './ChatScreen'
import {
  currentHistoryState,
  onHistoryDepth,
  onHistoryPop,
  pushHistory,
  replaceHistory,
} from './history'
import { Icon } from './icons'
import type { IconName } from './icons'
import { assistantFor, defaultAssistantFor, resolveRoute } from './model-labels'
import { ModelsScreen } from './ModelsScreen'
import { Onboarding } from './Onboarding'
import { DialogProvider, Modal, useDialogs } from './overlays'
import { SettingsScreen } from './SettingsScreen'
import { uid, useMediaQuery, useTouchPrimary } from './ui'

interface NavState {
  page: Page
  section: SettingsSection | null
}

const PAGES: Page[] = ['chats', 'agent', 'models', 'settings']

const NAV_ITEMS: Array<{ page: Page; label: string; icon: IconName; beta?: boolean }> = [
  { page: 'chats', label: 'Chat', icon: 'chat' },
  { page: 'agent', label: 'Agent', icon: 'sparkle', beta: true },
  { page: 'models', label: 'Models', icon: 'models' },
  { page: 'settings', label: 'Settings', icon: 'settings' },
]

function freshNewChat(): NewChatState {
  return { assistantId: null, privateChat: false, modelProfileId: null }
}

function isNavState(value: unknown): value is NavState {
  if (!value || typeof value !== 'object') return false
  const candidate = value as Partial<NavState>
  return PAGES.includes(candidate.page as Page)
}

// Viewport height with the keyboard closed, per width (orientation).
let restingViewport = { width: 0, height: 0 }

/**
 * Where Android already shrinks the WebView for the keyboard, the reported
 * keyboard inset would be applied twice; pad only by what is still covered.
 */
export function uncoveredKeyboardHeight(keyboard: number): number {
  const { innerWidth: width, innerHeight: height } = window
  if (keyboard <= 0 || width !== restingViewport.width) restingViewport = { width, height }
  const shrink = Math.max(0, restingViewport.height - height)
  return Math.max(0, keyboard - shrink)
}

function applyInsets(insets: WindowInsets) {
  const root = document.documentElement.style
  root.setProperty('--native-inset-top', `${Math.max(0, insets.top)}px`)
  root.setProperty('--native-inset-right', `${Math.max(0, insets.right)}px`)
  root.setProperty('--native-inset-bottom', `${Math.max(0, insets.bottom)}px`)
  root.setProperty('--native-inset-left', `${Math.max(0, insets.left)}px`)
  root.setProperty('--native-keyboard', `${uncoveredKeyboardHeight(insets.keyboard)}px`)
}

export default function App() {
  return (
    <DialogProvider>
      <JuniperApp />
    </DialogProvider>
  )
}

function JuniperApp() {
  const dialogs = useDialogs()
  const [data, setData] = useState<AppData>(() =>
    runningInTauri ? initialAppData() : loadAppData(),
  )
  const [hydrated, setHydrated] = useState(!runningInTauri)
  const [nav, setNav] = useState<NavState>({ page: 'chats', section: null })
  const [selectedChatId, setSelectedChatId] = useState<string | null>(null)
  const [viewKey, setViewKey] = useState(() => uid('view'))
  const [newChat, setNewChat] = useState<NewChatState>(freshNewChat)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [composing, setComposing] = useState(false)
  // Android reports the keyboard height; elsewhere composer focus stands in for it.
  const [keyboardOpen, setKeyboardOpen] = useState(false)
  const [replayingOnboarding, setReplayingOnboarding] = useState(false)
  // Set when stored state could not be read. Saving the in-memory defaults would
  // then overwrite the user's database, so persistence stays off for the session.
  const [loadFailed, setLoadFailed] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const mainRef = useRef<HTMLElement>(null)

  const mobile = useMediaQuery('(max-width: 760px)')
  const narrowDesktop = useMediaQuery('(max-width: 1099px)')
  const singlePaneSettings = useMediaQuery('(max-width: 899px)')
  const touchPrimary = useTouchPrimary()
  // Re-resolve "system" theme, contrast, and motion when the device changes them.
  const systemDark = useMediaQuery('(prefers-color-scheme: dark)')
  const systemContrast = useMediaQuery('(prefers-contrast: more)')
  const systemReducedMotion = useMediaQuery('(prefers-reduced-motion: reduce)')

  const update = useCallback(
    (change: (current: AppData) => AppData) => setData((current) => change(current)),
    [],
  )

  useEffect(() => {
    applyAppearance(document.documentElement, data.settings)
  }, [data.settings, systemDark, systemContrast, systemReducedMotion])

  useEffect(() => {
    if (!hydrated) return
    if (runningInTauri) {
      if (!loadFailed)
        void saveNativeAppData(data).then(
          () => setSaveError(null),
          (error: unknown) => setSaveError(error instanceof Error ? error.message : String(error)),
        )
    } else saveAppData(data)
  }, [data, hydrated, loadFailed])

  useEffect(() => {
    if (!runningInTauri) return
    void loadNativeAppData()
      .then((stored) => {
        if (stored) setData(stored)
      })
      .catch((error) => {
        setLoadFailed(true)
        void dialogs.notify(
          error instanceof Error ? error.message : 'Could not load the SQLite state.',
          'Juniper couldn’t read its stored data',
        )
      })
      .finally(() => setHydrated(true))
  }, [dialogs])

  useEffect(() => {
    if (!runningInTauri || !hydrated) return
    void reportFrontendReady().catch(() => undefined)
  }, [hydrated])

  // Memories an agent saved (each one approved by the user) are queued by the
  // host; the interface owns the memory list, so it applies and acknowledges them.
  useEffect(() => {
    if (!runningInTauri || !hydrated || loadFailed) return
    let applying = false
    const apply = async () => {
      if (applying) return
      applying = true
      try {
        const changes = await agentApi.memoryChanges()
        if (changes.length === 0) return
        update((current) => applyMemoryChanges(current, changes))
        await agentApi.memoryChangesApplied(changes.map((change) => change.id))
      } catch {
        // Retried on the next agent event.
      } finally {
        applying = false
      }
    }
    void apply()
    let stop: (() => void) | undefined
    let disposed = false
    void onAgentEvent((event) => {
      if (event.kind === 'inbox' || event.kind === 'task') void apply()
    }).then((unlisten) => {
      if (disposed) unlisten()
      else stop = unlisten
    })
    return () => {
      disposed = true
      stop?.()
    }
  }, [hydrated, loadFailed, update])

  // If the open chat disappears while its view is mounted, remount the view:
  // otherwise it would keep the old id and stream the next message into nothing.
  useEffect(() => {
    if (!hydrated || !selectedChatId) return
    if (data.conversations.some((chat) => chat.id === selectedChatId)) return
    setSelectedChatId(null)
    setNewChat(freshNewChat())
    setViewKey(uid('view'))
  }, [data.conversations, hydrated, selectedChatId])

  useEffect(() => {
    if (!runningInTauri || !hydrated) return
    const provider = data.providers.find((item) => item.enabled && item.kind === 'ollama')
    if (!provider) return
    void checkProviderConnection(provider)
      .then(() => {
        update((current) => {
          const currentProvider = current.providers.find((item) => item.id === provider.id)
          if (!currentProvider || currentProvider.status === 'connected') return current
          return {
            ...current,
            providers: current.providers.map((item) =>
              item.id === provider.id ? { ...item, status: 'connected' } : item,
            ),
          }
        })
        return listProviderModels(provider)
      })
      .then((models) => {
        if (!models) return
        update((current) => {
          const next = [...current.models]
          for (const discovered of models) {
            const existingIndex = next.findIndex(
              (model) => model.providerId === provider.id && model.modelId === discovered.modelId,
            )
            if (existingIndex >= 0) {
              next[existingIndex] = { ...next[existingIndex]!, status: 'ready' }
              continue
            }
            next.push(
              modelProfileFromDiscovery(provider, discovered.modelId, {
                displayName: discovered.displayName,
                fileSizeBytes: discovered.sizeBytes,
              }),
            )
          }
          return { ...current, models: next }
        })
      })
      .catch(() => {
        update((current) => {
          const currentProvider = current.providers.find((item) => item.id === provider.id)
          if (!currentProvider || currentProvider.status === 'offline') return current
          return {
            ...current,
            providers: current.providers.map((item) =>
              item.id === provider.id ? { ...item, status: 'offline' } : item,
            ),
          }
        })
      })
  }, [data.providers, hydrated, update])

  // Android draws Juniper edge to edge, so the native host reports the system
  // bar and keyboard insets that the WebView itself may not expose to CSS.
  useEffect(() => {
    if (!runningInTauri) return
    const onInsets = (event: Event) => {
      const detail = (event as CustomEvent<WindowInsets>).detail
      if (detail && typeof detail.top === 'number') {
        applyInsets(detail)
        setKeyboardOpen(detail.keyboard > 0)
      }
    }
    window.addEventListener('juniper-window-insets', onInsets)
    void getWindowInsets()
      .then((insets) => {
        if (!insets) return
        applyInsets(insets)
        setKeyboardOpen(insets.keyboard > 0)
      })
      .catch(() => undefined)
    return () => window.removeEventListener('juniper-window-insets', onInsets)
  }, [])

  // Android back closes an overlay or returns to the previous screen while
  // Juniper has history to go back through. With nothing left, the listener is
  // removed so Android's own back behaviour leaves the app as usual.
  useEffect(() => {
    if (!runningOnAndroid) return
    let unregister: (() => Promise<void>) | null = null
    let chain = Promise.resolve()
    const stop = onHistoryDepth((depth) => {
      chain = chain
        .then(async () => {
          if (depth > 0 && !unregister) {
            unregister = await listenForBackButton(() => window.history.back())
          } else if (depth === 0 && unregister) {
            const release = unregister
            unregister = null
            await release()
          }
        })
        .catch(() => undefined)
    })
    return () => {
      stop()
      void chain.then(() => unregister?.()).catch(() => undefined)
    }
  }, [])

  // In-app navigation is recorded in history so Android back and mouse back work.
  useEffect(() => {
    replaceHistory({ ...currentHistoryState(), juniperNav: { page: 'chats', section: null } })
    return onHistoryPop((state) => {
      // An entry Juniper never tagged is the page's starting point: Chats.
      const next = isNavState(state.juniperNav)
        ? state.juniperNav
        : { page: 'chats' as const, section: null }
      navRef.current = next
      setNav(next)
    })
  }, [])

  const navRef = useRef(nav)
  const navigate = useCallback((next: NavState, options: { replace?: boolean } = {}) => {
    const current = navRef.current
    if (current.page === next.page && current.section === next.section) return
    const state = { juniperNav: next }
    if (options.replace) replaceHistory(state)
    else pushHistory(state)
    navRef.current = next
    setNav(next)
  }, [])

  const startNewChat = useCallback(() => {
    setSelectedChatId(null)
    setNewChat(freshNewChat())
    setViewKey(uid('view'))
    setHistoryOpen(false)
    navigate({ page: 'chats', section: null })
  }, [navigate])

  const openChat = useCallback(
    (id: string) => {
      setSelectedChatId(id)
      setViewKey(id)
      setHistoryOpen(false)
      navigate({ page: 'chats', section: null })
    },
    [navigate],
  )

  const sidebarCollapsed =
    data.settings.sidebar === 'collapsed' || (data.settings.sidebar === 'auto' && narrowDesktop)

  const toggleSidebar = useCallback(() => {
    update((current) => {
      const collapsedNow =
        current.settings.sidebar === 'collapsed' ||
        (current.settings.sidebar === 'auto' && narrowDesktop)
      return {
        ...current,
        settings: { ...current.settings, sidebar: collapsedNow ? 'expanded' : 'collapsed' },
      }
    })
  }, [narrowDesktop, update])

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const modifier = event.ctrlKey || event.metaKey
      if (!modifier || event.altKey) return
      const key = event.key.toLowerCase()
      if (event.shiftKey && key === 'o') {
        event.preventDefault()
        startNewChat()
      } else if (event.shiftKey && key === 's' && !mobile) {
        event.preventDefault()
        toggleSidebar()
      } else if (!event.shiftKey && key === 'k') {
        event.preventDefault()
        if (mobile) {
          setHistoryOpen(true)
          return
        }
        if (sidebarCollapsed) toggleSidebar()
        window.setTimeout(() => searchRef.current?.focus(), 0)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [mobile, sidebarCollapsed, startNewChat, toggleSidebar])

  const selectedConversation = data.conversations.find((chat) => chat.id === selectedChatId)
  const route = resolveRoute(
    data,
    selectedConversation,
    newChat.assistantId ? assistantFor(data, newChat.assistantId) : defaultAssistantFor(data),
    newChat.modelProfileId,
  )

  function openSection(section: SettingsSection | null, options?: { replace?: boolean }) {
    navigate({ page: 'settings', section }, options)
  }

  function completeOnboarding() {
    update((current) => ({
      ...current,
      settings: { ...current.settings, onboardingComplete: true },
    }))
    setReplayingOnboarding(false)
  }

  const page = nav.page

  return (
    <>
      <div
        className="app-frame"
        data-layout={mobile ? 'mobile' : 'desktop'}
        data-sidebar={sidebarCollapsed ? 'collapsed' : 'expanded'}
        data-composing={
          mobile && (runningOnAndroid ? keyboardOpen : composing) ? 'true' : undefined
        }
      >
        <button className="skip-link" onClick={() => mainRef.current?.focus()}>
          Skip to content
        </button>
        {!mobile && (
          <aside className="sidebar" aria-label="Juniper">
            <div className="sidebar-top">
              {!sidebarCollapsed && (
                <div className="sidebar-brand">
                  <JuniperMark className="sidebar-logo" alt="" aria-hidden="true" />
                  <span>Juniper</span>
                </div>
              )}
              <button
                className="icon-button"
                onClick={toggleSidebar}
                aria-label={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
                aria-expanded={!sidebarCollapsed}
                title={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
              >
                <Icon name="sidebar" />
              </button>
            </div>
            <button
              className="sidebar-new-chat"
              onClick={startNewChat}
              title={sidebarCollapsed ? 'New chat' : undefined}
            >
              <Icon name="plus" />
              <span className={sidebarCollapsed ? 'visually-hidden' : undefined}>New chat</span>
            </button>
            <nav className="sidebar-nav" aria-label="Primary">
              {NAV_ITEMS.map((item) => (
                <button
                  key={item.page}
                  className={`sidebar-nav-item ${page === item.page ? 'active' : ''}`}
                  aria-current={page === item.page ? 'page' : undefined}
                  title={sidebarCollapsed ? item.label : undefined}
                  onClick={() =>
                    navigate({
                      page: item.page,
                      section: item.page === 'settings' ? nav.section : null,
                    })
                  }
                >
                  <Icon name={item.icon} />
                  <span className={sidebarCollapsed ? 'visually-hidden' : undefined}>
                    {item.label}
                    {item.beta && <span className="nav-beta"> Beta</span>}
                  </span>
                </button>
              ))}
            </nav>
            {!sidebarCollapsed && (
              <div className="sidebar-history">
                <h2 className="sidebar-heading">Recent</h2>
                <ChatHistory
                  data={data}
                  selectedChatId={page === 'chats' ? selectedChatId : null}
                  onSelect={openChat}
                  searchRef={searchRef}
                />
              </div>
            )}
          </aside>
        )}
        <main className="main" id="main-content" ref={mainRef} tabIndex={-1}>
          {(loadFailed || saveError) && (
            <div className="persistence-error" role="alert">
              {loadFailed
                ? 'Juniper could not read its stored data, so changes in this session are not saved and the stored data was left unchanged.'
                : `Juniper could not save your latest changes: ${saveError}`}
            </div>
          )}
          {page === 'chats' && (
            <ChatScreen
              key={viewKey}
              data={data}
              update={update}
              conversation={selectedConversation}
              newChat={newChat}
              setNewChat={setNewChat}
              onMaterialize={setSelectedChatId}
              onDeleted={startNewChat}
              onOpenHistory={mobile ? () => setHistoryOpen(true) : undefined}
              onBrowseModels={() => navigate({ page: 'models', section: null })}
              mobile={mobile}
              touchPrimary={touchPrimary}
              onComposerFocus={setComposing}
            />
          )}
          {page === 'agent' && (
            <AgentScreen
              data={data}
              mobile={mobile}
              onBrowseModels={() => navigate({ page: 'models', section: null })}
            />
          )}
          {page === 'models' && (
            <ModelsScreen
              data={data}
              update={update}
              openSettings={(section) => navigate({ page: 'settings', section })}
            />
          )}
          {page === 'settings' && (
            <SettingsScreen
              data={data}
              update={update}
              section={nav.section}
              openSection={openSection}
              twoPane={!singlePaneSettings}
              route={route}
              currentConversation={selectedConversation}
              onReplayWelcome={() => setReplayingOnboarding(true)}
            />
          )}
        </main>
        {mobile && (
          <nav className="bottom-nav" aria-label="Primary">
            {NAV_ITEMS.map((item) => (
              <button
                key={item.page}
                className={page === item.page ? 'active' : ''}
                aria-current={page === item.page ? 'page' : undefined}
                onClick={() =>
                  navigate({
                    page: item.page,
                    section: item.page === 'settings' ? nav.section : null,
                  })
                }
              >
                <Icon name={item.icon} size={22} />
                <span>
                  {item.label}
                  {item.beta && <span className="visually-hidden"> (Beta)</span>}
                </span>
              </button>
            ))}
          </nav>
        )}
      </div>
      {mobile && (
        <Modal
          open={historyOpen}
          onClose={() => setHistoryOpen(false)}
          title="Chats"
          variant="drawer"
        >
          <button className="sidebar-new-chat" onClick={startNewChat}>
            <Icon name="plus" />
            <span>New chat</span>
          </button>
          <ChatHistory data={data} selectedChatId={selectedChatId} onSelect={openChat} />
        </Modal>
      )}
      {/* Wait for stored state so a returning user never sees onboarding flash open. */}
      <Onboarding
        open={hydrated && (!data.settings.onboardingComplete || replayingOnboarding)}
        onDone={completeOnboarding}
      />
    </>
  )
}
