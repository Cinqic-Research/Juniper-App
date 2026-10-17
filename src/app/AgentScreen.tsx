import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AppData, Assistant } from '../types'
import type {
  AgentOverview,
  AgentTask,
  Approval,
  Artifact,
  Source,
  TaskDetail,
  TaskStatus,
} from '../types/agent'
import { agentApi, agentAvailable, deviceTimeZone, errorText, onAgentEvent } from '../lib/agent'
import { Markdown } from '../lib/markdown'
import {
  AgentSettingsDialog,
  PermissionsPanel,
  PluginsPanel,
  ProjectsPanel,
  SchedulesPanel,
  SkillsPanel,
} from './AgentManage'
import { Icon } from './icons'
import { isChatSelectable } from './model-labels'
import { Modal, useDialogs } from './overlays'
import { EmptyState, LocationBadge, PageHeader, Segmented } from './ui'

type View = 'tasks' | 'projects' | 'schedules' | 'skills' | 'permissions'

const STATUS_LABEL: Record<TaskStatus, string> = {
  queued: 'Queued',
  running: 'Working',
  'waiting-input': 'Needs your answer',
  'waiting-approval': 'Needs your approval',
  paused: 'Paused',
  interrupted: 'Interrupted',
  completed: 'Completed',
  failed: 'Failed',
  cancelled: 'Cancelled',
}

const ACTIVE: TaskStatus[] = ['queued', 'running', 'waiting-input', 'waiting-approval']

/** A finished task whose answer claims work no tool did (see TaskResult). */
function needsReview(task: Pick<AgentTask, 'status' | 'result'>) {
  return task.status === 'completed' && task.result?.outcome === 'needs-review'
}

export function StatusChip({ task }: { task: Pick<AgentTask, 'status' | 'result'> }) {
  return needsReview(task) ? (
    <span className="task-status waiting-input">Needs review</span>
  ) : (
    <span className={`task-status ${task.status}`}>{STATUS_LABEL[task.status]}</span>
  )
}

function BetaBadge() {
  return (
    <span className="beta-badge" title="Agent is in beta">
      Beta
    </span>
  )
}

export function AgentScreen({
  data,
  mobile,
  onBrowseModels,
}: {
  data: AppData
  mobile: boolean
  onBrowseModels: () => void
}) {
  const dialogs = useDialogs()
  const [overview, setOverview] = useState<AgentOverview | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [view, setView] = useState<View>('tasks')
  const [selectedTask, setSelectedTask] = useState<string | null>(null)
  const [settingsFor, setSettingsFor] = useState<Assistant | null>(null)
  const refreshing = useRef(false)

  const refresh = useCallback(async () => {
    if (!agentAvailable || refreshing.current) return
    refreshing.current = true
    try {
      const next = await agentApi.overview()
      setOverview(next)
      setLoadError(null)
      if (!next.settings.timezone) await agentApi.setSettings({ timezone: deviceTimeZone() })
    } catch (error) {
      setLoadError(errorText(error))
    } finally {
      refreshing.current = false
    }
  }, [])

  useEffect(() => {
    void refresh()
    let stop: (() => void) | undefined
    let disposed = false
    void onAgentEvent(() => void refresh()).then((unlisten) => {
      if (disposed) unlisten()
      else stop = unlisten
    })
    return () => {
      disposed = true
      stop?.()
    }
  }, [refresh])

  if (!agentAvailable) {
    return (
      <div className="page agent-page">
        <PageHeader title="Agent" description="Agent tasks run in the Juniper desktop app." />
        <EmptyState title="Open the Juniper app to use Agent">
          The browser preview has no native runtime, so no task can run here.
        </EmptyState>
      </div>
    )
  }

  if (selectedTask) {
    return (
      <div className="page agent-page">
        <TaskView
          taskId={selectedTask}
          data={data}
          onBack={() => setSelectedTask(null)}
          onOpenTask={setSelectedTask}
        />
      </div>
    )
  }

  return (
    <div className="page agent-page">
      <PageHeader
        title="Agent"
        description={
          <>
            <BetaBadge /> Give a goal to a personal agent. It works with the tools and folders you
            allow, and shows every step.
          </>
        }
        action={
          overview && (
            <button
              className={`button ${overview.stopped ? 'primary' : 'danger-ghost'}`}
              onClick={() =>
                void agentApi
                  .stopAll(!overview.stopped)
                  .then(refresh)
                  .catch((error) => dialogs.notify(errorText(error)))
              }
            >
              {overview.stopped ? 'Resume agent work' : 'Stop all'}
            </button>
          )
        }
      />
      <p className="agent-beta-note">
        Beta: tasks run on this device while Juniper is running and stop when you quit it. Results
        depend on the model; a task is marked complete only when the host saw it finish.
        {overview?.stopped && <strong> Agent work is stopped. Nothing new will start.</strong>}
      </p>
      {loadError && (
        <p className="warning-line" role="alert">
          {loadError}
        </p>
      )}
      <div className="agent-views">
        <Segmented<View>
          label="Agent sections"
          value={view}
          onChange={setView}
          options={[
            { value: 'tasks', label: 'Tasks' },
            { value: 'projects', label: 'Projects' },
            { value: 'schedules', label: mobile ? 'Schedule' : 'Schedules' },
            { value: 'skills', label: mobile ? 'Skills' : 'Skills & plugins' },
            { value: 'permissions', label: mobile ? 'Access' : 'Permissions' },
          ]}
        />
      </div>
      {!overview ? (
        !loadError && <p className="muted">Loading…</p>
      ) : view === 'tasks' ? (
        <TasksPanel
          data={data}
          overview={overview}
          onOpenTask={setSelectedTask}
          onOpenSettings={setSettingsFor}
          onBrowseModels={onBrowseModels}
          onChanged={refresh}
        />
      ) : view === 'projects' ? (
        <ProjectsPanel overview={overview} onChanged={refresh} />
      ) : view === 'schedules' ? (
        <SchedulesPanel data={data} overview={overview} onChanged={refresh} />
      ) : view === 'skills' ? (
        <>
          <SkillsPanel onChanged={refresh} />
          <PluginsPanel overview={overview} onChanged={refresh} />
        </>
      ) : (
        <PermissionsPanel
          data={data}
          overview={overview}
          onChanged={refresh}
          onOpenTask={setSelectedTask}
        />
      )}
      {settingsFor && (
        <AgentSettingsDialog
          assistant={settingsFor}
          data={data}
          overview={overview}
          onClose={() => {
            setSettingsFor(null)
            void refresh()
          }}
        />
      )}
    </div>
  )
}

function TasksPanel({
  data,
  overview,
  onOpenTask,
  onOpenSettings,
  onBrowseModels,
  onChanged,
}: {
  data: AppData
  overview: AgentOverview
  onOpenTask: (id: string) => void
  onOpenSettings: (assistant: Assistant) => void
  onBrowseModels: () => void
  onChanged: () => Promise<void>
}) {
  const dialogs = useDialogs()
  const [assistantId, setAssistantId] = useState(
    data.settings.defaultAssistantId ?? data.assistants[0]?.id ?? '',
  )
  const [projectId, setProjectId] = useState<string>('')
  const [goal, setGoal] = useState('')
  const [starting, setStarting] = useState(false)
  const assistant = data.assistants.find((item) => item.id === assistantId) ?? data.assistants[0]
  const profile = overview.profiles.find((item) => item.profile.assistantId === assistant?.id)
  const modelId = profile?.profile.modelProfileId ?? assistant?.modelProfileId ?? null
  const model = data.models.find((item) => item.id === modelId && isChatSelectable(item))
  const toolsQualified = model?.capabilities.tools === 'supported'
  const allowUnqualified = profile?.profile.allowUnqualifiedModel ?? false
  const projects = overview.projects.filter((project) => !project.archived)
  // A subtask's request surfaces on its top-level task.
  const waitingRoots = new Set(overview.pendingApprovals.map((approval) => approval.rootId))
  const attention = overview.tasks.filter(
    (task) =>
      task.status === 'waiting-approval' ||
      task.status === 'waiting-input' ||
      waitingRoots.has(task.id),
  )
  const active = overview.tasks.filter(
    (task) => ACTIVE.includes(task.status) && !attention.includes(task),
  )
  const recent = overview.tasks.filter((task) => !ACTIVE.includes(task.status)).slice(0, 30)

  async function start() {
    if (!assistant || !goal.trim()) return
    setStarting(true)
    try {
      const id = await agentApi.startTask({
        assistantId: assistant.id,
        goal,
        projectId: projectId || null,
      })
      setGoal('')
      await onChanged()
      onOpenTask(id)
    } catch (error) {
      void dialogs.notify(errorText(error), 'The task could not start')
    } finally {
      setStarting(false)
    }
  }

  const blocker = !assistant
    ? 'Create an assistant in Settings first.'
    : !model
      ? `Choose a model for ${assistant.name} in its agent settings.`
      : !toolsQualified && !allowUnqualified
        ? `${model.displayName} has no measured tool-use evidence. Choose a qualified model, or allow it in this agent's settings.`
        : null

  return (
    <>
      <section className="agent-composer" aria-label="New task">
        <div className="agent-composer-row">
          <label>
            <span className="label">Agent</span>
            <select
              value={assistant?.id ?? ''}
              onChange={(event) => setAssistantId(event.target.value)}
            >
              {data.assistants.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.avatar} {item.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            <span className="label">Project</span>
            <select value={projectId} onChange={(event) => setProjectId(event.target.value)}>
              <option value="">No project (no folder access)</option>
              {projects.map((project) => (
                <option key={project.id} value={project.id}>
                  {project.name}
                </option>
              ))}
            </select>
          </label>
          {assistant && (
            <button className="button ghost" onClick={() => onOpenSettings(assistant)}>
              <Icon name="settings" size={16} /> Agent settings
            </button>
          )}
        </div>
        <label className="agent-goal">
          <span className="label">What should {assistant?.name ?? 'the agent'} do?</span>
          <textarea
            value={goal}
            rows={3}
            maxLength={20000}
            placeholder="For example: Summarize the documents in my Budget project into a one-page report with sources."
            onChange={(event) => setGoal(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) void start()
            }}
          />
        </label>
        <div className="agent-composer-footer">
          <span className="muted small">
            {model ? (
              <>
                Model: {model.displayName} <LocationBadge location={model.executionLocation} />
                {!toolsQualified && (
                  <span className="warning-inline"> Not qualified for tools</span>
                )}
              </>
            ) : (
              'No model selected'
            )}
          </span>
          {blocker && !model ? (
            <button className="button secondary" onClick={onBrowseModels}>
              Browse models
            </button>
          ) : null}
          <button
            className="button primary"
            disabled={Boolean(blocker) || !goal.trim() || starting || overview.stopped}
            onClick={() => void start()}
          >
            {starting ? 'Starting…' : 'Start task'}
          </button>
        </div>
        {blocker && <p className="warning-line">{blocker}</p>}
      </section>

      {attention.length > 0 && (
        <TaskList title="Needs you" tasks={attention} data={data} onOpen={onOpenTask} />
      )}
      {active.length > 0 && (
        <TaskList title="In progress" tasks={active} data={data} onOpen={onOpenTask} />
      )}
      <TaskList
        title="Recent"
        tasks={recent}
        data={data}
        onOpen={onOpenTask}
        empty="No tasks yet. Describe a goal above to start one."
      />
    </>
  )
}

function TaskList({
  title,
  tasks,
  data,
  onOpen,
  empty,
}: {
  title: string
  tasks: AgentTask[]
  data: AppData
  onOpen: (id: string) => void
  empty?: string
}) {
  return (
    <section className="agent-task-section" aria-label={title}>
      <h2>{title}</h2>
      {tasks.length === 0 && empty ? <p className="muted">{empty}</p> : null}
      <ul className="agent-task-list">
        {tasks.map((task) => {
          const assistant = data.assistants.find((item) => item.id === task.assistantId)
          return (
            <li key={task.id}>
              <button className="agent-task-row" onClick={() => onOpen(task.id)}>
                <span className="agent-task-avatar" aria-hidden="true">
                  {assistant?.avatar ?? '◆'}
                </span>
                <span className="agent-task-main">
                  <strong>{task.title}</strong>
                  <small>
                    {task.statusReason ?? task.currentAction ?? ''}
                    {task.scheduleRunId ? ' · Scheduled' : ''}
                  </small>
                </span>
                <StatusChip task={task} />
              </button>
            </li>
          )
        })}
      </ul>
    </section>
  )
}

function sourceKind(source: Source): string {
  if (source.kind === 'web-result') return 'Search result (snippet only)'
  if (source.kind === 'web-page') return 'Web page read'
  if (source.kind === 'project-file') return 'Project file'
  if (source.kind === 'plugin') return 'Plugin result'
  return source.kind
}

function TaskView({
  taskId,
  data,
  onBack,
  onOpenTask,
}: {
  taskId: string
  data: AppData
  onBack: () => void
  onOpenTask: (id: string) => void
}) {
  const dialogs = useDialogs()
  const [detail, setDetail] = useState<TaskDetail | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [reply, setReply] = useState('')
  const [preview, setPreview] = useState<Artifact | null>(null)
  const [busy, setBusy] = useState(false)

  const load = useCallback(async () => {
    try {
      setDetail(await agentApi.task(taskId))
      setError(null)
    } catch (failure) {
      setError(errorText(failure))
    }
  }, [taskId])

  useEffect(() => {
    void load()
    let stop: (() => void) | undefined
    let disposed = false
    void onAgentEvent(() => void load()).then((unlisten) => {
      if (disposed) unlisten()
      else stop = unlisten
    })
    return () => {
      disposed = true
      stop?.()
    }
  }, [load])

  async function act(action: () => Promise<unknown>) {
    setBusy(true)
    try {
      await action()
      await load()
    } catch (failure) {
      void dialogs.notify(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  if (error && !detail) {
    return (
      <>
        <PageHeader title="Task" onBack={onBack} backLabel="Back to tasks" />
        <p className="warning-line">{error}</p>
      </>
    )
  }
  if (!detail) return <p className="muted">Loading…</p>
  const { task } = detail
  const assistant = data.assistants.find((item) => item.id === task.assistantId)
  const model = data.models.find((item) => item.id === task.provenance.modelProfileId)
  const pendingApprovals = detail.approvals.filter(
    (approval) => approval.status === 'pending' && approval.taskId === task.id,
  )
  const childApprovals = detail.approvals.filter(
    (approval) => approval.status === 'pending' && approval.taskId !== task.id,
  )
  const question =
    task.pending?.wait?.kind === 'input' && !task.pending.wait.answer
      ? task.pending.wait.question
      : null
  const running = task.status === 'running' || task.status === 'queued'
  const terminal = ['completed', 'failed', 'cancelled'].includes(task.status)
  const sourcesById = new Map(detail.sources.map((source) => [source.id, source]))

  return (
    <article className="task-view" aria-label={task.title}>
      <PageHeader
        title={task.title}
        onBack={onBack}
        backLabel="Back to tasks"
        description={
          <>
            {assistant?.avatar} {assistant?.name ?? 'Agent'} · <StatusChip task={task} />
            {task.role && <> · {task.role} subtask</>}
          </>
        }
      />
      <div className="task-status-line" role="status" aria-live="polite">
        {task.status === 'running' && <span className="task-spinner" aria-hidden="true" />}
        <span>{task.statusReason ?? task.currentAction ?? STATUS_LABEL[task.status]}</span>
      </div>

      <div className="button-row task-controls">
        {(task.status === 'running' || task.status === 'queued') && (
          <button
            className="button secondary"
            disabled={busy}
            onClick={() => void act(() => agentApi.pause(task.id))}
          >
            Pause
          </button>
        )}
        {(task.status === 'paused' || task.status === 'interrupted') && (
          <button
            className="button primary"
            disabled={busy}
            onClick={() => void act(() => agentApi.resume(task.id))}
          >
            Resume
          </button>
        )}
        {!terminal && (
          <button
            className="button danger-ghost"
            disabled={busy}
            onClick={() =>
              void dialogs
                .confirm({
                  title: 'Cancel this task?',
                  message: 'It stops now, along with any subtasks. Work already saved stays.',
                  confirmLabel: 'Cancel task',
                  cancelLabel: 'Keep working',
                  danger: true,
                })
                .then((yes) => {
                  if (yes) void act(() => agentApi.cancel(task.id))
                })
            }
          >
            Cancel
          </button>
        )}
      </div>

      {pendingApprovals.map((approval) => (
        <ApprovalCard
          key={approval.id}
          approval={approval}
          projectScoped={Boolean(task.projectId)}
          busy={busy}
          onDecide={(decision) => void act(() => agentApi.decide(approval.id, decision))}
        />
      ))}
      {childApprovals.length > 0 && (
        <p className="notice">
          A subtask is waiting for your approval.{' '}
          <button className="link-button" onClick={() => onOpenTask(childApprovals[0]!.taskId)}>
            Review it
          </button>
        </p>
      )}

      {question && (
        <section className="task-card task-question" aria-label="Question from the agent">
          <h2>{assistant?.name ?? 'The agent'} asks</h2>
          <p>{question}</p>
          <textarea
            value={reply}
            rows={2}
            onChange={(event) => setReply(event.target.value)}
            aria-label="Your answer"
          />
          <button
            className="button primary"
            disabled={!reply.trim() || busy}
            onClick={() =>
              void act(async () => {
                await agentApi.answer(task.id, reply)
                setReply('')
              })
            }
          >
            Send answer
          </button>
        </section>
      )}

      {task.plan.length > 0 && (
        <section className="task-card" aria-label="Plan">
          <h2>Plan</h2>
          <ol className="task-plan">
            {task.plan.map((item, index) => (
              <li key={index} className={item.status}>
                <span aria-hidden="true">
                  {item.status === 'done' ? '✓' : item.status === 'in-progress' ? '▸' : '○'}
                </span>{' '}
                {item.text}
                <span className="visually-hidden"> ({item.status})</span>
              </li>
            ))}
          </ol>
        </section>
      )}

      {task.result && (
        <section className="task-card task-result" aria-label="Result">
          <h2>
            {task.result.outcome === 'partial'
              ? 'Partial result'
              : task.result.outcome === 'needs-review'
                ? 'Needs review'
                : task.result.outcome === 'deliverable'
                  ? 'Result and documents'
                  : 'Answer'}
          </h2>
          {task.result.summary && <Markdown content={task.result.summary} />}
          {task.result.unverified.some((item) => item.startsWith('claim: ')) && (
            <p className="warning-line">
              Not done by Juniper: the answer says it{' '}
              {task.result.unverified
                .filter((item) => item.startsWith('claim: '))
                .map((item) => item.slice('claim: '.length))
                .join(' and ')}
              , but no tool in this task did that. Check before relying on it.
            </p>
          )}
          {task.result.unverified.some((item) => !item.startsWith('claim: ')) && (
            <p className="warning-line">
              Not verified:{' '}
              {task.result.unverified.filter((item) => !item.startsWith('claim: ')).join(', ')} —
              these citations do not match a source this task retrieved.
            </p>
          )}
        </section>
      )}

      {detail.artifacts.length > 0 && (
        <section className="task-card" aria-label="Documents">
          <h2>Documents</h2>
          <ul className="artifact-list">
            {detail.artifacts.map((artifact) => (
              <li key={artifact.id}>
                <span>
                  <strong>{artifact.title}</strong>{' '}
                  <small className="muted">
                    {artifact.fileName} · {Math.max(1, Math.round(artifact.sizeBytes / 1024))} KB
                    {artifact.sourceIds.length > 0 &&
                      ` · cites ${artifact.sourceIds.map((id) => id.split(':').pop()).join(', ')}`}
                  </small>
                </span>
                <span className="button-row">
                  <button className="button ghost" onClick={() => setPreview(artifact)}>
                    Open
                  </button>
                  <button
                    className="button ghost"
                    onClick={() => void act(() => agentApi.exportArtifact(artifact.id))}
                  >
                    Export
                  </button>
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {detail.sources.length > 0 && (
        <section className="task-card" aria-label="Sources">
          <h2>Sources</h2>
          <ul className="source-list">
            {detail.sources.map((source) => (
              <li key={source.id}>
                <strong>[{source.id.split(':').pop()}]</strong> {source.title}{' '}
                <small className="muted">
                  {sourceKind(source)}
                  {(source.finalUrl ?? source.url) ? ` · ${source.finalUrl ?? source.url}` : ''}
                  {source.location ? ` · ${source.location}` : ''}
                </small>
              </li>
            ))}
          </ul>
        </section>
      )}

      {detail.children.length > 0 && (
        <section className="task-card" aria-label="Subtasks">
          <h2>Subtasks</h2>
          <ul className="agent-task-list">
            {detail.children.map((child) => (
              <li key={child.id}>
                <button className="agent-task-row" onClick={() => onOpenTask(child.id)}>
                  <span className="agent-task-main">
                    <strong>{child.title}</strong>
                    <small>{child.statusReason ?? child.currentAction ?? ''}</small>
                  </span>
                  <StatusChip task={child} />
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {task.parentId == null && (terminal || task.status === 'paused') && (
        <section className="task-card" aria-label="Follow up">
          <h2>Add to this task</h2>
          <textarea
            value={reply}
            rows={2}
            maxLength={4000}
            placeholder="Ask for a change or a next step. The original goal still applies."
            onChange={(event) => setReply(event.target.value)}
            aria-label="Follow-up"
          />
          <button
            className="button primary"
            disabled={!reply.trim() || busy || running}
            onClick={() =>
              void act(async () => {
                await agentApi.steer(task.id, reply)
                setReply('')
              })
            }
          >
            Continue task
          </button>
        </section>
      )}

      <details className="task-card task-activity">
        <summary>Activity ({detail.events.length})</summary>
        <ol>
          {detail.events.map((event) => (
            <li key={event.id}>
              <time dateTime={event.createdAt}>
                {new Date(event.createdAt).toLocaleTimeString([], {
                  hour: 'numeric',
                  minute: '2-digit',
                })}
              </time>{' '}
              {event.summary}
            </li>
          ))}
        </ol>
      </details>

      <details className="task-card task-activity">
        <summary>Details</summary>
        <dl className="detail-list compact">
          <dt>Goal</dt>
          <dd>{task.goal}</dd>
          <dt>Model</dt>
          <dd>
            {model?.displayName ?? task.provenance.modelId} ({task.provenance.providerKind},{' '}
            {task.provenance.executionLocation})
          </dd>
          <dt>Permissions preset</dt>
          <dd>{task.provenance.autonomy}</dd>
          <dt>Tools offered</dt>
          <dd>{task.provenance.tools.join(', ') || 'none yet'}</dd>
          <dt>Usage</dt>
          <dd>
            {task.usage.steps}/{task.budget.maxSteps} steps · {task.usage.toolCalls} tool calls ·{' '}
            {task.usage.inputTokens + task.usage.outputTokens}
            {task.usage.tokensEstimated ? ' estimated' : ''} tokens of {task.budget.maxTokens}
          </dd>
        </dl>
      </details>

      {preview && (
        <ArtifactPreview
          artifact={preview}
          sources={sourcesById}
          onClose={() => setPreview(null)}
        />
      )}
    </article>
  )
}

function ApprovalCard({
  approval,
  projectScoped,
  busy,
  onDecide,
}: {
  approval: Approval
  projectScoped: boolean
  busy: boolean
  onDecide: (decision: 'allow-once' | 'allow-standing' | 'deny') => void
}) {
  const [headline, ...rest] = approval.preview.split('\n')
  return (
    <section className="task-card task-approval" aria-label="Approval needed" role="alertdialog">
      <h2>Approve this action?</h2>
      <p>{headline}</p>
      {rest.join('\n').trim() && <pre className="approval-preview">{rest.join('\n').trim()}</pre>}
      <div className="button-row">
        <button className="button primary" disabled={busy} onClick={() => onDecide('allow-once')}>
          Allow once
        </button>
        {approval.standingAllowed && (
          <button
            className="button secondary"
            disabled={busy}
            onClick={() => onDecide('allow-standing')}
          >
            {approval.tool === 'code.python'
              ? `Always allow Python runs for this agent${projectScoped ? ' in this project' : ' outside projects'} (any code)`
              : approval.tool === 'code.run'
                ? 'Always allow this program in this folder for this agent'
                : `Always allow for this agent${projectScoped ? ' in this project' : ''}`}
          </button>
        )}
        <button className="button danger-ghost" disabled={busy} onClick={() => onDecide('deny')}>
          Deny
        </button>
      </div>
      <p className="muted small">
        Expires {new Date(approval.expiresAt).toLocaleString()}. An unanswered request is never
        treated as approval.
      </p>
    </section>
  )
}

function ArtifactPreview({
  artifact,
  sources,
  onClose,
}: {
  artifact: Artifact
  sources: Map<string, Source>
  onClose: () => void
}) {
  const [text, setText] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    agentApi.artifact(artifact.id).then(
      (value) => setText(value.text + (value.truncated ? '\n\n… (preview shortened)' : '')),
      (failure) => setError(errorText(failure)),
    )
  }, [artifact.id])
  const cited = useMemo(
    () =>
      artifact.sourceIds
        .map((id) => sources.get(id))
        .filter((source): source is Source => Boolean(source)),
    [artifact.sourceIds, sources],
  )
  return (
    <Modal
      open
      onClose={onClose}
      title={artifact.title}
      description={artifact.fileName}
      variant="sheet"
    >
      {error && <p className="warning-line">{error}</p>}
      {text === null && !error && <p className="muted">Loading…</p>}
      {text !== null &&
        (artifact.mediaType === 'text/markdown' ? (
          <div className="artifact-preview">
            <Markdown content={text} />
          </div>
        ) : (
          <pre className="artifact-preview">{text}</pre>
        ))}
      {cited.length > 0 && (
        <>
          <h3>Cited sources</h3>
          <ul className="source-list">
            {cited.map((source) => (
              <li key={source.id}>
                [{source.id.split(':').pop()}] {source.title}{' '}
                <small className="muted">{source.finalUrl ?? source.url ?? source.location}</small>
              </li>
            ))}
          </ul>
        </>
      )}
    </Modal>
  )
}
