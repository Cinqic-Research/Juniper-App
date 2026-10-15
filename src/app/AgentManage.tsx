import { useEffect, useState } from 'react'
import type { AppData, Assistant } from '../types'
import type {
  AgentOverview,
  AgentProfile,
  AgentToolInfo,
  AutonomyPreset,
  Grant,
  PluginSummary,
  Project,
  Schedule,
  ScheduleRule,
  ScheduleSpec,
  Skill,
} from '../types/agent'
import { agentApi, composeAgentProfile, deviceTimeZone, errorText } from '../lib/agent'
import { randomUuid } from '../lib/ids'
import { saveProviderCredential } from '../lib/runtime'
import { isChatSelectable } from './model-labels'
import { Modal, useDialogs } from './overlays'
import { EmptyState, Section, Segmented, Switch, download } from './ui'

const RISK_LABEL: Record<string, string> = {
  safe: 'Local, safe',
  'project-read': 'Reads granted folders',
  'project-write': 'Changes files (asks first)',
  'memory-write': 'Saves memories (asks each time)',
  network: 'Uses the network',
  process: 'Runs code in a sandbox',
  plugin: 'Third-party plugin',
}

// ------------------------------------------------------------------ agent settings

export function AgentSettingsDialog({
  assistant,
  data,
  overview,
  onClose,
}: {
  assistant: Assistant
  data: AppData
  overview: AgentOverview | null
  onClose: () => void
}) {
  const dialogs = useDialogs()
  const [profile, setProfile] = useState<AgentProfile | null>(null)
  const [revision, setRevision] = useState(0)
  const [tools, setTools] = useState<AgentToolInfo[]>([])
  const [skills, setSkills] = useState<Skill[]>([])
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    void Promise.all([
      agentApi.profile(assistant.id),
      agentApi.tools(assistant.id),
      agentApi.skills(),
    ]).then(
      ([stored, available, installed]) => {
        setProfile(stored.profile)
        setRevision(stored.revision)
        setTools(available)
        setSkills(installed)
      },
      (error) => void dialogs.notify(errorText(error)),
    )
  }, [assistant.id, dialogs])

  if (!profile) {
    return (
      <Modal open onClose={onClose} title={`${assistant.name} — agent settings`}>
        <p className="muted">Loading…</p>
      </Modal>
    )
  }
  const models = data.models.filter(isChatSelectable)
  const update = (change: Partial<AgentProfile>) => setProfile({ ...profile, ...change })
  const toggleTool = (name: string, on: boolean) =>
    update({
      enabledTools: on
        ? [...profile.enabledTools, name]
        : profile.enabledTools.filter((tool) => tool !== name),
    })
  const selectedModelId = profile.modelProfileId ?? assistant.modelProfileId ?? ''
  const selectedModel = data.models.find((model) => model.id === selectedModelId)

  async function save() {
    if (!profile) return
    setSaving(true)
    try {
      const next = await agentApi.saveProfile(
        { ...profile, composedProfile: composeAgentProfile(assistant) },
        revision,
      )
      setRevision(next)
      onClose()
    } catch (error) {
      void dialogs.notify(errorText(error), 'Settings were not saved')
    } finally {
      setSaving(false)
    }
  }

  async function exportProfile() {
    const document = await agentApi.exportProfile(assistant.id)
    download(`${assistant.name}-agent.json`, JSON.stringify(document, null, 2), 'application/json')
  }

  return (
    <Modal
      open
      onClose={onClose}
      title={`${assistant.name} — agent settings`}
      description="Name, avatar, and personality are edited in Settings → Assistants. These settings decide how this assistant works as an agent."
      variant="sheet"
      footer={
        <div className="button-row">
          <button className="button ghost" onClick={() => void exportProfile()}>
            Export
          </button>
          <button className="button secondary" onClick={onClose}>
            Cancel
          </button>
          <button className="button primary" disabled={saving} onClick={() => void save()}>
            Save
          </button>
        </div>
      }
    >
      <Section title="Model">
        <label className="stacked-field">
          <span className="label">Model for agent tasks</span>
          <select
            value={selectedModelId}
            onChange={(event) => update({ modelProfileId: event.target.value || null })}
          >
            <option value="">Use the assistant’s model</option>
            {models.map((model) => (
              <option key={model.id} value={model.id}>
                {model.displayName}
                {model.capabilities.tools === 'supported' ? '' : ' (tools not qualified)'}
              </option>
            ))}
          </select>
        </label>
        {selectedModel && selectedModel.capabilities.tools !== 'supported' && (
          <div className="setting-row">
            <div className="setting-text">
              <span className="label">Try this model anyway</span>
              <small>
                {selectedModel.displayName} has no measured evidence of reliable tool use. Tasks
                will run but may fail or misuse tools; they stay labelled as not qualified.
              </small>
            </div>
            <Switch
              label="Allow an unqualified model"
              checked={profile.allowUnqualifiedModel}
              onChange={(value) => update({ allowUnqualifiedModel: value })}
            />
          </div>
        )}
        <label className="stacked-field">
          <span className="label">Preferred language (optional)</span>
          <input
            value={profile.preferredLanguage ?? ''}
            maxLength={40}
            placeholder="For example: English"
            onChange={(event) => update({ preferredLanguage: event.target.value })}
          />
        </label>
      </Section>

      <Section title="Permissions" description="What the agent may do without asking you first.">
        <Segmented<AutonomyPreset>
          label="Permission preset"
          value={profile.autonomy}
          onChange={(value) => update({ autonomy: value })}
          options={[
            { value: 'ask', label: 'Ask first' },
            { value: 'project', label: 'Within projects' },
            { value: 'custom', label: 'Custom' },
          ]}
        />
        <p className="muted small">
          {overview?.presets.find((preset) => preset.id === profile.autonomy)?.description}
        </p>
      </Section>

      <Section
        title="Tools"
        description="Only enabled tools are offered to the model. Unavailable tools say why."
      >
        <ul className="tool-list">
          {tools.map((tool) => (
            <li key={tool.name} className="tool-row">
              <div className="tool-row-head">
                <span>
                  <strong>{tool.displayName}</strong>{' '}
                  <small className="muted">
                    {RISK_LABEL[tool.risk] ?? tool.risk}
                    {tool.source !== 'built-in' ? ` · ${tool.source}` : ''}
                  </small>
                </span>
                <Switch
                  label={`Enable ${tool.displayName}`}
                  checked={tool.enabled && profile.enabledTools.includes(tool.name)}
                  disabled={!tool.available}
                  onChange={(value) => toggleTool(tool.name, value)}
                />
              </div>
              <small className="muted">{tool.available ? tool.description : tool.reason}</small>
            </li>
          ))}
        </ul>
      </Section>

      <Section
        title="Skills"
        description="Procedures the agent can load when a task fits. Skills never grant permissions."
      >
        {skills.filter((skill) => skill.enabled).length === 0 && (
          <p className="muted">No skills are turned on. Turn them on under Skills & plugins.</p>
        )}
        {skills
          .filter((skill) => skill.enabled)
          .map((skill) => (
            <div key={skill.id} className="setting-row">
              <div className="setting-text">
                <span className="label">{skill.name}</span>
                <small>{skill.description}</small>
              </div>
              <Switch
                label={`Use ${skill.name}`}
                checked={profile.enabledSkills.includes(skill.id)}
                onChange={(value) =>
                  update({
                    enabledSkills: value
                      ? [...profile.enabledSkills, skill.id]
                      : profile.enabledSkills.filter((id) => id !== skill.id),
                  })
                }
              />
            </div>
          ))}
      </Section>

      <Section
        title="Limits"
        description="Each task stops when it reaches a limit and keeps what it made."
      >
        <div className="limit-grid">
          <label>
            <span className="label">Model steps</span>
            <input
              type="number"
              min={1}
              max={64}
              value={profile.budget.maxSteps}
              onChange={(event) =>
                update({ budget: { ...profile.budget, maxSteps: Number(event.target.value) || 1 } })
              }
            />
          </label>
          <label>
            <span className="label">Minutes</span>
            <input
              type="number"
              min={1}
              max={240}
              value={Math.round(profile.budget.maxSeconds / 60)}
              onChange={(event) =>
                update({
                  budget: { ...profile.budget, maxSeconds: (Number(event.target.value) || 1) * 60 },
                })
              }
            />
          </label>
          <label>
            <span className="label">Tokens</span>
            <input
              type="number"
              min={1000}
              step={1000}
              value={profile.budget.maxTokens}
              onChange={(event) =>
                update({
                  budget: { ...profile.budget, maxTokens: Number(event.target.value) || 1000 },
                })
              }
            />
          </label>
        </div>
      </Section>
    </Modal>
  )
}

// ------------------------------------------------------------------ projects

export function ProjectsPanel({
  overview,
  onChanged,
}: {
  overview: AgentOverview
  onChanged: () => Promise<void>
}) {
  const dialogs = useDialogs()
  const [editing, setEditing] = useState<Project | 'new' | null>(null)
  const [busy, setBusy] = useState<string | null>(null)

  async function act(
    key: string,
    action: () => Promise<unknown>,
    done?: (value: unknown) => string | null,
  ) {
    setBusy(key)
    try {
      const value = await action()
      await onChanged()
      const message = done?.(value)
      if (message) void dialogs.notify(message)
    } catch (error) {
      void dialogs.notify(errorText(error))
    } finally {
      setBusy(null)
    }
  }

  async function importProject(file: File) {
    try {
      const document: unknown = JSON.parse(await file.text())
      await agentApi.importProject(document)
      await onChanged()
    } catch (error) {
      void dialogs.notify(errorText(error), 'The project could not be imported')
    }
  }

  return (
    <Section
      title="Projects"
      description="A project gathers instructions and the folders an agent may use. Folder access is granted here and nowhere else."
      action={
        <span className="button-row">
          <label className="button ghost">
            Import
            <input
              type="file"
              accept="application/json"
              className="visually-hidden"
              onChange={(event) => {
                const file = event.target.files?.[0]
                if (file) void importProject(file)
                event.target.value = ''
              }}
            />
          </label>
          <button className="button primary" onClick={() => setEditing('new')}>
            New project
          </button>
        </span>
      }
    >
      {overview.projects.length === 0 && (
        <EmptyState title="No projects yet">
          Create one, then grant it a folder so an agent can read (and, if you allow, change) files
          there.
        </EmptyState>
      )}
      <ul className="project-list">
        {overview.projects.map((project) => (
          <li key={project.id} className={`project-row ${project.archived ? 'archived' : ''}`}>
            <div className="project-head">
              <strong>{project.name}</strong>
              {project.archived && <span className="chip">Archived</span>}
              <span className="button-row">
                <button className="button ghost" onClick={() => setEditing(project)}>
                  Edit
                </button>
                <button
                  className="button ghost"
                  disabled={busy === project.id}
                  onClick={() =>
                    void act(
                      project.id,
                      () => agentApi.indexProject(project.id),
                      (value) => {
                        const stats = value as {
                          indexed: number
                          unchanged: number
                          removed: number
                        }
                        return `Indexed ${stats.indexed} changed file(s); ${stats.unchanged} unchanged; ${stats.removed} removed from the index.`
                      },
                    )
                  }
                >
                  {busy === project.id ? 'Indexing…' : 'Index documents'}
                </button>
                <button
                  className="button ghost"
                  onClick={() =>
                    void agentApi
                      .exportProject(project.id)
                      .then((document) =>
                        download(
                          `${project.name}.juniper-project.json`,
                          JSON.stringify(document, null, 2),
                          'application/json',
                        ),
                      )
                  }
                >
                  Export
                </button>
              </span>
            </div>
            {project.instructions && <p className="muted small">{project.instructions}</p>}
            <ul className="root-list">
              {project.roots.map((root) => (
                <li key={root.id}>
                  <span>
                    <strong>{root.label}</strong> <small className="muted mono">{root.path}</small>{' '}
                    <span className="chip">
                      {root.access === 'read-write' ? 'Read and write' : 'Read only'}
                    </span>
                  </span>
                  <button
                    className="button danger-ghost"
                    onClick={() =>
                      void dialogs
                        .confirm({
                          title: `Remove access to ${root.label}?`,
                          message:
                            'Agents lose access at once, and its passages are removed from the project index. Files are not touched.',
                          confirmLabel: 'Remove access',
                          danger: true,
                        })
                        .then((yes) => {
                          if (yes) void act(root.id, () => agentApi.removeRoot(root.id))
                        })
                    }
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ul>
            {!project.archived && (
              <div className="button-row">
                <button
                  className="button secondary"
                  onClick={() =>
                    void act(`${project.id}-r`, () => agentApi.addRoot(project.id, 'read'))
                  }
                >
                  Grant a folder (read only)
                </button>
                <button
                  className="button secondary"
                  onClick={() =>
                    void act(`${project.id}-w`, () => agentApi.addRoot(project.id, 'read-write'))
                  }
                >
                  Grant a folder (read and write)
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>
      {editing && (
        <ProjectEditor
          project={editing === 'new' ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={async () => {
            setEditing(null)
            await onChanged()
          }}
        />
      )}
    </Section>
  )
}

function ProjectEditor({
  project,
  onClose,
  onSaved,
}: {
  project: Project | null
  onClose: () => void
  onSaved: () => Promise<void>
}) {
  const dialogs = useDialogs()
  const [name, setName] = useState(project?.name ?? '')
  const [instructions, setInstructions] = useState(project?.instructions ?? '')
  const [archived, setArchived] = useState(project?.archived ?? false)

  async function save() {
    try {
      await agentApi.saveProject({
        id: project?.id ?? null,
        name,
        instructions,
        archived,
        expectedRevision: project?.revision ?? 0,
      })
      await onSaved()
    } catch (error) {
      void dialogs.notify(errorText(error))
    }
  }

  async function remove() {
    if (!project) return
    const yes = await dialogs.confirm({
      title: `Delete ${project.name}?`,
      message:
        'Its folder grants and document index are removed. Tasks and documents it produced stay, unlinked. Files on disk are not touched.',
      confirmLabel: 'Delete project',
      danger: true,
    })
    if (!yes) return
    await agentApi.deleteProject(project.id).catch((error) => dialogs.notify(errorText(error)))
    await onSaved()
  }

  return (
    <Modal
      open
      onClose={onClose}
      title={project ? 'Edit project' : 'New project'}
      footer={
        <div className="button-row">
          {project && (
            <button className="button danger-ghost" onClick={() => void remove()}>
              Delete
            </button>
          )}
          <button className="button secondary" onClick={onClose}>
            Cancel
          </button>
          <button className="button primary" disabled={!name.trim()} onClick={() => void save()}>
            Save
          </button>
        </div>
      }
    >
      <label className="stacked-field">
        <span className="label">Name</span>
        <input value={name} maxLength={100} onChange={(event) => setName(event.target.value)} />
      </label>
      <label className="stacked-field">
        <span className="label">Instructions for agents (optional)</span>
        <textarea
          rows={5}
          maxLength={8000}
          value={instructions}
          placeholder="Context about this project. These are lower priority than Juniper’s rules and your request."
          onChange={(event) => setInstructions(event.target.value)}
        />
      </label>
      {project && (
        <div className="setting-row">
          <div className="setting-text">
            <span className="label">Archived</span>
            <small>Archived projects keep their data but cannot start tasks.</small>
          </div>
          <Switch label="Archived" checked={archived} onChange={setArchived} />
        </div>
      )}
    </Modal>
  )
}

// ------------------------------------------------------------------ schedules

const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

function describeRule(rule: ScheduleRule): string {
  switch (rule.kind) {
    case 'once':
      return `Once at ${rule.at.replace('T', ' ')}`
    case 'daily':
      return `Every day at ${rule.time}`
    case 'weekly':
      return `Every ${rule.days.map((day) => WEEKDAYS[day - 1]).join(', ')} at ${rule.time}`
    case 'monthly':
      return `Monthly on day ${rule.day} at ${rule.time}`
    case 'interval':
      return `Every ${rule.minutes} minutes`
  }
}

export function SchedulesPanel({
  data,
  overview,
  onChanged,
}: {
  data: AppData
  overview: AgentOverview
  onChanged: () => Promise<void>
}) {
  const dialogs = useDialogs()
  const [schedules, setSchedules] = useState<Schedule[]>([])
  const [editing, setEditing] = useState<Schedule | 'new' | null>(null)

  const load = () =>
    agentApi.schedules().then(setSchedules, (error) => void dialogs.notify(errorText(error)))
  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [overview])

  async function act(action: () => Promise<unknown>) {
    try {
      await action()
      await load()
      await onChanged()
    } catch (error) {
      void dialogs.notify(errorText(error))
    }
  }

  return (
    <Section
      title="Schedules"
      description={overview.capabilities.scheduling.note}
      action={
        <button className="button primary" onClick={() => setEditing('new')}>
          New schedule
        </button>
      }
    >
      {schedules.length === 0 && (
        <EmptyState title="Nothing scheduled">
          Schedule a reminder, or a task that an agent runs at set times.
        </EmptyState>
      )}
      <ul className="schedule-list">
        {schedules.map((schedule) => (
          <li key={schedule.id} className="schedule-row">
            <div className="project-head">
              <strong>{schedule.spec.title}</strong>
              <span className="chip">
                {schedule.spec.action === 'reminder' ? 'Reminder' : 'Agent task'}
              </span>
              <Switch
                label={`${schedule.enabled ? 'Pause' : 'Resume'} ${schedule.spec.title}`}
                checked={schedule.enabled}
                onChange={(enabled) =>
                  void act(() =>
                    agentApi.saveSchedule(schedule.id, schedule.spec, enabled, schedule.revision),
                  )
                }
              />
            </div>
            <p className="muted small">
              {describeRule(schedule.spec.rule)} ({schedule.spec.timezone}) · missed runs:{' '}
              {schedule.spec.missedPolicy === 'skip' ? 'skipped' : 'run once at next launch'}
            </p>
            {schedule.enabled && schedule.upcoming.length > 0 && (
              <p className="small">Next: {schedule.upcoming.slice(0, 3).join(' · ')}</p>
            )}
            <div className="button-row">
              <button
                className="button ghost"
                onClick={() => void act(() => agentApi.runScheduleNow(schedule.id))}
              >
                Run now
              </button>
              <button className="button ghost" onClick={() => setEditing(schedule)}>
                Edit
              </button>
              <button
                className="button danger-ghost"
                onClick={() =>
                  void dialogs
                    .confirm({
                      title: `Delete ${schedule.spec.title}?`,
                      confirmLabel: 'Delete',
                      danger: true,
                    })
                    .then((yes) => {
                      if (yes) void act(() => agentApi.deleteSchedule(schedule.id))
                    })
                }
              >
                Delete
              </button>
            </div>
          </li>
        ))}
      </ul>
      {editing && (
        <ScheduleEditor
          data={data}
          overview={overview}
          schedule={editing === 'new' ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={async () => {
            setEditing(null)
            await load()
            await onChanged()
          }}
        />
      )}
    </Section>
  )
}

function ScheduleEditor({
  data,
  overview,
  schedule,
  onClose,
  onSaved,
}: {
  data: AppData
  overview: AgentOverview
  schedule: Schedule | null
  onClose: () => void
  onSaved: () => Promise<void>
}) {
  const dialogs = useDialogs()
  const [spec, setSpec] = useState<ScheduleSpec>(
    schedule?.spec ?? {
      title: '',
      action: 'agent-task',
      prompt: '',
      assistantId: data.settings.defaultAssistantId ?? data.assistants[0]?.id ?? '',
      projectId: null,
      timezone: overview.timeZone || deviceTimeZone(),
      rule: { kind: 'daily', time: '09:00' },
      missedPolicy: 'run-once',
      retryLimit: 0,
    },
  )
  const [preview, setPreview] = useState<string[]>([])
  const [previewError, setPreviewError] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    agentApi.previewSchedule(spec).then(
      (next) => {
        if (current) {
          setPreview(next)
          setPreviewError(null)
        }
      },
      (error) => current && setPreviewError(errorText(error)),
    )
    return () => {
      current = false
    }
  }, [spec])

  const update = (change: Partial<ScheduleSpec>) => setSpec({ ...spec, ...change })
  const setRule = (rule: ScheduleRule) => update({ rule })
  const time = 'time' in spec.rule ? spec.rule.time : '09:00'

  async function save() {
    try {
      await agentApi.saveSchedule(
        schedule?.id ?? null,
        spec,
        schedule?.enabled ?? true,
        schedule?.revision ?? 0,
      )
      await onSaved()
    } catch (error) {
      void dialogs.notify(errorText(error))
    }
  }

  return (
    <Modal
      open
      onClose={onClose}
      title={schedule ? 'Edit schedule' : 'New schedule'}
      variant="sheet"
      footer={
        <div className="button-row">
          <button className="button secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="button primary"
            disabled={Boolean(previewError)}
            onClick={() => void save()}
          >
            Save schedule
          </button>
        </div>
      }
    >
      <label className="stacked-field">
        <span className="label">Title</span>
        <input
          value={spec.title}
          maxLength={100}
          onChange={(event) => update({ title: event.target.value })}
        />
      </label>
      <Segmented<ScheduleSpec['action']>
        label="What happens"
        value={spec.action}
        onChange={(action) => update({ action })}
        options={[
          { value: 'agent-task', label: 'Run an agent task' },
          { value: 'reminder', label: 'Remind me (no model)' },
        ]}
      />
      {spec.action === 'agent-task' && (
        <div className="agent-composer-row">
          <label>
            <span className="label">Agent</span>
            <select
              value={spec.assistantId}
              onChange={(event) => update({ assistantId: event.target.value })}
            >
              {data.assistants.map((assistant) => (
                <option key={assistant.id} value={assistant.id}>
                  {assistant.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            <span className="label">Project</span>
            <select
              value={spec.projectId ?? ''}
              onChange={(event) => update({ projectId: event.target.value || null })}
            >
              <option value="">No project</option>
              {overview.projects
                .filter((project) => !project.archived)
                .map((project) => (
                  <option key={project.id} value={project.id}>
                    {project.name}
                  </option>
                ))}
            </select>
          </label>
        </div>
      )}
      <label className="stacked-field">
        <span className="label">{spec.action === 'reminder' ? 'Reminder text' : 'Task'}</span>
        <textarea
          rows={3}
          maxLength={8000}
          value={spec.prompt}
          onChange={(event) => update({ prompt: event.target.value })}
        />
      </label>
      <label className="stacked-field">
        <span className="label">Repeats</span>
        <select
          value={spec.rule.kind}
          onChange={(event) => {
            const kind = event.target.value as ScheduleRule['kind']
            if (kind === 'once')
              setRule({ kind, at: `${new Date().toISOString().slice(0, 10)}T${time}` })
            else if (kind === 'daily') setRule({ kind, time })
            else if (kind === 'weekly') setRule({ kind, days: [1], time })
            else if (kind === 'monthly') setRule({ kind, day: 1, time })
            else setRule({ kind, minutes: 60 })
          }}
        >
          <option value="once">Once</option>
          <option value="daily">Daily</option>
          <option value="weekly">Weekly</option>
          <option value="monthly">Monthly</option>
          <option value="interval">Every few minutes or hours</option>
        </select>
      </label>
      {spec.rule.kind === 'once' && (
        <label className="stacked-field">
          <span className="label">Date and time</span>
          <input
            type="datetime-local"
            value={spec.rule.at}
            onChange={(event) => setRule({ kind: 'once', at: event.target.value })}
          />
        </label>
      )}
      {(spec.rule.kind === 'daily' ||
        spec.rule.kind === 'weekly' ||
        spec.rule.kind === 'monthly') && (
        <label className="stacked-field">
          <span className="label">Time</span>
          <input
            type="time"
            value={time}
            onChange={(event) =>
              setRule({ ...spec.rule, time: event.target.value } as ScheduleRule)
            }
          />
        </label>
      )}
      {spec.rule.kind === 'weekly' && (
        <fieldset className="weekday-picker">
          <legend className="label">Days</legend>
          {WEEKDAYS.map((name, index) => {
            const rule = spec.rule as Extract<ScheduleRule, { kind: 'weekly' }>
            const day = index + 1
            return (
              <label key={name}>
                <input
                  type="checkbox"
                  checked={rule.days.includes(day)}
                  onChange={(event) =>
                    setRule({
                      ...rule,
                      days: event.target.checked
                        ? [...rule.days, day].sort()
                        : rule.days.filter((value) => value !== day),
                    })
                  }
                />
                {name}
              </label>
            )
          })}
        </fieldset>
      )}
      {spec.rule.kind === 'monthly' && (
        <label className="stacked-field">
          <span className="label">Day of month</span>
          <input
            type="number"
            min={1}
            max={31}
            value={spec.rule.day}
            onChange={(event) =>
              setRule({
                ...(spec.rule as Extract<ScheduleRule, { kind: 'monthly' }>),
                day: Number(event.target.value) || 1,
              })
            }
          />
        </label>
      )}
      {spec.rule.kind === 'interval' && (
        <label className="stacked-field">
          <span className="label">Minutes between runs (15 or more)</span>
          <input
            type="number"
            min={15}
            value={spec.rule.minutes}
            onChange={(event) =>
              setRule({ kind: 'interval', minutes: Number(event.target.value) || 15 })
            }
          />
        </label>
      )}
      <label className="stacked-field">
        <span className="label">Time zone</span>
        <input
          value={spec.timezone}
          onChange={(event) => update({ timezone: event.target.value })}
        />
      </label>
      <Segmented<ScheduleSpec['missedPolicy']>
        label="If Juniper was closed at the scheduled time"
        value={spec.missedPolicy}
        onChange={(missedPolicy) => update({ missedPolicy })}
        options={[
          { value: 'run-once', label: 'Run once when it opens' },
          { value: 'skip', label: 'Skip it' },
        ]}
      />
      <div className="schedule-preview" aria-live="polite">
        <span className="label">Next runs</span>
        {previewError ? (
          <p className="warning-line">{previewError}</p>
        ) : (
          <ul>
            {preview.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        )}
        <p className="muted small">
          Runs happen only while Juniper is running. Actions that need approval wait for you.
        </p>
      </div>
    </Modal>
  )
}

// ------------------------------------------------------------------ skills and plugins

export function SkillsPanel({ onChanged }: { onChanged: () => Promise<void> }) {
  const dialogs = useDialogs()
  const [skills, setSkills] = useState<Skill[]>([])
  const [viewing, setViewing] = useState<Skill | null>(null)
  const load = () =>
    agentApi.skills().then(setSkills, (error) => void dialogs.notify(errorText(error)))
  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  async function act(action: () => Promise<unknown>) {
    try {
      await action()
      await load()
      await onChanged()
    } catch (error) {
      void dialogs.notify(errorText(error))
    }
  }

  return (
    <Section
      title="Skills"
      description="Step-by-step procedures an agent can follow. A skill is guidance only: it cannot grant access, install anything, or run code."
      action={
        <button
          className="button secondary"
          onClick={() =>
            void act(async () => {
              const imported = await agentApi.importSkill()
              if (imported) setViewing(imported)
            })
          }
        >
          Import skill folder
        </button>
      }
    >
      <ul className="tool-list">
        {skills.map((skill) => (
          <li key={skill.id} className="tool-row">
            <div className="tool-row-head">
              <span>
                <strong>{skill.name}</strong>{' '}
                <small className="muted">
                  v{skill.version} · {skill.bundled ? 'Bundled with Juniper' : skill.source}
                </small>
              </span>
              <Switch
                label={`Turn on ${skill.name}`}
                checked={skill.enabled}
                onChange={(value) => void act(() => agentApi.setSkillEnabled(skill.id, value))}
              />
            </div>
            <small className="muted">{skill.description}</small>
            <div className="button-row">
              <button className="button ghost" onClick={() => setViewing(skill)}>
                Inspect
              </button>
              <button
                className="button ghost"
                onClick={() => void act(() => agentApi.exportSkill(skill.id))}
              >
                Export
              </button>
              {!skill.bundled && (
                <button
                  className="button danger-ghost"
                  onClick={() => void act(() => agentApi.removeSkill(skill.id))}
                >
                  Remove
                </button>
              )}
            </div>
          </li>
        ))}
      </ul>
      {viewing && (
        <Modal
          open
          onClose={() => setViewing(null)}
          title={viewing.name}
          description={viewing.description}
          variant="sheet"
        >
          <dl className="detail-list compact">
            <dt>Source</dt>
            <dd>{viewing.source}</dd>
            <dt>Version</dt>
            <dd>{viewing.version}</dd>
            <dt>License</dt>
            <dd>{viewing.license ?? 'Not stated'}</dd>
            <dt>Asks for tools</dt>
            <dd>
              {viewing.requires.join(', ') || 'None'} (the agent’s own permissions still decide)
            </dd>
            <dt>Digest</dt>
            <dd className="mono">{viewing.digest}</dd>
          </dl>
          <pre className="artifact-preview">{viewing.body}</pre>
          {!viewing.enabled && (
            <p className="muted small">
              Imported skills start turned off. Read it, then turn it on in the list.
            </p>
          )}
        </Modal>
      )}
    </Section>
  )
}

export function PluginsPanel({
  overview,
  onChanged,
}: {
  overview: AgentOverview
  onChanged: () => Promise<void>
}) {
  const dialogs = useDialogs()
  const [plugins, setPlugins] = useState<PluginSummary[]>([])
  const [adding, setAdding] = useState<'http' | 'local' | null>(null)
  const [testing, setTesting] = useState<string | null>(null)
  const load = () =>
    agentApi.plugins().then(setPlugins, (error) => void dialogs.notify(errorText(error)))
  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  async function act(action: () => Promise<unknown>) {
    try {
      await action()
      await load()
      await onChanged()
    } catch (error) {
      void dialogs.notify(errorText(error))
    }
  }

  return (
    <Section
      title="Plugins (MCP servers)"
      description="Optional connections to Model Context Protocol servers. Their tools are third-party: every call asks you first unless you allow it, and a changed tool set needs approval again."
      action={
        <span className="button-row">
          {overview.capabilities.mcpStdio.available && (
            <button className="button secondary" onClick={() => setAdding('local')}>
              Add local server
            </button>
          )}
          <button className="button secondary" onClick={() => setAdding('http')}>
            Add remote server
          </button>
        </span>
      }
    >
      {plugins.length === 0 && (
        <p className="muted">No servers connected. Chat and agents work without them.</p>
      )}
      <ul className="tool-list">
        {plugins.map((plugin) => (
          <li key={plugin.id} className="tool-row">
            <div className="tool-row-head">
              <span>
                <strong>{plugin.name}</strong> <span className="chip">{plugin.location}</span>{' '}
                <small className="muted">
                  {plugin.state.status === 'connected'
                    ? `Connected · MCP ${plugin.state.protocolVersion} (${plugin.state.era}) · ${plugin.tools.length} tools`
                    : plugin.state.status === 'error'
                      ? `Error: ${plugin.state.message ?? 'unknown'}`
                      : 'Not tested'}
                </small>
              </span>
              <Switch
                label={`Enable ${plugin.name}`}
                checked={plugin.enabled}
                disabled={plugin.state.status !== 'connected'}
                onChange={(value) => void act(() => agentApi.setPluginEnabled(plugin.id, value))}
              />
            </div>
            <small className="muted">
              {plugin.access}
              {plugin.kind === 'mcp-stdio'
                ? ` Program: ${plugin.command} ${plugin.args.join(' ')}`
                : ` Endpoint: ${plugin.url}`}
            </small>
            {plugin.tools.length > 0 && (
              <details>
                <summary>Tools</summary>
                <ul>
                  {plugin.tools.map((tool) => (
                    <li key={tool.name}>
                      <strong>{tool.name}</strong>{' '}
                      <small className="muted">{tool.description}</small>
                    </li>
                  ))}
                </ul>
                {plugin.state.rejectedTools.length > 0 && (
                  <p className="warning-line">
                    Not offered (invalid definitions): {plugin.state.rejectedTools.join('; ')}
                  </p>
                )}
                <p className="muted small">Turn tools on for an agent in its agent settings.</p>
              </details>
            )}
            <div className="button-row">
              <button
                className="button ghost"
                disabled={testing === plugin.id}
                onClick={() => {
                  setTesting(plugin.id)
                  void act(() => agentApi.testPlugin(plugin.id)).finally(() => setTesting(null))
                }}
              >
                {testing === plugin.id ? 'Testing…' : 'Test connection'}
              </button>
              <button
                className="button danger-ghost"
                onClick={() =>
                  void dialogs
                    .confirm({
                      title: `Remove ${plugin.name}?`,
                      message:
                        'Its permissions are revoked and running tasks that use it are paused.',
                      confirmLabel: 'Remove',
                      danger: true,
                    })
                    .then((yes) => {
                      if (yes) void act(() => agentApi.removePlugin(plugin.id))
                    })
                }
              >
                Remove
              </button>
            </div>
          </li>
        ))}
      </ul>
      {adding && (
        <PluginEditor
          kind={adding}
          onClose={() => setAdding(null)}
          onSaved={async () => {
            setAdding(null)
            await load()
          }}
        />
      )}
    </Section>
  )
}

function PluginEditor({
  kind,
  onClose,
  onSaved,
}: {
  kind: 'http' | 'local'
  onClose: () => void
  onSaved: () => Promise<void>
}) {
  const dialogs = useDialogs()
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [token, setToken] = useState('')
  const [args, setArgs] = useState('')

  async function save() {
    try {
      if (kind === 'http') {
        let reference: string | null = null
        if (token.trim()) {
          reference = `mcp-${randomUuid()}`
          await saveProviderCredential(reference, token.trim())
        }
        await agentApi.registerHttpPlugin(name, url, reference)
      } else {
        const list = args
          .split('\n')
          .map((line) => line.trim())
          .filter(Boolean)
        const result = await agentApi.registerLocalPlugin(name, list, [])
        if (!result) return
      }
      await onSaved()
    } catch (error) {
      void dialogs.notify(errorText(error))
    }
  }

  return (
    <Modal
      open
      onClose={onClose}
      title={kind === 'http' ? 'Add a remote MCP server' : 'Add a local MCP server'}
      description={
        kind === 'http'
          ? 'Tool arguments you approve are sent to this server over the network. Juniper supports a bearer token, kept in your system keychain and sent only to this address.'
          : 'You will choose the server program next. It runs as a normal program with your account’s access — it is not sandboxed. Only add servers you trust.'
      }
      footer={
        <div className="button-row">
          <button className="button secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="button primary"
            disabled={!name.trim() || (kind === 'http' && !url.trim())}
            onClick={() => void save()}
          >
            {kind === 'http' ? 'Add server' : 'Choose program…'}
          </button>
        </div>
      }
    >
      <label className="stacked-field">
        <span className="label">Name</span>
        <input value={name} maxLength={80} onChange={(event) => setName(event.target.value)} />
      </label>
      {kind === 'http' ? (
        <>
          <label className="stacked-field">
            <span className="label">MCP endpoint (https)</span>
            <input
              value={url}
              placeholder="https://example.com/mcp"
              onChange={(event) => setUrl(event.target.value)}
            />
          </label>
          <label className="stacked-field">
            <span className="label">Access token (optional)</span>
            <input
              type="password"
              value={token}
              autoComplete="off"
              onChange={(event) => setToken(event.target.value)}
            />
          </label>
        </>
      ) : (
        <label className="stacked-field">
          <span className="label">Arguments, one per line</span>
          <textarea
            rows={3}
            value={args}
            placeholder={'-y\n@modelcontextprotocol/server-everything'}
            onChange={(event) => setArgs(event.target.value)}
          />
        </label>
      )}
    </Modal>
  )
}

// ------------------------------------------------------------------ permissions, web, inbox

export function PermissionsPanel({
  data,
  overview,
  onChanged,
  onOpenTask,
}: {
  data: AppData
  overview: AgentOverview
  onChanged: () => Promise<void>
  onOpenTask: (id: string) => void
}) {
  const dialogs = useDialogs()
  const [grants, setGrants] = useState<Grant[]>([])
  const web = (overview.settings.web ?? {}) as {
    provider?: string
    language?: string
    baseUrl?: string
  }
  const [provider, setProvider] = useState(web.provider ?? 'wikipedia')
  const [language, setLanguage] = useState(web.language ?? 'en')
  const [baseUrl, setBaseUrl] = useState(web.baseUrl ?? '')
  const [braveKey, setBraveKey] = useState('')
  const load = () =>
    agentApi.grants().then(setGrants, (error) => void dialogs.notify(errorText(error)))
  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [overview])

  async function saveWeb() {
    try {
      let settings: Record<string, unknown> = { provider, language }
      if (provider === 'brave') {
        const reference = 'web-search-brave'
        if (braveKey.trim()) await saveProviderCredential(reference, braveKey.trim())
        settings = { provider, credentialRef: reference }
      } else if (provider === 'searxng') settings = { provider, baseUrl }
      await agentApi.setSettings({ web: settings })
      setBraveKey('')
      await onChanged()
    } catch (error) {
      void dialogs.notify(errorText(error))
    }
  }

  const unread = overview.inbox.filter((item) => !item.read)
  const assistantName = (id: string) =>
    data.assistants.find((assistant) => assistant.id === id)?.name ?? 'Deleted agent'
  const projectName = (id?: string | null) =>
    id
      ? (overview.projects.find((project) => project.id === id)?.name ?? 'Deleted project')
      : 'any project'

  return (
    <>
      <Section
        title="Activity inbox"
        description="Results, failures, questions, and approvals from agent work."
      >
        {overview.inbox.length === 0 && <p className="muted">Nothing yet.</p>}
        {unread.length > 0 && (
          <button
            className="button ghost"
            onClick={() => void agentApi.markRead(unread.map((item) => item.id)).then(onChanged)}
          >
            Mark all as read
          </button>
        )}
        <ul className="inbox-list">
          {overview.inbox.slice(0, 30).map((item) => (
            <li key={item.id} className={item.read ? 'read' : 'unread'}>
              <span>
                <strong>{item.title}</strong>
                <small className="muted"> · {new Date(item.createdAt).toLocaleString()}</small>
                {item.body && <span className="inbox-body">{item.body.split('\n')[0]}</span>}
              </span>
              {item.taskId && (
                <button className="button ghost" onClick={() => onOpenTask(item.taskId!)}>
                  Open
                </button>
              )}
            </li>
          ))}
        </ul>
      </Section>

      <Section
        title="Standing permissions"
        description="Actions you allowed an agent to repeat without asking. Revoking takes effect immediately; running tasks of that agent pause."
      >
        {grants.filter((grant) => !grant.revokedAt).length === 0 && (
          <p className="muted">None. Agents ask before every action that needs permission.</p>
        )}
        <ul className="tool-list">
          {grants
            .filter((grant) => !grant.revokedAt)
            .map((grant) => (
              <li key={grant.id} className="tool-row">
                <div className="tool-row-head">
                  <span>
                    <strong>{grant.capability}</strong>{' '}
                    <small className="muted">
                      {assistantName(grant.assistantId)} · {projectName(grant.projectId)} · since{' '}
                      {new Date(grant.createdAt).toLocaleDateString()}
                    </small>
                  </span>
                  <button
                    className="button danger-ghost"
                    onClick={() => void agentApi.revokeGrant(grant.id).then(load).then(onChanged)}
                  >
                    Revoke
                  </button>
                </div>
              </li>
            ))}
        </ul>
      </Section>

      <Section
        title="Web search"
        description="Web tools are off until you turn them on for an agent. Searches go to the provider below; pages are fetched directly from public addresses only."
      >
        <Segmented
          label="Search provider"
          value={provider}
          onChange={setProvider}
          options={[
            { value: 'wikipedia', label: 'Wikipedia (no account)' },
            { value: 'brave', label: 'Brave Search (API key)' },
            { value: 'searxng', label: 'My SearXNG' },
          ]}
        />
        {provider === 'wikipedia' && (
          <label className="stacked-field">
            <span className="label">Wikipedia language code</span>
            <input
              value={language}
              maxLength={12}
              onChange={(event) => setLanguage(event.target.value)}
            />
          </label>
        )}
        {provider === 'brave' && (
          <label className="stacked-field">
            <span className="label">Brave Search API key (stored in your system keychain)</span>
            <input
              type="password"
              value={braveKey}
              autoComplete="off"
              onChange={(event) => setBraveKey(event.target.value)}
            />
          </label>
        )}
        {provider === 'searxng' && (
          <label className="stacked-field">
            <span className="label">SearXNG address (JSON output enabled)</span>
            <input
              value={baseUrl}
              placeholder="https://search.example.org"
              onChange={(event) => setBaseUrl(event.target.value)}
            />
          </label>
        )}
        <button className="button secondary" onClick={() => void saveWeb()}>
          Save search settings
        </button>
      </Section>

      <Section title="Where agent work runs">
        <ul className="capability-list">
          {(
            [
              ['Project folders', overview.capabilities.workspace],
              ['Web search and pages', overview.capabilities.web],
              ['Sandboxed code', overview.capabilities.sandbox],
              ['Local MCP servers', overview.capabilities.mcpStdio],
              ['Scheduling', overview.capabilities.scheduling],
            ] as const
          ).map(([label, availability]) => (
            <li key={label}>
              <strong>{label}:</strong> {availability.available ? 'available' : 'unavailable'}
              {availability.reason && <span className="muted"> — {availability.reason}</span>}
              {availability.note && <span className="muted"> — {availability.note}</span>}
            </li>
          ))}
        </ul>
      </Section>
    </>
  )
}
