// IPC for the Agent (Beta) screen. Every call is validated by the native
// host; nothing here decides permissions or task state.
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { AppData, Assistant } from '../types'
import type {
  AgentEvent,
  AgentOverview,
  AgentProfile,
  AgentTask,
  AgentToolInfo,
  Grant,
  PluginSummary,
  Project,
  Schedule,
  ScheduleSpec,
  Skill,
  TaskDetail,
} from '../types/agent'
import { compilePersonality } from './context'
import { runningInTauri } from './runtime'

export const agentAvailable = runningInTauri

const RESPONSE_LENGTH: Record<Assistant['responseLength'], string> = {
  concise: 'Keep replies short.',
  balanced: 'Use moderate length.',
  detailed: 'Give detailed replies when useful.',
}

/**
 * The assistant's style as profile text for agent tasks. It shapes tone and
 * format only; the host places it below its own rules.
 */
export function composeAgentProfile(assistant: Assistant): string {
  return [
    assistant.systemPrompt.trim(),
    compilePersonality(assistant.personality),
    RESPONSE_LENGTH[assistant.responseLength],
  ]
    .filter(Boolean)
    .join('\n\n')
}

export const agentApi = {
  overview: () => invoke<AgentOverview>('agent_overview'),
  tools: (assistantId: string) =>
    invoke<{ tools: AgentToolInfo[] }>('agent_tools', { assistantId }).then((value) => value.tools),
  profile: (assistantId: string) =>
    invoke<{ profile: AgentProfile; revision: number }>('agent_profile_get', { assistantId }),
  saveProfile: (profile: AgentProfile, expectedRevision: number) =>
    invoke<number>('agent_profile_save', { profile, expectedRevision }),
  exportProfile: (assistantId: string) =>
    invoke<Record<string, unknown>>('agent_profile_export', { assistantId }),
  importProfile: (assistantId: string, document: unknown) =>
    invoke<number>('agent_profile_import', { assistantId, document }),

  startTask: (request: {
    assistantId: string
    goal: string
    projectId?: string | null
    title?: string
  }) => invoke<string>('task_start', { request }),
  task: (taskId: string) => invoke<TaskDetail>('task_detail', { taskId }),
  tasks: (limit?: number) => invoke<AgentTask[]>('task_list', { limit }),
  pause: (taskId: string) => invoke<void>('task_pause', { taskId }),
  resume: (taskId: string) => invoke<void>('task_resume', { taskId }),
  cancel: (taskId: string) => invoke<void>('task_cancel', { taskId }),
  steer: (taskId: string, text: string) => invoke<void>('task_steer', { taskId, text }),
  answer: (taskId: string, answer: string) => invoke<void>('task_answer', { taskId, answer }),
  decide: (approvalId: string, decision: 'allow-once' | 'allow-standing' | 'deny') =>
    invoke<void>('approval_decide', { approvalId, decision }),

  grants: () => invoke<Grant[]>('grants_list'),
  revokeGrant: (grantId: string) => invoke<void>('grant_revoke', { grantId }),
  stopAll: (stopped: boolean) => invoke<void>('agent_stop_all', { stopped }),
  setSettings: (changes: Record<string, unknown>) =>
    invoke<Record<string, unknown>>('agent_settings_set', { changes }),

  saveProject: (project: {
    id?: string | null
    name: string
    instructions: string
    archived: boolean
    expectedRevision: number
  }) => invoke<Project>('project_save', project),
  deleteProject: (projectId: string) => invoke<void>('project_delete', { projectId }),
  addRoot: (projectId: string, access: 'read' | 'read-write') =>
    invoke<Project | null>('project_add_root', { projectId, access }),
  removeRoot: (rootId: string) => invoke<void>('project_remove_root', { rootId }),
  indexProject: (projectId: string) =>
    invoke<{ indexed: number; unchanged: number; removed: number; skipped: number }>(
      'project_index',
      { projectId },
    ),
  exportProject: (projectId: string) =>
    invoke<Record<string, unknown>>('project_export', { projectId }),
  importProject: (document: unknown) => invoke<Project>('project_import', { document }),

  artifact: (artifactId: string) =>
    invoke<{ text: string; truncated: boolean }>('artifact_content', { artifactId }),
  exportArtifact: (artifactId: string) => invoke<boolean>('artifact_export', { artifactId }),
  markRead: (ids: number[]) => invoke<void>('inbox_mark_read', { ids }),
  memoryChanges: () => invoke<MemoryChange[]>('memory_changes'),
  memoryChangesApplied: (ids: string[]) => invoke<void>('memory_changes_applied', { ids }),

  skills: () => invoke<Skill[]>('skills_list'),
  setSkillEnabled: (skillId: string, enabled: boolean) =>
    invoke<void>('skill_set_enabled', { skillId, enabled }),
  removeSkill: (skillId: string) => invoke<void>('skill_remove', { skillId }),
  importSkill: () => invoke<Skill | null>('skill_import'),
  exportSkill: (skillId: string) => invoke<boolean>('skill_export', { skillId }),

  plugins: () => invoke<PluginSummary[]>('plugins_list'),
  registerHttpPlugin: (name: string, url: string, credentialRef?: string | null) =>
    invoke<PluginSummary>('plugin_register_http', { name, url, credentialRef }),
  registerLocalPlugin: (
    name: string,
    args: string[],
    env: Array<{ name: string; credentialRef: string }>,
  ) => invoke<PluginSummary | null>('plugin_register_local', { name, args, env }),
  testPlugin: (pluginId: string) => invoke<PluginSummary>('plugin_test', { pluginId }),
  setPluginEnabled: (pluginId: string, enabled: boolean) =>
    invoke<void>('plugin_set_enabled', { pluginId, enabled }),
  removePlugin: (pluginId: string) => invoke<void>('plugin_remove', { pluginId }),

  schedules: () => invoke<Schedule[]>('schedules_list'),
  previewSchedule: (spec: ScheduleSpec) => invoke<string[]>('schedule_preview', { spec }),
  saveSchedule: (
    id: string | null,
    spec: ScheduleSpec,
    enabled: boolean,
    expectedRevision: number,
  ) => invoke<Schedule>('schedule_save', { id, spec, enabled, expectedRevision }),
  deleteSchedule: (scheduleId: string) => invoke<void>('schedule_delete', { scheduleId }),
  runScheduleNow: (scheduleId: string) => invoke<string>('schedule_run_now', { scheduleId }),
}

/** Subscribes to "something changed" notices; the caller re-reads state. */
export async function onAgentEvent(handler: (event: AgentEvent) => void): Promise<() => void> {
  if (!runningInTauri) return () => undefined
  return listen<AgentEvent>('juniper://agent/event', (event) => handler(event.payload))
}

/** A host error (`CODE: message`) as the sentence a person reads. */
export function errorText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error)
  const match = text.match(/^[A-Z0-9_]+: (.*)$/s)
  return match ? match[1]! : text
}

export function deviceTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}

export interface MemoryChange {
  id: string
  assistantId: string
  operation: 'save' | 'delete'
  payload: { content?: string; id?: string; taskId?: string }
  createdAt: string
}

/** Applies host-queued memory changes; each id is applied at most once. */
export function applyMemoryChanges(data: AppData, changes: MemoryChange[]): AppData {
  let memories = data.memories
  for (const change of changes) {
    const id = `memory-agent-${change.id}`
    if (
      change.operation === 'save' &&
      change.payload.content &&
      !memories.some((memory) => memory.id === id)
    ) {
      memories = [
        ...memories,
        {
          id,
          assistantId: change.assistantId,
          content: change.payload.content,
          source: 'agent-task',
          enabled: true,
          createdAt: change.createdAt,
          updatedAt: change.createdAt,
        },
      ]
    } else if (change.operation === 'delete' && change.payload.id) {
      memories = memories.filter((memory) => memory.id !== change.payload.id)
    }
  }
  return memories === data.memories ? data : { ...data, memories }
}
