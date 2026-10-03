import { useState } from 'react'
import type { ChangeEvent } from 'react'
import { MAX_ASSISTANT_IMPORT_BYTES, parseAssistant, serializeAssistant } from '../lib/assistant'
import { defaultAssistant } from '../lib/defaults'
import type { AppData, Assistant } from '../types'
import { AssistantAvatar } from './branding'
import { Icon } from './icons'
import { defaultAssistantFor, isChatSelectable } from './model-labels'
import { useDialogs } from './overlays'
import { Section, download, now, uid } from './ui'

type Update = (change: (current: AppData) => AppData) => void

export function AssistantsSettings({
  data,
  update,
  onEditingChange,
}: {
  data: AppData
  update: Update
  onEditingChange?: (editing: boolean) => void
}) {
  const [editing, setEditingState] = useState<Assistant | null>(null)
  const defaultId = defaultAssistantFor(data).id

  function setEditing(next: Assistant | null) {
    setEditingState(next)
    onEditingChange?.(next !== null)
  }

  function save(assistant: Assistant) {
    update((current) => ({
      ...current,
      assistants: current.assistants.some((item) => item.id === assistant.id)
        ? current.assistants.map((item) =>
            item.id === assistant.id ? { ...assistant, updatedAt: now() } : item,
          )
        : [...current.assistants, assistant],
    }))
    setEditing(null)
  }

  function newAssistant() {
    setEditing({
      ...defaultAssistant,
      id: uid('assistant'),
      name: 'New assistant',
      description: 'A custom Juniper assistant.',
      systemPrompt: defaultAssistant.systemPrompt,
      createdAt: now(),
      updatedAt: now(),
    })
  }

  if (editing) {
    return (
      <AssistantBuilder
        key={editing.id}
        assistant={editing}
        models={data.models}
        onSave={save}
        onCancel={() => setEditing(null)}
      />
    )
  }

  return (
    <>
      <Section
        title="Your assistants"
        description="Behavior is separate from the model. Each assistant has its own personality, boundaries, and default model."
        action={
          <button className="button secondary" onClick={newAssistant}>
            <Icon name="plus" size={18} />
            New assistant
          </button>
        }
      >
        <ul className="assistant-list">
          {data.assistants.map((assistant) => {
            const model = data.models.find((item) => item.id === assistant.modelProfileId)
            return (
              <li key={assistant.id} className="assistant-row">
                <AssistantAvatar assistant={assistant} className="assistant-row-avatar" />
                <div className="assistant-row-text">
                  <strong>
                    {assistant.name}
                    {assistant.id === defaultId && <span className="chip">Default</span>}
                  </strong>
                  <small>
                    {assistant.description} · {model?.displayName ?? 'Model not selected'}
                  </small>
                </div>
                <div className="assistant-row-actions">
                  {assistant.id !== defaultId && (
                    <button
                      className="button ghost"
                      onClick={() =>
                        update((current) => ({
                          ...current,
                          settings: { ...current.settings, defaultAssistantId: assistant.id },
                        }))
                      }
                    >
                      Use for new chats
                    </button>
                  )}
                  <button
                    className="button secondary"
                    onClick={() => setEditing(assistant)}
                    aria-label={`Edit ${assistant.name}`}
                  >
                    Edit
                  </button>
                </div>
              </li>
            )
          })}
        </ul>
      </Section>
    </>
  )
}

function AssistantBuilder({
  assistant: initial,
  models,
  onSave,
  onCancel,
}: {
  assistant: Assistant
  models: AppData['models']
  onSave: (assistant: Assistant) => void
  onCancel: () => void
}) {
  const dialogs = useDialogs()
  const [assistant, setAssistant] = useState(initial)
  const set = <K extends keyof Assistant>(key: K, value: Assistant[K]) =>
    setAssistant((current) => ({ ...current, [key]: value }))

  async function importFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0]
    if (!file) return
    try {
      if (file.size > MAX_ASSISTANT_IMPORT_BYTES) {
        throw new Error('This assistant file is too large. The maximum size is 128 KiB.')
      }
      const imported = parseAssistant(await file.text())
      // `assistant-juniper` is reserved for the built-in profile. Importing a
      // Juniper backup into a custom editor applies its settings to that custom
      // profile without taking over Juniper's stored identity or creation time.
      setAssistant(
        imported.id === 'assistant-juniper' && initial.id !== 'assistant-juniper'
          ? { ...imported, id: initial.id, createdAt: initial.createdAt }
          : imported,
      )
    } catch (error) {
      void dialogs.notify(
        error instanceof Error ? error.message : 'Could not import assistant.',
        'Could not import the assistant',
      )
    } finally {
      event.target.value = ''
    }
  }

  function exportFile() {
    download(
      `${assistant.name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}.juniper`,
      serializeAssistant(assistant),
      'application/json',
    )
  }

  const generationNumber = (
    key: 'temperature' | 'topP' | 'maxOutput',
    label: string,
    min: number,
    max: number,
    step: number,
  ) => (
    <label className="field">
      <span>{label}</span>
      <input
        type="number"
        min={min}
        max={max}
        step={step}
        value={assistant.generation[key] ?? ''}
        onChange={(event) =>
          set('generation', {
            ...assistant.generation,
            [key]: event.target.value ? Number(event.target.value) : undefined,
          })
        }
      />
    </label>
  )

  return (
    <div className="builder">
      <div className="builder-bar">
        <button className="button ghost" onClick={onCancel}>
          <Icon name="back" size={18} />
          Assistants
        </button>
        <div className="button-row">
          <label className="button ghost file-button">
            Import
            <input
              type="file"
              accept=".juniper,.json,application/json"
              onChange={(event) => void importFile(event)}
            />
          </label>
          <button className="button ghost" onClick={exportFile}>
            Export
          </button>
          <button className="button primary" onClick={() => onSave(assistant)}>
            Save
          </button>
        </div>
      </div>

      <div className="builder-identity">
        <AssistantAvatar assistant={assistant} className="builder-avatar" />
        <h2>{assistant.name || 'Untitled assistant'}</h2>
      </div>

      <Section title="Identity">
        <div className="form-grid">
          <label className="field">
            <span>Name</span>
            <input value={assistant.name} onChange={(event) => set('name', event.target.value)} />
          </label>
          <label className="field">
            <span>Avatar (1–2 characters)</span>
            <input
              value={assistant.avatar}
              maxLength={2}
              onChange={(event) => set('avatar', event.target.value)}
            />
          </label>
          <label className="field wide">
            <span>Description</span>
            <input
              value={assistant.description}
              onChange={(event) => set('description', event.target.value)}
            />
          </label>
          <label className="field wide">
            <span>Instructions</span>
            <textarea
              value={assistant.systemPrompt}
              onChange={(event) => set('systemPrompt', event.target.value)}
              rows={7}
            />
          </label>
          <label className="field wide">
            <span>Welcome message</span>
            <input
              value={assistant.welcomeMessage}
              onChange={(event) => set('welcomeMessage', event.target.value)}
            />
          </label>
        </div>
      </Section>

      <Section
        title="Personality"
        description="Human labels are a starting point; the instructions stay editable."
      >
        <div className="personality-grid">
          {(Object.keys(assistant.personality) as Array<keyof Assistant['personality']>).map(
            (key) => {
              const value = assistant.personality[key]
              const level = value > 66 ? 'High' : value < 34 ? 'Low' : 'Balanced'
              return (
                <label key={key} className="field">
                  <span>
                    {key[0]?.toUpperCase()}
                    {key.slice(1)} <em>{level}</em>
                  </span>
                  <input
                    type="range"
                    min="0"
                    max="100"
                    value={value}
                    aria-valuetext={`${level}, ${value}`}
                    onChange={(event) =>
                      set('personality', {
                        ...assistant.personality,
                        [key]: Number(event.target.value),
                      })
                    }
                  />
                </label>
              )
            },
          )}
        </div>
        <div className="form-grid">
          <label className="field">
            <span>Response style</span>
            <select
              value={assistant.responseLength}
              onChange={(event) =>
                set('responseLength', event.target.value as Assistant['responseLength'])
              }
            >
              <option value="concise">Concise</option>
              <option value="balanced">Balanced</option>
              <option value="detailed">Detailed</option>
            </select>
          </label>
          <label className="field">
            <span>Default model</span>
            <select
              value={assistant.modelProfileId ?? ''}
              onChange={(event) => set('modelProfileId', event.target.value || null)}
            >
              <option value="">Not selected</option>
              {models.filter(isChatSelectable).map((model) => (
                <option key={model.id} value={model.id}>
                  {model.displayName}
                </option>
              ))}
            </select>
          </label>
        </div>
      </Section>

      <Section title="Boundaries" description="Juniper’s host runtime enforces these choices.">
        <div className="form-grid">
          <label className="field">
            <span>Tools</span>
            <select
              value={assistant.toolPolicy}
              onChange={(event) => set('toolPolicy', event.target.value as Assistant['toolPolicy'])}
            >
              <option value="ask">Ask before user-data tools</option>
              <option value="safe-automatic">Safe automatic tools only</option>
              <option value="disabled">Tools disabled</option>
            </select>
          </label>
          <label className="field">
            <span>Memory</span>
            <select
              value={assistant.memoryPolicy}
              onChange={(event) =>
                set('memoryPolicy', event.target.value as Assistant['memoryPolicy'])
              }
            >
              <option value="curated">Curated memories</option>
              <option value="off">Off</option>
            </select>
          </label>
        </div>
      </Section>

      <details className="disclosure section-disclosure">
        <summary>
          <Icon name="chevronRight" size={16} />
          Advanced generation
        </summary>
        <p className="muted small">
          Only controls the selected runtime supports are sent to it. Some models, such as gpt-oss,
          always reason; for them “Off or lowest” means low effort.
        </p>
        <div className="form-grid">
          {generationNumber('temperature', 'Temperature', 0, 2, 0.05)}
          {generationNumber('topP', 'Top P', 0, 1, 0.05)}
          {generationNumber('maxOutput', 'Max output tokens', 1, 32768, 1)}
          <label className="field">
            <span>Thinking</span>
            <select
              value={assistant.generation.thinking ?? 'auto'}
              onChange={(event) =>
                set('generation', {
                  ...assistant.generation,
                  thinking: event.target.value as Assistant['generation']['thinking'],
                })
              }
            >
              <option value="auto">Auto</option>
              <option value="off">Off or lowest</option>
              <option value="on">On</option>
              <option value="low">Low</option>
              <option value="medium">Medium</option>
              <option value="high">High</option>
            </select>
          </label>
        </div>
      </details>
    </div>
  )
}
