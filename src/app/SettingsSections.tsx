import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { colorTokens, resolveTheme, resolveHighContrast } from '../lib/appearance'
import { builtinTools } from '../lib/defaults'
import { getDiagnostics, getRuntimeLogs } from '../lib/runtime'
import {
  ACCENT_PALETTE,
  ACCESSIBILITY_KEYS,
  APPEARANCE_KEYS,
  MAX_FONT_SCALE,
  MIN_FONT_SCALE,
  parseHexColor,
  resetSettings,
} from '../lib/settings'
import { withoutPrivateChats } from '../lib/storage'
import type {
  AppData,
  AppSettings,
  ProviderProfile,
  RuntimeLogEntry,
  SettingsSection,
  ToolDefinition,
} from '../types'
import { AssistantAvatar } from './branding'
import { Icon } from './icons'
import { assistantFor, defaultAssistantFor, type ConversationRoute } from './model-labels'
import { useDialogs } from './overlays'
import {
  LocationBadge,
  Row,
  Section,
  Segmented,
  Switch,
  download,
  labelExecutionLocation,
  now,
  uid,
  useTouchPrimary,
} from './ui'

type Update = (change: (current: AppData) => AppData) => void

function useSettingsUpdater(update: Update) {
  return function set<K extends keyof AppSettings>(key: K, value: AppSettings[K]) {
    update((current) => ({ ...current, settings: { ...current.settings, [key]: value } }))
  }
}

// ---------------------------------------------------------------------------
// General
// ---------------------------------------------------------------------------

export function GeneralSettings({
  data,
  update,
  onReplayWelcome,
}: {
  data: AppData
  update: Update
  onReplayWelcome: () => void
}) {
  const set = useSettingsUpdater(update)
  const touch = useTouchPrimary()
  const modifier =
    typeof navigator !== 'undefined' && /mac/i.test(navigator.platform) ? '⌘' : 'Ctrl'
  return (
    <>
      <Section title="Chats">
        <Row
          label="Assistant for new chats"
          description="You can also pick an assistant at the top of a new chat."
          control={({ id, describedBy }) => (
            <select
              id={id}
              aria-describedby={describedBy}
              value={defaultAssistantFor(data).id}
              onChange={(event) => set('defaultAssistantId', event.target.value)}
            >
              {data.assistants.map((assistant) => (
                <option key={assistant.id} value={assistant.id}>
                  {assistant.name}
                </option>
              ))}
            </select>
          )}
        />
      </Section>
      <Section title="Welcome">
        <div className="setting-row">
          <div className="setting-text">
            <span className="label">Welcome tour</span>
            <small>See the introduction to Juniper again.</small>
          </div>
          <div className="setting-control">
            <button className="button secondary" onClick={onReplayWelcome}>
              Show welcome
            </button>
          </div>
        </div>
      </Section>
      {!touch && (
        <Section title="Keyboard shortcuts">
          <dl className="shortcut-list">
            <div>
              <dt>Send message</dt>
              <dd>
                <kbd>Enter</kbd>
              </dd>
            </div>
            <div>
              <dt>New line</dt>
              <dd>
                <kbd>Shift</kbd> + <kbd>Enter</kbd>
              </dd>
            </div>
            <div>
              <dt>New chat</dt>
              <dd>
                <kbd>{modifier}</kbd> + <kbd>Shift</kbd> + <kbd>O</kbd>
              </dd>
            </div>
            <div>
              <dt>Search chats</dt>
              <dd>
                <kbd>{modifier}</kbd> + <kbd>K</kbd>
              </dd>
            </div>
            <div>
              <dt>Show or hide sidebar</dt>
              <dd>
                <kbd>{modifier}</kbd> + <kbd>Shift</kbd> + <kbd>S</kbd>
              </dd>
            </div>
            <div>
              <dt>Close a menu or dialog</dt>
              <dd>
                <kbd>Esc</kbd>
              </dd>
            </div>
          </dl>
        </Section>
      )}
    </>
  )
}

// ---------------------------------------------------------------------------
// Appearance
// ---------------------------------------------------------------------------

export function AppearanceSettings({ data, update }: { data: AppData; update: Update }) {
  const settings = data.settings
  const set = useSettingsUpdater(update)
  const [customDraft, setCustomDraft] = useState(settings.accent)
  const [customError, setCustomError] = useState<string | null>(null)
  useEffect(() => setCustomDraft(settings.accent), [settings.accent])
  const paletteMatch = ACCENT_PALETTE.find((item) => item.value === settings.accent)
  const accentChoice =
    settings.accentMode === 'neutral' ? 'neutral' : paletteMatch ? paletteMatch.value : 'custom'
  const theme = resolveTheme(settings)
  const tokens = colorTokens({ settings, theme, highContrast: resolveHighContrast(settings) })
  const adjusted = settings.accentMode === 'color' && tokens['--accent-ink'] !== tokens['--accent']

  function applyCustom(value: string) {
    const parsed = parseHexColor(value)
    if (!parsed) {
      setCustomError('Enter a color as #RRGGBB, for example #3B82F6.')
      return
    }
    setCustomError(null)
    update((current) => ({
      ...current,
      settings: { ...current.settings, accent: parsed, accentMode: 'color' },
    }))
  }

  return (
    <>
      <div className="appearance-preview" aria-hidden="true">
        <div className="preview-user">Can you help me plan my week?</div>
        <div className="preview-assistant">
          <span className="preview-name">Juniper</span>
          <p>
            Of course. Let’s start with what can’t move, then fit the rest around it.{' '}
            <span className="preview-link">Open plan</span>
          </p>
        </div>
        <div className="preview-actions">
          <span className="preview-button">Send</span>
          <span className="preview-focus">Focused</span>
        </div>
      </div>

      <Section title="Theme">
        <Row
          label="Color mode"
          group
          control={({ describedBy }) => (
            <Segmented<AppSettings['theme']>
              label="Color mode"
              describedBy={describedBy}
              value={settings.theme}
              onChange={(value) => set('theme', value)}
              options={[
                { value: 'system', label: 'System' },
                { value: 'light', label: 'Light' },
                { value: 'dark', label: 'Dark' },
              ]}
            />
          )}
          stacked
        />
        <div className="setting-row stacked">
          <div className="setting-text">
            <span className="label" id="accent-label">
              Accent color
            </span>
            <small>
              Used for buttons, links, and focus.
              {adjusted && ' Text and focus rings use a deeper shade so they stay readable.'}
            </small>
          </div>
          <div className="setting-control">
            <div className="swatches" role="radiogroup" aria-labelledby="accent-label">
              {ACCENT_PALETTE.map((item) => (
                <label key={item.value} className="swatch" title={item.name}>
                  <input
                    type="radio"
                    name="accent"
                    checked={accentChoice === item.value}
                    onChange={() =>
                      update((current) => ({
                        ...current,
                        settings: { ...current.settings, accent: item.value, accentMode: 'color' },
                      }))
                    }
                  />
                  <span className="swatch-color" style={{ background: item.value }} />
                  <span className="visually-hidden">{item.name}</span>
                </label>
              ))}
              <label className="swatch" title="Neutral">
                <input
                  type="radio"
                  name="accent"
                  checked={accentChoice === 'neutral'}
                  onChange={() => set('accentMode', 'neutral')}
                />
                <span className="swatch-color neutral" />
                <span className="visually-hidden">Neutral (no color)</span>
              </label>
              <label className="swatch" title="Custom">
                <input
                  type="radio"
                  name="accent"
                  checked={accentChoice === 'custom'}
                  onChange={() => applyCustom(customDraft)}
                />
                <span className="swatch-color custom" />
                <span className="visually-hidden">Custom color</span>
              </label>
            </div>
            <form
              className="custom-accent"
              onSubmit={(event) => {
                event.preventDefault()
                applyCustom(customDraft)
              }}
            >
              <input
                type="color"
                aria-label="Pick a custom accent color"
                value={parseHexColor(customDraft)?.toLowerCase() ?? '#32cd32'}
                onChange={(event) => {
                  setCustomDraft(event.target.value.toUpperCase())
                  applyCustom(event.target.value)
                }}
              />
              <input
                className="hex-input"
                aria-label="Custom accent color hex value"
                aria-invalid={customError ? true : undefined}
                aria-describedby={customError ? 'accent-error' : undefined}
                value={customDraft}
                maxLength={7}
                spellCheck={false}
                onChange={(event) => setCustomDraft(event.target.value)}
                onBlur={() => {
                  if (customDraft !== settings.accent) applyCustom(customDraft)
                }}
              />
              <button type="submit" className="button ghost">
                Apply
              </button>
            </form>
            {customError && (
              <small id="accent-error" className="field-error" role="alert">
                {customError}
              </small>
            )}
          </div>
        </div>
      </Section>

      <Section title="Text">
        <Row
          label="Font"
          description="Every font ships with Juniper; nothing is downloaded."
          control={({ id, describedBy }) => (
            <select
              id={id}
              aria-describedby={describedBy}
              value={settings.fontFamily}
              onChange={(event) =>
                set('fontFamily', event.target.value as AppSettings['fontFamily'])
              }
            >
              <option value="juniper">Juniper default (Inter)</option>
              <option value="system">System font</option>
              <option value="legible">Atkinson Hyperlegible (high legibility)</option>
              <option value="dyslexic">OpenDyslexic</option>
            </select>
          )}
        />
        <Row
          label="Chat text size"
          group
          control={({ describedBy }) => (
            <Segmented<AppSettings['chatTextSize']>
              label="Chat text size"
              describedBy={describedBy}
              value={settings.chatTextSize}
              onChange={(value) => set('chatTextSize', value)}
              options={[
                { value: 'small', label: 'Small' },
                { value: 'medium', label: 'Medium' },
                { value: 'large', label: 'Large' },
              ]}
            />
          )}
          stacked
        />
        <Row
          label="Line spacing"
          group
          control={({ describedBy }) => (
            <Segmented<AppSettings['lineSpacing']>
              label="Line spacing"
              describedBy={describedBy}
              value={settings.lineSpacing}
              onChange={(value) => set('lineSpacing', value)}
              options={[
                { value: 'compact', label: 'Compact' },
                { value: 'standard', label: 'Standard' },
                { value: 'relaxed', label: 'Relaxed' },
              ]}
            />
          )}
          stacked
        />
      </Section>

      <Section title="Layout">
        <Row
          label="Density"
          description="Spacing and control size throughout Juniper."
          group
          control={({ describedBy }) => (
            <Segmented<AppSettings['density']>
              label="Density"
              describedBy={describedBy}
              value={settings.density}
              onChange={(value) => set('density', value)}
              options={[
                { value: 'compact', label: 'Compact' },
                { value: 'comfortable', label: 'Comfortable' },
                { value: 'spacious', label: 'Spacious' },
              ]}
            />
          )}
          stacked
        />
        <Row
          label="Conversation width"
          group
          control={({ describedBy }) => (
            <Segmented<AppSettings['conversationWidth']>
              label="Conversation width"
              describedBy={describedBy}
              value={settings.conversationWidth}
              onChange={(value) => set('conversationWidth', value)}
              options={[
                { value: 'narrow', label: 'Narrow' },
                { value: 'balanced', label: 'Balanced' },
                { value: 'wide', label: 'Wide' },
              ]}
            />
          )}
          stacked
        />
        <Row
          label="Sidebar"
          description="Desktop only. Auto collapses the sidebar in narrower windows."
          group
          control={({ describedBy }) => (
            <Segmented<AppSettings['sidebar']>
              label="Sidebar"
              describedBy={describedBy}
              value={settings.sidebar}
              onChange={(value) => set('sidebar', value)}
              options={[
                { value: 'auto', label: 'Auto' },
                { value: 'expanded', label: 'Expanded' },
                { value: 'collapsed', label: 'Collapsed' },
              ]}
            />
          )}
          stacked
        />
        <Row
          label="Message style"
          description="Bubbles put your messages in a bubble; Minimal keeps both sides flat."
          group
          control={({ describedBy }) => (
            <Segmented<AppSettings['messageStyle']>
              label="Message style"
              describedBy={describedBy}
              value={settings.messageStyle}
              onChange={(value) => set('messageStyle', value)}
              options={[
                { value: 'bubbles', label: 'Bubbles' },
                { value: 'minimal', label: 'Minimal' },
              ]}
            />
          )}
          stacked
        />
        <Row
          label="Show timestamps"
          control={({ id }) => (
            <Switch
              id={id}
              checked={settings.showTimestamps}
              onChange={(value) => set('showTimestamps', value)}
            />
          )}
        />
        <Row
          label="Always show response details"
          description="Model name and usage under each reply. Otherwise use the details button."
          control={({ id, describedBy }) => (
            <Switch
              id={id}
              describedBy={describedBy}
              checked={settings.showMessageDetails}
              onChange={(value) => set('showMessageDetails', value)}
            />
          )}
        />
      </Section>

      <div className="section-footer">
        <button
          className="button ghost"
          onClick={() =>
            update((current) => ({
              ...current,
              settings: resetSettings(current.settings, APPEARANCE_KEYS),
            }))
          }
        >
          Reset appearance
        </button>
      </div>
    </>
  )
}

// ---------------------------------------------------------------------------
// Accessibility
// ---------------------------------------------------------------------------

export function AccessibilitySettings({
  data,
  update,
  openSection,
}: {
  data: AppData
  update: Update
  openSection: (section: SettingsSection) => void
}) {
  const settings = data.settings
  const set = useSettingsUpdater(update)
  return (
    <>
      <Section title="Vision">
        <Row
          label="Contrast"
          description="High contrast strengthens text, borders, and focus rings. System follows your device."
          group
          control={({ describedBy }) => (
            <Segmented<AppSettings['contrast']>
              label="Contrast"
              describedBy={describedBy}
              value={settings.contrast}
              onChange={(value) => set('contrast', value)}
              options={[
                { value: 'system', label: 'System' },
                { value: 'standard', label: 'Standard' },
                { value: 'high', label: 'High' },
              ]}
            />
          )}
          stacked
        />
        <Row
          label={`Interface size · ${Math.round(settings.fontScale * 100)}%`}
          description="Scales text and controls across Juniper."
          control={({ id, describedBy }) => (
            <input
              id={id}
              aria-describedby={describedBy}
              type="range"
              min={MIN_FONT_SCALE}
              max={MAX_FONT_SCALE}
              step="0.05"
              value={settings.fontScale}
              aria-valuetext={`${Math.round(settings.fontScale * 100)} percent`}
              onChange={(event) => set('fontScale', Number(event.target.value))}
            />
          )}
          stacked
        />
        <p className="muted small">
          Legible and dyslexia-friendly fonts, chat text size, and line spacing are in{' '}
          <button className="link-button" onClick={() => openSection('appearance')}>
            Appearance
          </button>
          .
        </p>
      </Section>
      <Section title="Motion">
        <Row
          label="Animations"
          description="System follows your device’s reduced-motion setting."
          group
          control={({ describedBy }) => (
            <Segmented<AppSettings['motion']>
              label="Animations"
              describedBy={describedBy}
              value={settings.motion}
              onChange={(value) => set('motion', value)}
              options={[
                { value: 'system', label: 'System' },
                { value: 'reduced', label: 'Reduce' },
                { value: 'full', label: 'Allow' },
              ]}
            />
          )}
          stacked
        />
      </Section>
      <div className="section-footer">
        <button
          className="button ghost"
          onClick={() =>
            update((current) => ({
              ...current,
              settings: resetSettings(current.settings, ACCESSIBILITY_KEYS),
            }))
          }
        >
          Reset accessibility settings
        </button>
      </div>
    </>
  )
}

// ---------------------------------------------------------------------------
// Tools & permissions
// ---------------------------------------------------------------------------

function riskLabel(risk: ToolDefinition['risk']): string {
  switch (risk) {
    case 'automatic-safe':
      return 'Runs automatically'
    case 'user-data-read':
      return 'Reads your data · asks first'
    case 'user-data-write':
      return 'Changes your data · asks first'
    case 'filesystem-read':
      return 'Reads attached files · asks first'
    case 'network':
      return 'Uses the network · asks first'
    case 'external-process':
      return 'Runs a program · asks first'
    default:
      return 'Sensitive · asks first'
  }
}

export function ToolsSettings({ data, update }: { data: AppData; update: Update }) {
  return (
    <>
      <div className="callout">
        <Icon name="shield" />
        <div>
          <strong>The permission boundary is on</strong>
          <p>
            Models can only request tools. Juniper’s host runtime runs them and writes the real
            results. Attached files, model output, MCP results, and imported assistants are treated
            as untrusted and can’t grant permissions.
          </p>
        </div>
      </div>

      <Section
        title="Tool access by assistant"
        description="Safe tools can run automatically. Tools that read or change your data always ask first when allowed."
      >
        {data.assistants.map((assistant) => (
          <Row
            key={assistant.id}
            label={assistant.name}
            control={({ id }) => (
              <select
                id={id}
                value={assistant.toolPolicy}
                onChange={(event) =>
                  update((current) => ({
                    ...current,
                    assistants: current.assistants.map((item) =>
                      item.id === assistant.id
                        ? {
                            ...item,
                            toolPolicy: event.target.value as typeof assistant.toolPolicy,
                            updatedAt: now(),
                          }
                        : item,
                    ),
                  }))
                }
              >
                <option value="ask">Ask before user-data tools</option>
                <option value="safe-automatic">Safe automatic tools only</option>
                <option value="disabled">Tools disabled</option>
              </select>
            )}
          />
        ))}
      </Section>

      <Section
        title="Saved permissions"
        description="Revoke chat- or assistant-wide access at any time."
      >
        {data.permissions.length === 0 ? (
          <p className="muted">No saved permissions.</p>
        ) : (
          <ul className="grant-list">
            {data.permissions.map((grant) => {
              const tool = builtinTools.find((item) => item.name === grant.toolName)
              const chat = data.conversations.find((item) => item.id === grant.conversationId)
              return (
                <li key={grant.id} className="grant-row">
                  <div>
                    <strong>{tool?.displayName ?? grant.toolName}</strong>
                    <small>
                      {assistantFor(data, grant.assistantId).name} ·{' '}
                      {grant.scope === 'assistant'
                        ? 'every chat'
                        : `chat “${chat?.title ?? 'removed chat'}”`}
                    </small>
                  </div>
                  <button
                    className="button ghost"
                    onClick={() =>
                      update((current) => ({
                        ...current,
                        permissions: current.permissions.filter((item) => item.id !== grant.id),
                      }))
                    }
                  >
                    Revoke
                  </button>
                </li>
              )
            })}
          </ul>
        )}
      </Section>

      <Section title="Available tools" description="Protocol: juniper-tool-protocol-v1">
        <ul className="tool-list">
          {builtinTools.map((tool) => (
            <li key={tool.name} className="tool-row">
              <div className="tool-row-head">
                <strong>{tool.displayName}</strong>
                <span className={`chip ${tool.risk === 'automatic-safe' ? '' : 'caution'}`}>
                  {riskLabel(tool.risk)}
                </span>
              </div>
              <p className="muted small">{tool.description}</p>
              <details className="disclosure">
                <summary>
                  <Icon name="chevronRight" size={16} />
                  Technical details
                </summary>
                <p className="mono small">{tool.name}</p>
                <pre className="code-block">{JSON.stringify(tool.schema, null, 2)}</pre>
              </details>
            </li>
          ))}
        </ul>
      </Section>
    </>
  )
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

export function MemorySettings({ data, update }: { data: AppData; update: Update }) {
  const [draft, setDraft] = useState('')
  function add(event: FormEvent) {
    event.preventDefault()
    const content = draft.trim()
    if (!content) return
    const timestamp = now()
    update((current) => ({
      ...current,
      memories: [
        ...current.memories,
        {
          id: uid('memory'),
          content,
          source: 'user',
          enabled: true,
          createdAt: timestamp,
          updatedAt: timestamp,
        },
      ],
    }))
    setDraft('')
  }
  return (
    <Section
      title="Curated memory"
      description="Juniper only uses memories you can see here. Nothing is silently added."
    >
      <form className="inline-field" onSubmit={add}>
        <label className="field">
          <span>New memory</span>
          <input
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            placeholder="A preference or helpful fact"
            maxLength={1000}
          />
        </label>
        <button className="button primary" type="submit" disabled={!draft.trim()}>
          Save memory
        </button>
      </form>
      {data.memories.length === 0 ? (
        <p className="muted">No memories yet.</p>
      ) : (
        <ul className="memory-list">
          {data.memories.map((memory) => (
            <li key={memory.id} className="memory-row">
              <Switch
                checked={memory.enabled}
                label={`Use memory: ${memory.content}`}
                onChange={() =>
                  update((current) => ({
                    ...current,
                    memories: current.memories.map((item) =>
                      item.id === memory.id ? { ...item, enabled: !item.enabled } : item,
                    ),
                  }))
                }
              />
              <span className={memory.enabled ? '' : 'muted'}>{memory.content}</span>
              <button
                className="icon-button"
                aria-label={`Delete memory: ${memory.content}`}
                title="Delete"
                onClick={() =>
                  update((current) => ({
                    ...current,
                    memories: current.memories.filter((item) => item.id !== memory.id),
                  }))
                }
              >
                <Icon name="trash" size={18} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </Section>
  )
}

// ---------------------------------------------------------------------------
// Privacy & data
// ---------------------------------------------------------------------------

function redactProvider(provider: ProviderProfile): ProviderProfile {
  const safe = { ...provider }
  delete safe.apiKeyRef
  return safe
}

export function PrivacySettings({
  data,
  update,
  route,
  currentConversation,
}: {
  data: AppData
  update: Update
  route: ConversationRoute
  currentConversation?: AppData['conversations'][number]
}) {
  const dialogs = useDialogs()
  const { assistant, model, provider } = route
  const privateChat = currentConversation?.privateChat === true

  async function clearChats() {
    const confirmed = await dialogs.confirm({
      title: 'Clear all saved chats?',
      message: 'Every saved chat, its attachment records, and chat-scoped permissions are deleted.',
      confirmLabel: 'Clear chats',
      danger: true,
    })
    if (confirmed)
      update((current) => ({
        ...current,
        conversations: [],
        attachments: [],
        permissions: current.permissions.filter((grant) => grant.scope !== 'chat'),
      }))
  }

  async function clearMemory() {
    const confirmed = await dialogs.confirm({
      title: 'Clear all memories?',
      message: 'Every curated memory is deleted.',
      confirmLabel: 'Clear memories',
      danger: true,
    })
    if (confirmed) update((current) => ({ ...current, memories: [] }))
  }

  return (
    <>
      <div className="callout success">
        <Icon name="shield" />
        <div>
          <strong>Telemetry is off</strong>
          <p>
            Juniper has no analytics, advertising, crash reporting, or automatic conversation
            uploads.
          </p>
        </div>
      </div>

      <Section
        title="Current route"
        description={currentConversation ? 'For the open chat.' : 'For a new chat.'}
      >
        <dl className="detail-list">
          <div>
            <dt>Assistant</dt>
            <dd>{assistant.name}</dd>
          </div>
          <div>
            <dt>Model</dt>
            <dd>{model?.displayName ?? 'No model selected'}</dd>
          </div>
          <div>
            <dt>Provider</dt>
            <dd>{provider?.name ?? 'No provider selected'}</dd>
          </div>
          <div>
            <dt>Execution</dt>
            <dd>
              <LocationBadge location={model?.executionLocation} />
            </dd>
          </div>
          <div>
            <dt>Persistence</dt>
            <dd>
              {currentConversation
                ? privateChat
                  ? 'PRIVATE · SESSION ONLY'
                  : 'SAVED'
                : 'No active chat'}
            </dd>
          </div>
          <div>
            <dt>Memory</dt>
            <dd>{assistant.memoryPolicy === 'curated' ? 'Curated memory enabled' : 'Off'}</dd>
          </div>
          <div>
            <dt>Network tools</dt>
            <dd>Off — no network tools are enabled in this release.</dd>
          </div>
        </dl>
        <p className="muted">
          {model?.executionLocation === 'remote'
            ? `Prompts sent to ${provider?.name ?? 'this provider'} leave the device. This is explicit and visible.`
            : model?.executionLocation === 'on-device'
              ? 'Prompts remain on this device while using this model.'
              : model?.executionLocation === 'local-network'
                ? 'Prompts are sent to another device on your local network.'
                : `Execution location is ${labelExecutionLocation(undefined)} until the provider reports enough information.`}
        </p>
      </Section>

      <Section title="Stored on this device">
        <dl className="detail-list">
          <div>
            <dt>Saved chats</dt>
            <dd>{data.conversations.filter((chat) => !chat.privateChat).length}</dd>
          </div>
          <div>
            <dt>Memories in use</dt>
            <dd>{data.memories.filter((memory) => memory.enabled).length}</dd>
          </div>
        </dl>
        <p className="muted small">
          Private chats are never written to storage and disappear when Juniper closes.
        </p>
      </Section>

      <Section title="Your data" description="Exports never include API keys or private chats.">
        <div className="button-row wrap">
          <button
            className="button secondary"
            onClick={() =>
              download(
                'juniper-export.json',
                JSON.stringify(
                  {
                    format: 'juniper-export',
                    version: 2,
                    ...withoutPrivateChats(data),
                    providers: data.providers.map(redactProvider),
                  },
                  null,
                  2,
                ),
                'application/json',
              )
            }
          >
            <Icon name="download" size={18} />
            Export data
          </button>
          <button className="button danger-ghost" onClick={() => void clearChats()}>
            Clear chats
          </button>
          <button className="button danger-ghost" onClick={() => void clearMemory()}>
            Clear memory
          </button>
        </div>
      </Section>
    </>
  )
}

// ---------------------------------------------------------------------------
// Advanced & diagnostics
// ---------------------------------------------------------------------------

export function AdvancedSettings({
  data,
  update,
  openSection,
}: {
  data: AppData
  update: Update
  openSection: (section: SettingsSection) => void
}) {
  const set = useSettingsUpdater(update)
  return (
    <>
      <Section title="Developer">
        <Row
          label="Developer mode"
          description="Adds the context inspector to chat menus, internal error codes, and extra model metadata."
          control={({ id, describedBy }) => (
            <Switch
              id={id}
              describedBy={describedBy}
              checked={data.settings.developerMode}
              onChange={(value) => set('developerMode', value)}
            />
          )}
        />
        <div className="setting-row">
          <div className="setting-text">
            <span className="label">Diagnostics</span>
            <small>Runtime details, provider capabilities, and recent runtime events.</small>
          </div>
          <div className="setting-control">
            <button className="button secondary" onClick={() => openSection('diagnostics')}>
              Open diagnostics
            </button>
          </div>
        </div>
      </Section>
      <Section
        title="Not available in this release"
        description="Listed so you know they are intentionally off, not hidden."
      >
        <div className="setting-row">
          <div className="setting-text">
            <span className="label">Runtime and process limits</span>
            <small>Runtime limits are managed by the provider in this release.</small>
          </div>
          <div className="setting-control">
            <span className="chip">Unavailable</span>
          </div>
        </div>
        <div className="setting-row">
          <div className="setting-text">
            <span className="label">MCP servers</span>
            <small>MCP is an explicitly unavailable advanced feature in this release.</small>
          </div>
          <div className="setting-control">
            <span className="chip">Unavailable</span>
          </div>
        </div>
        <p className="muted small">
          Generation controls are capability-detected per assistant under Assistants › Advanced
          generation. No unsupported parameter is sent to a provider.
        </p>
      </Section>
    </>
  )
}

export function DiagnosticsSettings({ data }: { data: AppData }) {
  const [diagnostics, setDiagnostics] = useState<Record<string, string>>({})
  const [logs, setLogs] = useState<RuntimeLogEntry[]>([])
  useEffect(() => {
    void getDiagnostics().then(setDiagnostics)
    void getRuntimeLogs().then(setLogs)
  }, [])
  return (
    <>
      <Section
        title="Runtime"
        description="Truthful details for troubleshooting — never API secrets."
      >
        <dl className="detail-list">
          {Object.entries({
            ...diagnostics,
            telemetry: 'Off',
            models: `${data.models.length} profile(s)`,
          }).map(([key, value]) => (
            <div key={key}>
              <dt>{key.replace(/_/g, ' ')}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      </Section>
      <Section title="Provider capabilities" description="Unknown is never treated as supported.">
        <ul className="provider-list">
          {data.providers.map((provider) => (
            <li key={provider.id} className="provider-row">
              <div className="provider-row-main">
                <strong>{provider.name}</strong>
                <small className="mono">{provider.baseUrl}</small>
              </div>
              <LocationBadge location={provider.transportLocation} />
            </li>
          ))}
        </ul>
      </Section>
      <Section
        title="Runtime events"
        description="Bounded metadata only; private content and credentials are never recorded."
      >
        {logs.length === 0 ? (
          <p className="muted">No native runtime events recorded in this session.</p>
        ) : (
          <ol className="log-list" aria-label="Runtime events">
            {[...logs].reverse().map((entry, index) => (
              <li key={`${entry.timestamp}-${entry.event}-${index}`}>
                <span>{entry.event.replace(/_/g, ' ')}</span>
                <span className="mono">
                  {[entry.providerKind, entry.modelId, entry.code].filter(Boolean).join(' · ') ||
                    'ok'}
                </span>
              </li>
            ))}
          </ol>
        )}
      </Section>
    </>
  )
}

// ---------------------------------------------------------------------------
// About
// ---------------------------------------------------------------------------

export function AboutSettings({ data }: { data: AppData }) {
  const [version, setVersion] = useState<string | null>(null)
  useEffect(() => {
    void getDiagnostics().then((info) => setVersion(info.application ?? null))
  }, [])
  const juniper = data.assistants.find((assistant) => assistant.id === 'assistant-juniper')
  return (
    <>
      <div className="about-hero">
        {juniper && <AssistantAvatar assistant={juniper} className="about-mark" />}
        <div>
          <strong>{version ?? 'Juniper'}</strong>
          <p className="muted">Your AI. Your models. Your machine.</p>
        </div>
      </div>
      <Section title="License and credits">
        <dl className="detail-list">
          <div>
            <dt>Juniper</dt>
            <dd>Apache License 2.0 · Cinqic</dd>
          </div>
          <div>
            <dt>Fonts</dt>
            <dd>Inter, Atkinson Hyperlegible Next, OpenDyslexic · SIL Open Font License 1.1</dd>
          </div>
          <div>
            <dt>Third-party notices</dt>
            <dd>Included with the app as THIRD_PARTY_NOTICES.md</dd>
          </div>
          <div>
            <dt>Source</dt>
            <dd className="mono">github.com/Cinqic-Research/Juniper-App</dd>
          </div>
        </dl>
      </Section>
    </>
  )
}
