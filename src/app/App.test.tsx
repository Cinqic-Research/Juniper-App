import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { serializeAssistant } from '../lib/assistant'
import {
  DEFAULT_SYSTEM_PROMPT,
  HISTORICAL_STOCK_JUNIPER_SYSTEM_PROMPTS,
  initialAppData,
  modelProfileFromDiscovery,
} from '../lib/defaults'
import { rc32StoredState } from '../test/fixtures'
import type { AppData } from '../types'
import App from './App'
import { chatTitle } from './ChatScreen'
import { modelFitLabel } from './model-labels'

const STORAGE_KEY = 'juniper.app-data.v1'

function buttonByText(container: ParentNode, text: string): HTMLButtonElement {
  const button = Array.from(container.querySelectorAll('button')).find(
    (item) => item.textContent?.trim() === text || item.getAttribute('aria-label') === text,
  )
  if (!(button instanceof HTMLButtonElement)) throw new Error(`Button not found: ${text}`)
  return button
}

function byLabel<T extends Element>(container: ParentNode, label: string): T {
  const element = container.querySelector(`[aria-label="${label}"]`)
  if (!element) throw new Error(`Element not found: ${label}`)
  return element as T
}

function setMedia(matches: (query: string) => boolean) {
  window.matchMedia = ((query: string) => ({
    matches: matches(query),
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  })) as unknown as typeof window.matchMedia
}

async function click(element: Element) {
  await act(async () => {
    ;(element as HTMLElement).click()
  })
}

describe('Juniper application shell', () => {
  let container: HTMLDivElement
  let root: Root
  const originalMatchMedia = window.matchMedia

  async function mount(data?: unknown) {
    if (root) act(() => root.unmount())
    if (data) localStorage.setItem(STORAGE_KEY, JSON.stringify(data))
    await act(async () => {
      root = createRoot(container)
      root.render(<App />)
    })
  }

  function stored(): AppData {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') as AppData
  }

  function settingsFor(settings: Partial<AppData['settings']> = {}) {
    const data = rc32StoredState()
    return { ...data, settings: { ...data.settings, reducedMotion: false, ...settings } }
  }

  async function openSettingsSection(label: string) {
    await click(buttonByText(document.querySelector('.sidebar-nav')!, 'Settings'))
    await click(buttonByText(document.querySelector('.settings-nav')!, label))
  }

  async function importAssistantFile(contents: string) {
    const input = container.querySelector<HTMLInputElement>('.builder input[type="file"]')
    if (!input) throw new Error('Assistant import input was not found')
    const file = new File([contents], 'assistant-backup.juniper', { type: 'application/json' })
    Object.defineProperty(file, 'text', { configurable: true, value: async () => contents })
    Object.defineProperty(input, 'files', { configurable: true, value: [file] })
    await act(async () => {
      input.dispatchEvent(new Event('change', { bubbles: true }))
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }

  beforeEach(async () => {
    localStorage.clear()
    window.history.replaceState(null, '')
    container = document.createElement('div')
    document.body.append(container)
    await mount()
  })

  afterEach(() => {
    act(() => root.unmount())
    container.remove()
    document.body.innerHTML = ''
    window.matchMedia = originalMatchMedia
  })

  it('shows onboarding with truthful runtime copy and a real way back to it', async () => {
    expect(document.body.textContent).toContain('Your AI. Your models. Your machine.')
    expect(document.body.textContent).toContain('No account. No telemetry.')
    await click(buttonByText(document.body, 'Continue'))
    await click(buttonByText(document.body, 'Continue'))
    expect(document.body.textContent).toContain('Browser development preview')
    expect(document.body.textContent).not.toContain('Ollama detected')
    await click(buttonByText(document.body, 'Continue'))
    await click(buttonByText(document.body, 'Enter Juniper'))
    expect(document.querySelector('[role="dialog"]')).toBeNull()
    expect(stored().settings.onboardingComplete).toBe(true)

    await openSettingsSection('General')
    await click(buttonByText(container, 'Show welcome'))
    expect(document.body.textContent).toContain('Your AI. Your models. Your machine.')
  })

  it('enters a usable no-model state without a fake production response', async () => {
    await click(buttonByText(document.body, 'Skip'))
    await click(buttonByText(container, 'New chat'))
    expect(container.textContent).toContain('No model selected')
    expect(container.textContent).toContain('choose one in Models')
    expect(byLabel<HTMLButtonElement>(container, 'Send message').disabled).toBe(true)
  })

  it('keeps exactly three primary destinations', async () => {
    await mount(settingsFor())
    const primary = Array.from(document.querySelectorAll('.sidebar-nav button')).map((button) =>
      button.textContent?.trim(),
    )
    expect(primary).toEqual(['Chats', 'Models', 'Settings'])
  })

  it('keeps every pre-redesign surface reachable from the new navigation', async () => {
    await mount(settingsFor())
    await click(buttonByText(document.querySelector('.sidebar-nav')!, 'Models'))
    expect(container.textContent).toContain('SmolLM2 135M Instruct')
    expect(container.textContent).toContain('From your connections')
    expect(container.textContent).toContain('Hosted reviewer model')
    expect(container.textContent).toContain('REMOTE')

    const expectations: Array<[string, string[]]> = [
      ['General', ['Assistant for new chats', 'Keyboard shortcuts']],
      [
        'Appearance',
        [
          'Color mode',
          'Accent color',
          'Font',
          'Density',
          'Conversation width',
          'Sidebar',
          'Message style',
        ],
      ],
      ['Accessibility', ['Contrast', 'Interface size', 'Animations']],
      ['Assistants', ['Your assistants', 'Juniper', 'Rowan', 'New assistant']],
      [
        'Models & runtime',
        ['Local engine', 'Download from Ollama', 'Import a GGUF file', 'Fit guidance'],
      ],
      ['Connections', ['Model providers', 'Ollama on this machine', 'Device Link', 'Preview only']],
      [
        'Tools & permissions',
        [
          'The permission boundary is on',
          'juniper-tool-protocol-v1',
          'Saved permissions',
          'Revoke',
          'Calculator',
        ],
      ],
      ['Memory', ['Curated memory', 'Prefers metric units.']],
      [
        'Privacy & data',
        [
          'Telemetry is off',
          'Current route',
          'Network tools',
          'Export data',
          'Clear chats',
          'Clear memory',
        ],
      ],
      [
        'Advanced',
        ['Developer mode', 'Runtime and process limits', 'MCP servers', 'Open diagnostics'],
      ],
      ['About', ['Apache License 2.0', 'SIL Open Font License 1.1']],
    ]
    for (const [section, texts] of expectations) {
      await openSettingsSection(section)
      for (const text of texts) expect(container.textContent, `${section}: ${text}`).toContain(text)
    }

    await openSettingsSection('Advanced')
    await click(buttonByText(container, 'Open diagnostics'))
    expect(container.textContent).toContain('Browser preview (development only)')
    expect(container.textContent).toContain('Provider capabilities')
    await click(buttonByText(container, 'Back to Advanced'))
    expect(container.querySelector('h1')?.textContent).not.toBe('Diagnostics')

    await openSettingsSection('Assistants')
    await click(buttonByText(container, 'Edit Rowan'))
    for (const text of [
      'Identity',
      'Personality',
      'Boundaries',
      'Advanced generation',
      'Import',
      'Export',
    ])
      expect(container.textContent).toContain(text)
  })

  it('keeps Juniper reserved when importing a Juniper backup into a custom assistant', async () => {
    await mount(settingsFor())
    await openSettingsSection('Assistants')
    await click(buttonByText(container, 'New assistant'))
    await click(buttonByText(container, 'Save'))
    const originalCustom = stored().assistants.find(
      (assistant) => assistant.name === 'New assistant',
    )!

    await click(buttonByText(container, 'Edit New assistant'))
    await importAssistantFile(
      serializeAssistant({
        ...initialAppData().assistants[0]!,
        name: 'Imported Juniper backup',
        systemPrompt: HISTORICAL_STOCK_JUNIPER_SYSTEM_PROMPTS[0],
        createdAt: '2024-01-02T03:04:05.000Z',
      }),
    )
    await click(buttonByText(container, 'Save'))

    const assistants = stored().assistants
    const custom = assistants.find((assistant) => assistant.name === 'Imported Juniper backup')!
    const juniper = assistants.find((assistant) => assistant.id === 'assistant-juniper')!
    expect(custom.id).toBe(originalCustom.id)
    expect(custom.createdAt).toBe(originalCustom.createdAt)
    expect(custom.systemPrompt).toBe(HISTORICAL_STOCK_JUNIPER_SYSTEM_PROMPTS[0])
    expect(juniper.name).toBe('Juniper')
    expect(juniper.systemPrompt).toBe(DEFAULT_SYSTEM_PROMPT)
  })

  it('restores a Juniper backup into the canonical Juniper editor', async () => {
    await mount(settingsFor())
    await openSettingsSection('Assistants')
    await click(buttonByText(container, 'Edit Juniper'))
    await importAssistantFile(
      serializeAssistant({
        ...initialAppData().assistants[0]!,
        name: 'Restored Juniper',
        systemPrompt: 'A restored Juniper backup prompt.',
        createdAt: '2024-01-02T03:04:05.000Z',
      }),
    )
    await click(buttonByText(container, 'Save'))

    const juniper = stored().assistants.find((assistant) => assistant.id === 'assistant-juniper')!
    expect(juniper.name).toBe('Restored Juniper')
    expect(juniper.createdAt).toBe('2024-01-02T03:04:05.000Z')
    expect(juniper.systemPrompt).toBe('A restored Juniper backup prompt.')
    expect(stored().assistants).toHaveLength(2)
  })

  it('shows each chat with its own assistant avatar and names the composer after it', async () => {
    await mount(settingsFor())
    const reviewItem = Array.from(container.querySelectorAll('.history-item')).find((item) =>
      item.textContent?.includes('Review the storage patch'),
    )!
    expect(reviewItem.querySelector('.assistant-avatar-glyph')?.textContent).toBe('R')
    const gardenItem = Array.from(container.querySelectorAll('.history-item')).find((item) =>
      item.textContent?.includes('Plan the garden beds'),
    )!
    expect(gardenItem.querySelector('.assistant-avatar-glyph')).toBeNull()

    await click(reviewItem)
    expect(container.querySelector('textarea')?.getAttribute('aria-label')).toBe('Message Rowan')
    expect(container.textContent).toContain('Model unavailable')
    expect(container.querySelector('[aria-label="Regenerate response"]')).toBeNull()
  })

  it('routes a chat through its own assistant model, not the default assistant', async () => {
    const data = settingsFor()
    Reflect.deleteProperty(data.conversations[1]!, 'modelProfileId')
    await mount(data)
    const reviewItem = Array.from(container.querySelectorAll('.history-item')).find((item) =>
      item.textContent?.includes('Review the storage patch'),
    )!
    await click(reviewItem)
    expect(container.querySelector('.model-pill')?.textContent).toContain('Hosted reviewer model')
  })

  it('switches the conversation model without rewriting historical attribution', async () => {
    const data = initialAppData()
    data.settings.onboardingComplete = true
    const provider = data.providers[0]!
    const modelA = modelProfileFromDiscovery(provider, 'future-model-a:7b', {
      displayName: 'Future Model A',
      status: 'ready',
      compatibilityStatus: 'chat-compatible',
    })
    const modelB = modelProfileFromDiscovery(provider, 'future-model-b:7b', {
      displayName: 'Future Model B',
      status: 'ready',
      compatibilityStatus: 'chat-compatible',
    })
    data.models = [modelA, modelB]
    data.assistants[0] = { ...data.assistants[0]!, modelProfileId: modelA.id }
    data.conversations = [
      {
        id: 'chat-model-switch',
        title: 'Model switch',
        assistantId: data.assistants[0]!.id,
        createdAt: '',
        updatedAt: '',
        modelProfileId: modelA.id,
        messages: [
          {
            id: 'historical-assistant-message',
            conversationId: 'chat-model-switch',
            role: 'assistant',
            modelId: modelA.id,
            providerId: provider.id,
            parts: [{ id: 'historical-text', type: 'text', text: 'Answered by model A.' }],
            createdAt: '',
          },
        ],
      },
    ]
    await mount(data)
    await click(container.querySelector('.history-item')!)
    await click(container.querySelector('.model-pill')!)
    const option = Array.from(
      document.querySelectorAll<HTMLInputElement>('input[name="conversation-model"]'),
    ).find((input) => input.closest('label')?.textContent?.includes('Future Model B'))!
    await click(option)

    expect(container.querySelector('.model-pill')?.textContent).toContain('Future Model B')
    expect(stored().conversations[0]!.modelProfileId).toBe(modelB.id)
    expect(stored().conversations[0]!.messages[0]!.modelId).toBe(modelA.id)
  })

  it('shows the selected conversation model in the privacy route', async () => {
    const data = initialAppData()
    data.settings.onboardingComplete = true
    const provider = data.providers[0]!
    const modelA = modelProfileFromDiscovery(provider, 'route-model-a', {
      displayName: 'Route Model A',
      status: 'ready',
      compatibilityStatus: 'chat-compatible',
    })
    const modelB = modelProfileFromDiscovery(provider, 'route-model-b', {
      displayName: 'Route Model B',
      status: 'ready',
      compatibilityStatus: 'chat-compatible',
    })
    data.models = [modelA, modelB]
    data.assistants[0] = { ...data.assistants[0]!, modelProfileId: modelA.id }
    data.conversations = [
      {
        id: 'chat-route',
        title: 'Route check',
        assistantId: data.assistants[0]!.id,
        createdAt: '',
        updatedAt: '',
        modelProfileId: modelB.id,
        messages: [],
      },
    ]
    await mount(data)
    await click(container.querySelector('.history-item')!)
    await openSettingsSection('Privacy & data')
    expect(container.textContent).toContain('Route Model B')
    expect(container.textContent).not.toContain('Route Model A')
    expect(container.textContent).toContain('ON DEVICE')
  })

  it('keeps conversation actions in an overflow menu and disables export for private chats', async () => {
    const data = settingsFor()
    Object.assign(data.conversations[0]!, { privateChat: true })
    await mount(data)
    await click(container.querySelector('.history-item')!)
    const header = container.querySelector('.chat-header')!
    expect(header.textContent).not.toContain('Export')
    expect(header.textContent).not.toContain('Rename')
    expect(header.textContent).toContain('Private')

    await click(byLabel(container, 'Chat options'))
    const exportItem = Array.from(container.querySelectorAll('[role="menuitem"]')).find((item) =>
      item.textContent?.includes('Export as Markdown'),
    )!
    expect(exportItem.getAttribute('aria-disabled')).toBe('true')
    expect(exportItem.textContent).toContain('Private chats can’t be exported')
  })

  it('asks for confirmation before deleting a chat', async () => {
    await mount(settingsFor())
    await click(container.querySelector('.history-item')!)
    const title = container.querySelector('.chat-title h1')!.textContent!

    const openDelete = async () => {
      await click(byLabel(container, 'Chat options'))
      const item = Array.from(container.querySelectorAll('[role="menuitem"]')).find((element) =>
        element.textContent?.includes('Delete chat'),
      )!
      await click(item)
    }

    await openDelete()
    const dialog = document.querySelector('[role="alertdialog"]')!
    expect(dialog.textContent).toContain('Delete this chat?')
    await click(buttonByText(dialog, 'Cancel'))
    expect(stored().conversations.map((chat) => chat.title)).toContain(title)

    await openDelete()
    await click(buttonByText(document.querySelector('[role="alertdialog"]')!, 'Delete chat'))
    expect(stored().conversations.map((chat) => chat.title)).not.toContain(title)
    expect(container.querySelector('.chat-title')?.textContent).not.toContain(title)
  })

  it('renames a chat through an in-app dialog', async () => {
    await mount(settingsFor())
    await click(container.querySelector('.history-item')!)
    await click(byLabel(container, 'Chat options'))
    await click(
      Array.from(container.querySelectorAll('[role="menuitem"]')).find((item) =>
        item.textContent?.includes('Rename'),
      )!,
    )
    const input = document.querySelector<HTMLInputElement>('[role="dialog"] input')!
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!
      setter.call(input, 'Raised beds, final')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await click(buttonByText(document.querySelector('[role="dialog"]')!, 'Rename'))
    expect(stored().conversations.find((chat) => chat.id === 'chat-plan')?.title).toBe(
      'Raised beds, final',
    )
  })

  it('puts reasoning, tool results, and model details behind progressive disclosure', async () => {
    await mount(settingsFor({ developerMode: false }))
    await click(container.querySelector('.history-item')!)
    const reply = container.querySelector('.message.assistant')!
    expect(reply.textContent).toContain('A simple layout')
    expect(reply.querySelectorAll('details:not([open])')).toHaveLength(2)
    expect(reply.textContent).not.toContain('Model: qwen3:8b')

    const details = byLabel<HTMLButtonElement>(reply, 'Response details')
    expect(details.getAttribute('aria-expanded')).toBe('false')
    await click(details)
    expect(details.getAttribute('aria-expanded')).toBe('true')
    expect(reply.textContent).toContain('Model: qwen3:8b')
    expect(reply.textContent).toContain('1,026 tokens')
    expect(byLabel(reply, 'Copy response')).toBeTruthy()
    expect(byLabel(reply, 'Regenerate response')).toBeTruthy()
    expect(container.querySelectorAll('[aria-label="Regenerate response"]')).toHaveLength(1)
  })

  it('always shows response details and timestamps when those settings are on', async () => {
    await mount(settingsFor({ showMessageDetails: true, showTimestamps: true } as never))
    await click(container.querySelector('.history-item')!)
    const reply = container.querySelector('.message.assistant')!
    expect(reply.textContent).toContain('Model: qwen3:8b')
    expect(reply.querySelector('time')).not.toBeNull()
    expect(reply.querySelector('[aria-label="Response details"]')).toBeNull()
  })

  it('describes keyboard sending on desktop only and never sends on Enter on touch devices', async () => {
    await mount(settingsFor())
    const desktopTextarea = container.querySelector('textarea')!
    const hintId = desktopTextarea.getAttribute('aria-describedby')
    expect(hintId && document.getElementById(hintId)?.textContent).toContain('Shift+Enter')

    setMedia((query) => query.includes('pointer: coarse') || query.includes('max-width: 760px'))
    await mount(settingsFor())
    const touchTextarea = container.querySelector('textarea')!
    expect(touchTextarea.getAttribute('aria-describedby')).toBeNull()
    expect(document.body.textContent).not.toContain('Shift')
    expect(container.querySelector('.bottom-nav')?.textContent).toContain('Settings')
    expect(container.querySelector('.sidebar')).toBeNull()
  })

  it('makes density, width, font, motion, and message style change the rendered document', async () => {
    await mount(settingsFor())
    await openSettingsSection('Appearance')
    const pick = async (group: string, option: string) => {
      const radio = Array.from(
        byLabel(container, group).querySelectorAll<HTMLInputElement>('input[type="radio"]'),
      ).find((input) => input.closest('label')?.textContent === option)!
      await click(radio)
    }
    const rootStyle = document.documentElement.style
    await pick('Density', 'Spacious')
    expect(document.documentElement.dataset.density).toBe('spacious')
    expect(rootStyle.getPropertyValue('--density')).toBe('1.2')
    await pick('Conversation width', 'Wide')
    expect(rootStyle.getPropertyValue('--conversation-width')).toBe('64rem')
    await pick('Message style', 'Minimal')
    expect(document.documentElement.dataset.messageStyle).toBe('minimal')
    await pick('Chat text size', 'Large')
    expect(rootStyle.getPropertyValue('--chat-font-size')).toBe('1.125rem')

    const font = container.querySelector<HTMLSelectElement>('select')!
    await act(async () => {
      font.value = 'dyslexic'
      font.dispatchEvent(new Event('change', { bubbles: true }))
    })
    expect(rootStyle.getPropertyValue('--font-ui')).toContain('OpenDyslexic')

    await openSettingsSection('Accessibility')
    await pick('Animations', 'Reduce')
    expect(document.documentElement.dataset.motion).toBe('reduced')
    await pick('Contrast', 'High')
    expect(document.documentElement.dataset.contrast).toBe('high')

    const settings = stored().settings
    expect(settings).toMatchObject({
      density: 'spacious',
      conversationWidth: 'wide',
      messageStyle: 'minimal',
      chatTextSize: 'large',
      fontFamily: 'dyslexic',
      motion: 'reduced',
      contrast: 'high',
    })
  })

  it('rejects an invalid custom accent and keeps the previous one', async () => {
    await mount(settingsFor({ accent: '#3B82F6' }))
    await openSettingsSection('Appearance')
    const input = byLabel<HTMLInputElement>(container, 'Custom accent color hex value')
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!
      setter.call(input, 'red;x')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => {
      input.form!.requestSubmit()
    })
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('#RRGGBB')
    expect(input.getAttribute('aria-invalid')).toBe('true')
    expect(stored().settings.accent).toBe('#3B82F6')
    expect(document.documentElement.style.getPropertyValue('--accent')).toBe('#3B82F6')

    await openSettingsSection('Appearance')
    const coral = Array.from(
      container.querySelectorAll<HTMLInputElement>('input[name="accent"]'),
    ).find((radio) => radio.closest('label')?.textContent === 'Coral')!
    await click(coral)
    expect(stored().settings.accent).toBe('#F2665A')
    await click(buttonByText(container, 'Reset appearance'))
    expect(stored().settings.accent).toBe('#32CD32')
  })

  it('remembers the sidebar preference and exposes an accessible toggle', async () => {
    await mount(settingsFor())
    const toggle = byLabel<HTMLButtonElement>(container, 'Collapse sidebar')
    expect(toggle.getAttribute('aria-expanded')).toBe('true')
    await click(toggle)
    expect(stored().settings.sidebar).toBe('collapsed')
    expect(container.querySelector('.app-frame')?.getAttribute('data-sidebar')).toBe('collapsed')
    expect(container.querySelector('.sidebar-history')).toBeNull()
    expect(byLabel(container, 'Expand sidebar').getAttribute('aria-expanded')).toBe('false')
    expect(buttonByText(container.querySelector('.sidebar-nav')!, 'Models')).toBeTruthy()
  })

  it('persists the assistant used for new chats', async () => {
    await mount(settingsFor())
    await openSettingsSection('Assistants')
    await click(buttonByText(container, 'Use for new chats'))
    expect(stored().settings.defaultAssistantId).toBe('assistant-coder')
    await click(buttonByText(container, 'New chat'))
    expect(container.querySelector('textarea')?.getAttribute('aria-label')).toBe('Message Rowan')
  })

  it('sets the tool policy for the chosen assistant rather than the first one', async () => {
    await mount(settingsFor())
    await openSettingsSection('Tools & permissions')
    const select = Array.from(container.querySelectorAll<HTMLSelectElement>('select'))[1]!
    await act(async () => {
      select.value = 'disabled'
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    const assistants = stored().assistants
    expect(assistants.find((item) => item.id === 'assistant-coder')?.toolPolicy).toBe('disabled')
    expect(assistants.find((item) => item.id === 'assistant-juniper')?.toolPolicy).toBe('ask')
  })

  it('streams a browser-preview reply into a new chat and saves it', async () => {
    const data = settingsFor()
    await mount(data)
    const textarea = container.querySelector('textarea')!
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!
      setter.call(textarea, 'who are you?')
      textarea.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await click(byLabel(container, 'Send message'))
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1500))
    })
    const chats = stored().conversations
    const created = chats.find((chat) => chat.title === 'who are you?')!
    expect(created.assistantId).toBe('assistant-juniper')
    expect(created.messages).toHaveLength(2)
    expect(created.messages[1]!.parts[0]!.text).toContain('I’m Juniper')
    expect(container.querySelector('.chat-title h1')?.textContent).toBe('who are you?')
    expect(container.querySelector('[role="status"]')?.textContent).toContain('Juniper replied.')
  })

  it('starts a fresh chat when the open chat is cleared from Privacy', async () => {
    await mount(settingsFor())
    await click(container.querySelector('.history-item')!)
    await openSettingsSection('Privacy & data')
    await click(buttonByText(container, 'Clear chats'))
    await click(buttonByText(document.querySelector('[role="alertdialog"]')!, 'Clear chats'))
    expect(stored().conversations).toHaveLength(0)

    await click(buttonByText(document.querySelector('.sidebar-nav')!, 'Chats'))
    expect(container.querySelector('.chat-title h1')?.textContent).not.toBe('Plan the garden beds')
    const textarea = container.querySelector('textarea')!
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!
      setter.call(textarea, 'who are you?')
      textarea.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await click(byLabel(container, 'Send message'))
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1500))
    })
    expect(stored().conversations.map((chat) => chat.title)).toEqual(['who are you?'])
    expect(stored().conversations[0]!.messages).toHaveLength(2)
  })

  it('titles new chats at a word boundary without attachment markers', () => {
    expect(chatTitle('who are you?')).toBe('who are you?')
    expect(chatTitle('Explain in two short paragraphs why raised garden beds drain well.')).toBe(
      'Explain in two short paragraphs why raised…',
    )
    expect(chatTitle('Summarise this\n\n[Attached: notes.md]')).toBe('Summarise this')
    expect(chatTitle('[Attached: notes.md]')).toBe('Attached files')
  })

  it('returns from a phone Settings section without duplicating history', async () => {
    setMedia((query) => query.includes('max-width'))
    await mount(settingsFor())
    await click(buttonByText(container.querySelector('.bottom-nav')!, 'Settings'))
    await click(
      Array.from(container.querySelectorAll('.settings-nav-item')).find((item) =>
        item.textContent?.startsWith('Appearance'),
      )!,
    )
    expect(container.querySelector('h1')?.textContent).toBe('Appearance')
    await click(buttonByText(container, 'Back to Settings'))
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 60))
    })
    expect(container.querySelector('h1')?.textContent).toBe('Settings')
    await act(async () => {
      window.history.back()
      await new Promise((resolve) => setTimeout(resolve, 60))
    })
    expect(container.querySelector('.chat-screen')).not.toBeNull()
  })

  it('keeps the model name when an Ollama pull fails', async () => {
    await mount(settingsFor())
    await openSettingsSection('Models & runtime')
    const input = container.querySelector<HTMLInputElement>('.inline-field input')!
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!
      setter.call(input, 'qwen3:06b')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await click(buttonByText(container, 'Download'))
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    expect(container.textContent).toContain('Model downloads require')
    expect(input.value).toBe('qwen3:06b')
  })

  it('does not claim a copy succeeded when nothing was copied', async () => {
    await mount(settingsFor())
    await click(container.querySelector('.history-item')!)
    const copy = byLabel<HTMLButtonElement>(
      container.querySelector('.message.assistant')!,
      'Copy response',
    )
    await click(copy)
    await act(async () => {
      await Promise.resolve()
    })
    expect(container.querySelector('[aria-label="Copied"]')).toBeNull()
  })

  it('labels model fit as an estimate and stays unknown without runtime data', () => {
    const provider = initialAppData().providers[0]!
    const model = modelProfileFromDiscovery(provider, 'fit-model', { fileSizeBytes: 1024 })
    expect(modelFitLabel(model, null)).toBe('Unknown')
    expect(modelFitLabel(model, '16 GB')).toBe('Excellent')
    expect(modelFitLabel(model, '1 KB')).toBe('Not recommended')
  })
})
