import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { initialAppData } from '../lib/defaults'
import type { AgentOverview, AgentTask, TaskDetail } from '../types/agent'

// The Agent screen reads everything from the native host. These tests stand
// in for the host at the IPC boundary and check what the screen shows and
// which intents it sends — never that it decides anything itself.

type Invocation = { command: string; args?: Record<string, unknown> }
let invocations: Invocation[] = []
let respond: (command: string, args?: Record<string, unknown>) => unknown = () => null

beforeAll(() => {
  Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', {
    configurable: true,
    value: { unregisterListener: () => undefined },
  })
  Object.defineProperty(window, '__TAURI_INTERNALS__', {
    configurable: true,
    value: {
      transformCallback: () => 0,
      invoke: async (command: string, args?: Record<string, unknown>) => {
        invocations.push({ command, args })
        return respond(command, args)
      },
    },
  })
})

const data = initialAppData()
const assistant = data.assistants[0]!

const task: AgentTask = {
  id: 'task-1',
  rootId: 'task-1',
  depth: 0,
  assistantId: assistant.id,
  projectId: 'project-1',
  title: 'Write the budget summary',
  goal: 'Write the budget summary',
  status: 'waiting-approval',
  statusReason: 'Approve or deny: Write a project file',
  plan: [
    { text: 'Read notes', status: 'done' },
    { text: 'Write summary', status: 'in-progress' },
  ],
  budget: {
    maxSteps: 16,
    maxToolCalls: 48,
    maxTokens: 120000,
    maxSeconds: 1800,
    maxArtifacts: 10,
    maxChildren: 4,
    maxDepth: 2,
    maxRetriesPerCause: 2,
  },
  usage: {
    steps: 2,
    toolCalls: 3,
    inputTokens: 900,
    outputTokens: 120,
    tokensEstimated: true,
    activeMs: 4000,
    artifacts: 0,
    children: 0,
    retries: {},
  },
  provenance: {
    assistantId: assistant.id,
    agentProfileRevision: 1,
    modelProfileId: 'model-1',
    modelId: 'fixture',
    providerKind: 'openai-compatible',
    executionLocation: 'on-device',
    skills: [],
    autonomy: 'ask',
    tools: ['workspace.write'],
  },
  revision: 4,
  createdAt: '2026-10-15T07:00:00Z',
  updatedAt: '2026-10-15T07:01:00Z',
}

const overview: AgentOverview = {
  profiles: [],
  projects: [],
  tasks: [task],
  pendingApprovals: [],
  inbox: [],
  stopped: false,
  timeZone: 'America/New_York',
  settings: { timezone: 'America/New_York' },
  capabilities: {
    workspace: { available: true },
    web: { available: true },
    sandbox: { available: false, reason: 'Install bubblewrap (bwrap) to run code in a sandbox.' },
    mcpStdio: { available: true },
    mcpHttp: { available: true },
    scheduling: { available: true, note: 'Scheduled tasks run while Juniper is running.' },
  },
  presets: [{ id: 'ask', description: 'Asks first.' }],
}

const detail: TaskDetail = {
  task,
  events: [
    { id: 1, taskId: 'task-1', kind: 'status', summary: 'Queued', createdAt: task.createdAt },
  ],
  actions: [],
  approvals: [
    {
      id: 'approval-1',
      taskId: 'task-1',
      rootId: 'task-1',
      actionId: 'action-1',
      tool: 'workspace.write',
      capability: 'workspace.write',
      preview: 'Create “out/summary.md” in project:\n\n+Q4 budget: 12,000 dollars.',
      standingAllowed: true,
      status: 'pending',
      createdAt: task.createdAt,
      expiresAt: '2026-10-18T07:00:00Z',
    },
  ],
  artifacts: [],
  sources: [],
  children: [],
  messages: [],
}

describe('Agent screen (Beta)', () => {
  let container: HTMLDivElement
  let root: Root

  async function mount() {
    const { AgentScreen } = await import('./AgentScreen')
    const { DialogProvider } = await import('./overlays')
    container = document.createElement('div')
    document.body.append(container)
    await act(async () => {
      root = createRoot(container)
      root.render(
        <DialogProvider>
          <AgentScreen data={data} mobile={false} onBrowseModels={() => undefined} />
        </DialogProvider>,
      )
    })
    await settle()
  }

  async function settle() {
    for (let tick = 0; tick < 6; tick += 1) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5))
      })
    }
  }

  function button(label: string): HTMLButtonElement {
    const found = Array.from(container.querySelectorAll('button')).find((item) =>
      item.textContent?.includes(label),
    )
    if (!found) throw new Error(`Button not found: ${label}`)
    return found as HTMLButtonElement
  }

  afterEach(() => {
    act(() => root?.unmount())
    container?.remove()
    invocations = []
  })

  it('labels the beta, lists tasks that need the user, and sends only an approval intent', async () => {
    respond = (command) => {
      if (command === 'agent_overview') return overview
      if (command === 'task_detail') return detail
      return null
    }
    await mount()
    expect(container.textContent).toContain('Beta')
    expect(container.textContent).toContain('Needs you')
    expect(container.textContent).toContain('Needs your approval')
    await act(async () => button('Write the budget summary').click())
    await settle()
    expect(container.textContent).toContain('Approve this action?')
    expect(container.querySelector('.approval-preview')?.textContent).toContain(
      '+Q4 budget: 12,000 dollars.',
    )
    expect(container.textContent).toContain('Always allow for this agent in this project')
    await act(async () => button('Allow once').click())
    await settle()
    const decision = invocations.find((call) => call.command === 'approval_decide')
    expect(decision?.args).toEqual({ approvalId: 'approval-1', decision: 'allow-once' })
    // The screen never writes task state itself.
    expect(invocations.some((call) => call.command === 'save_app_data')).toBe(false)
  })

  it('says how broad a standing Python grant is', async () => {
    const pythonApproval = {
      ...detail.approvals[0]!,
      tool: 'code.python',
      capability: 'code.python',
      preview: 'Run this Python in the sandbox (no network, no access to your files):\n\nprint(1)',
    }
    respond = (command) => {
      if (command === 'agent_overview') return overview
      if (command === 'task_detail') return { ...detail, approvals: [pythonApproval] }
      return null
    }
    await mount()
    await act(async () => button('Write the budget summary').click())
    await settle()
    expect(container.textContent).toContain(
      'Always allow Python runs for this agent in this project (any code)',
    )
  })

  it('shows a host-flagged unperformed claim as Needs review, never Completed', async () => {
    const reviewed: AgentTask = {
      ...task,
      status: 'completed',
      statusReason:
        'The answer says it saved or changed a file, but no tool in this task did that. Check the result before relying on it.',
      result: {
        outcome: 'needs-review',
        summary: 'The summary has already been saved as `summary.md` in the project folder.',
        artifactIds: [],
        sourceIds: [],
        unverified: ['claim: saved or changed a file'],
      },
    }
    respond = (command) => {
      if (command === 'agent_overview') return { ...overview, tasks: [reviewed] }
      if (command === 'task_detail') return { ...detail, task: reviewed, approvals: [] }
      return null
    }
    await mount()
    expect(container.querySelector('.task-status')?.textContent).toBe('Needs review')
    await act(async () => button('Write the budget summary').click())
    await settle()
    const chips = Array.from(container.querySelectorAll('.task-status')).map(
      (chip) => chip.textContent,
    )
    expect(chips).toContain('Needs review')
    expect(chips).not.toContain('Completed')
    expect(container.textContent).toContain('Not done by Juniper')
    expect(container.textContent).not.toContain('these citations do not match')
  })

  it('does not offer a standing grant for once-only actions such as memory changes', async () => {
    respond = (command) => {
      if (command === 'agent_overview') return overview
      if (command === 'task_detail')
        return {
          ...detail,
          approvals: [{ ...detail.approvals[0]!, tool: 'memory.save', standingAllowed: false }],
        }
      return null
    }
    await mount()
    await act(async () => button('Write the budget summary').click())
    await settle()
    expect(container.textContent).toContain('Allow once')
    expect(container.textContent).not.toContain('Always allow')
  })

  it('shows platform limits instead of hiding them', async () => {
    respond = (command) =>
      command === 'agent_overview' ? overview : command === 'grants_list' ? [] : null
    await mount()
    await act(async () =>
      container
        .querySelector<HTMLInputElement>('input[type="radio"][value="permissions"]')!
        .click(),
    )
    await settle()
    expect(container.textContent).toContain('Sandboxed code: unavailable')
    expect(container.textContent).toContain('Install bubblewrap')
    expect(container.textContent).toContain('Scheduled tasks run while Juniper is running.')
  })
})
