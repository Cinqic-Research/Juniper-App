import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { initialAppData } from '../lib/defaults'
import type { AppData, ProviderProfile } from '../types'

// Exercises the native (Tauri) startup path, which the browser-mode shell tests
// cannot reach because `runningInTauri` is fixed when the runtime module loads.
// rc.31 shipped a blank desktop window: stored state with an enabled Ollama
// provider made startup model discovery throw a ReferenceError inside a React
// state update, which unmounted the whole application.

// Present Juniper as the Android app so Android-only behaviour is exercised.
Object.defineProperty(window.navigator, 'userAgent', {
  configurable: true,
  value: 'Mozilla/5.0 (Linux; Android 11; sdk_gphone_x86_64) AppleWebKit/537.36 Juniper-test',
})

type InvokeHandler = (command: string, args?: Record<string, unknown>) => unknown
type Invocation = { command: string; args?: Record<string, unknown> }

const ollamaProvider: ProviderProfile = {
  id: 'ollama-local',
  name: 'Ollama on this machine',
  kind: 'ollama',
  baseUrl: 'http://127.0.0.1:11434',
  locality: 'local',
  transportLocation: 'on-device',
  enabled: true,
  status: 'connected',
  capabilities: initialAppData().providers[0]!.capabilities,
}

function storedStateWithOllama(): AppData {
  const data = initialAppData()
  return {
    ...data,
    settings: { ...data.settings, onboardingComplete: true },
    providers: [ollamaProvider],
    models: [],
  }
}

describe('Juniper native startup', () => {
  let container: HTMLDivElement
  let root: Root
  let commands: string[]
  let invocations: Invocation[]
  let uncaught: unknown[]

  function installTauri(handler: InvokeHandler) {
    commands = []
    invocations = []
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      value: {
        transformCallback: () => 0,
        invoke: async (command: string, args?: Record<string, unknown>) => {
          commands.push(command)
          invocations.push({ command, args })
          return handler(command, args)
        },
      },
    })
  }

  async function mountApp() {
    const { default: App } = await import('./App')
    container = document.createElement('div')
    document.body.append(container)
    uncaught = []
    await act(async () => {
      root = createRoot(container, { onUncaughtError: (error) => uncaught.push(error) })
      root.render(<App />)
    })
    for (let tick = 0; tick < 10; tick += 1) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5))
      })
    }
  }

  beforeEach(() => {
    localStorage.clear()
  })

  afterEach(async () => {
    act(() => root?.unmount())
    container?.remove()
    document.body.innerHTML = ''
    // Unmounting closes overlays, which pops their history entries asynchronously.
    await new Promise((resolve) => setTimeout(resolve, 50))
    Reflect.deleteProperty(window, '__TAURI_INTERNALS__')
  })

  it('stays rendered after discovering models from a stored Ollama provider', async () => {
    installTauri((command) => {
      switch (command) {
        case 'load_app_data':
          return storedStateWithOllama()
        case 'health_check':
          return 'Ollama 0.33.2'
        case 'list_models':
          return [
            { modelId: 'qwen3:8b', displayName: 'qwen3:8b', sizeBytes: 5_225_388_164 },
            { modelId: 'qwen3:0.6b', displayName: 'qwen3:0.6b', sizeBytes: 522_653_767 },
          ]
        default:
          return null
      }
    })

    await mountApp()

    expect(uncaught).toEqual([])
    expect(commands).toContain('list_models')
    expect(container.textContent).toContain('Chats')
    const lastSave = invocations.filter((call) => call.command === 'save_app_data').at(-1)
    const savedModels = JSON.stringify(lastSave?.args ?? {})
    expect(savedModels).toContain('ollama-local:qwen3:8b')
    expect(savedModels).toContain('ollama-local:qwen3:0.6b')
  })

  it('never overwrites stored state with defaults when loading it failed', async () => {
    installTauri((command) => {
      if (command === 'load_app_data') throw new Error('DATABASE_ERROR: disk I/O error')
      return null
    })

    await mountApp()

    expect(uncaught).toEqual([])
    const notice = document.querySelector('[role="alertdialog"]')
    expect(notice?.textContent).toContain('DATABASE_ERROR: disk I/O error')
    expect(commands).not.toContain('save_app_data')
    expect(container.textContent).toContain('stored data was left unchanged')
  })

  it('shows a save failure instead of silently dropping changes', async () => {
    installTauri((command) => {
      if (command === 'load_app_data') return storedStateWithOllama()
      if (command === 'save_app_data')
        throw new Error('DATABASE_ERROR: FOREIGN KEY constraint failed')
      return null
    })

    await mountApp()

    expect(uncaught).toEqual([])
    expect(commands).toContain('save_app_data')
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      'could not save your latest changes: DATABASE_ERROR: FOREIGN KEY constraint failed',
    )
  })

  it('reports frontend readiness to the native host once stored state is hydrated', async () => {
    installTauri(() => null)

    await mountApp()

    expect(uncaught).toEqual([])
    expect(commands).toContain('frontend_ready')
    expect(commands.indexOf('frontend_ready')).toBeGreaterThan(commands.indexOf('load_app_data'))
  })

  it('applies Android system bar and keyboard insets reported by the native host', async () => {
    installTauri((command) =>
      command === 'window_insets' ? { top: 24, right: 0, bottom: 48, left: 0, keyboard: 0 } : null,
    )

    await mountApp()

    const style = document.documentElement.style
    expect(commands).toContain('window_insets')
    expect(style.getPropertyValue('--native-inset-top')).toBe('24px')
    expect(style.getPropertyValue('--native-inset-bottom')).toBe('48px')
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('juniper-window-insets', {
          detail: { top: 24, right: 0, bottom: 48, left: 0, keyboard: 310 },
        }),
      )
    })
    expect(style.getPropertyValue('--native-keyboard')).toBe('310px')
  })

  it('takes over Android back only while Juniper has somewhere to go back to', async () => {
    installTauri((command) => (command === 'load_app_data' ? storedStateWithOllama() : null))
    await mountApp()
    const registrations = () =>
      invocations.filter((call) => call.command === 'plugin:app|register_listener')
    expect(registrations()).toHaveLength(0)

    const settings = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent?.trim() === 'Settings',
    )!
    await act(async () => {
      settings.click()
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
    expect(registrations()).toHaveLength(1)
    expect(registrations()[0]!.args?.event).toBe('back-button')

    await act(async () => {
      window.history.back()
      await new Promise((resolve) => setTimeout(resolve, 50))
    })
    expect(commands).toContain('plugin:app|remove_listener')
    expect(container.querySelector('.chat-screen')).not.toBeNull()
  })

  it('uses the Android WebView file chooser and hides desktop-only GGUF import', async () => {
    installTauri((command) => (command === 'load_app_data' ? storedStateWithOllama() : null))
    await mountApp()

    const attach = container.querySelector<HTMLButtonElement>('button[aria-label="Attach a file"]')
    const fileInput = container.querySelector<HTMLInputElement>('input[type="file"]')
    expect(attach).not.toBeNull()
    expect(fileInput).not.toBeNull()
    const openWebViewChooser = vi.spyOn(fileInput!, 'click').mockImplementation(() => {})
    await act(async () => attach!.click())
    expect(openWebViewChooser).toHaveBeenCalledOnce()
    expect(commands).not.toContain('pick_attachment')
    const mobileFile = new File(['Android attachment text'], 'mobile.txt', {
      type: 'text/plain',
    })
    Object.defineProperty(mobileFile, 'text', {
      value: async () => 'Android attachment text',
    })
    Object.defineProperty(fileInput!, 'files', { configurable: true, value: [mobileFile] })
    await act(async () => {
      fileInput!.dispatchEvent(new Event('change', { bubbles: true }))
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(container.querySelector('.attachment-chips')?.textContent).toContain('mobile.txt')

    const settings = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Settings',
    )!
    await act(async () => settings.click())
    const connections = Array.from(
      container.querySelectorAll<HTMLButtonElement>('.settings-nav-item'),
    ).find((button) => button.textContent?.includes('Connections'))!
    await act(async () => connections.click())
    expect(container.textContent).not.toContain('Import a GGUF file')
    openWebViewChooser.mockRestore()
  })

  it('does not double the keyboard offset when the WebView already shrank', async () => {
    const { uncoveredKeyboardHeight } = await import('./App')
    const setHeight = (height: number) =>
      Object.defineProperty(window, 'innerHeight', { configurable: true, value: height })
    setHeight(732)
    expect(uncoveredKeyboardHeight(0)).toBe(0)
    expect(uncoveredKeyboardHeight(343)).toBe(343)
    setHeight(389)
    expect(uncoveredKeyboardHeight(343)).toBe(0)
    setHeight(732)
    expect(uncoveredKeyboardHeight(0)).toBe(0)
  })
})
