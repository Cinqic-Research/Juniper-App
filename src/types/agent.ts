// Contracts for the Agent (Beta) screen. The native host owns every record
// here; the interface reads them and sends intents. Shapes mirror
// src-tauri/src/agent/types.rs (camelCase over IPC).

export type TaskStatus =
  | 'queued'
  | 'running'
  | 'waiting-input'
  | 'waiting-approval'
  | 'paused'
  | 'interrupted'
  | 'completed'
  | 'failed'
  | 'cancelled'

export type AutonomyPreset = 'ask' | 'project' | 'custom'

export interface Budget {
  maxSteps: number
  maxToolCalls: number
  maxTokens: number
  maxSeconds: number
  maxArtifacts: number
  maxChildren: number
  maxDepth: number
  maxRetriesPerCause: number
}

export interface TaskUsage {
  steps: number
  toolCalls: number
  inputTokens: number
  outputTokens: number
  tokensEstimated: boolean
  activeMs: number
  artifacts: number
  children: number
  retries: Record<string, number>
  notBefore?: number | null
}

export interface PlanItem {
  text: string
  status: 'pending' | 'in-progress' | 'done'
}

export type Wait =
  | { kind: 'approval'; approvalId: string }
  | { kind: 'input'; question: string; answer?: string | null }
  | { kind: 'children'; taskIds: string[] }

export interface PendingCalls {
  calls: Array<{ id: string; name: string; arguments: unknown }>
  next: number
  wait?: Wait | null
}

export interface TaskResult {
  /** `needs-review`: the answer claims an effect no tool performed; not shown as completed. */
  outcome: 'answer' | 'deliverable' | 'partial' | 'needs-review'
  summary: string
  artifactIds: string[]
  sourceIds: string[]
  unverified: string[]
}

export interface TaskProvenance {
  assistantId: string
  agentProfileRevision: number
  modelProfileId: string
  modelId: string
  providerKind: string
  executionLocation: string
  catalogId?: string | null
  artifactId?: string | null
  skills: Array<{ id: string; version: string; digest: string }>
  projectId?: string | null
  autonomy: string
  tools: string[]
}

export interface AgentTask {
  id: string
  parentId?: string | null
  rootId: string
  depth: number
  role?: string | null
  assistantId: string
  projectId?: string | null
  scheduleRunId?: string | null
  title: string
  goal: string
  status: TaskStatus
  statusReason?: string | null
  currentAction?: string | null
  plan: PlanItem[]
  budget: Budget
  usage: TaskUsage
  provenance: TaskProvenance
  pending?: PendingCalls | null
  result?: TaskResult | null
  revision: number
  createdAt: string
  updatedAt: string
  finishedAt?: string | null
}

export interface TaskEvent {
  id: number
  taskId: string
  kind: string
  summary: string
  detail?: Record<string, unknown> | null
  createdAt: string
}

export interface TaskAction {
  id: string
  taskId: string
  step: number
  callId: string
  tool: string
  arguments: unknown
  status: string
  result?: Record<string, unknown> | null
  createdAt: string
}

export interface Approval {
  id: string
  taskId: string
  /** The top-level task the approval belongs to. */
  rootId: string
  actionId: string
  tool: string
  capability: string
  preview: string
  standingAllowed: boolean
  status: 'pending' | 'approved' | 'denied' | 'expired' | 'stale'
  decision?: string | null
  createdAt: string
  expiresAt: string
}

export interface Artifact {
  id: string
  taskId?: string | null
  projectId?: string | null
  kind: string
  title: string
  fileName: string
  mediaType: string
  sizeBytes: number
  sha256: string
  sourceIds: string[]
  status: string
  createdAt: string
}

export interface Source {
  id: string
  taskId?: string | null
  kind: 'web-result' | 'web-page' | 'project-file' | 'plugin' | string
  title: string
  url?: string | null
  finalUrl?: string | null
  location?: string | null
  retrievedAt: string
  excerpt: string
}

export interface TranscriptMessage {
  role: string
  content: string
}

export interface TaskDetail {
  task: AgentTask
  events: TaskEvent[]
  actions: TaskAction[]
  approvals: Approval[]
  artifacts: Artifact[]
  sources: Source[]
  children: AgentTask[]
  messages: TranscriptMessage[]
}

export interface AgentProfile {
  schemaVersion: 1
  assistantId: string
  preferredLanguage?: string | null
  autonomy: AutonomyPreset
  enabledTools: string[]
  enabledSkills: string[]
  projectIds: string[]
  modelProfileId?: string | null
  allowUnqualifiedModel: boolean
  budget: Budget
  composedProfile: string
}

export interface ProjectRoot {
  id: string
  projectId: string
  label: string
  path: string
  access: 'read' | 'read-write'
  createdAt: string
}

export interface Project {
  id: string
  name: string
  instructions: string
  archived: boolean
  roots: ProjectRoot[]
  revision: number
  createdAt: string
  updatedAt: string
}

export interface Grant {
  id: string
  assistantId: string
  projectId?: string | null
  capability: string
  scope: Record<string, unknown>
  createdAt: string
  expiresAt?: string | null
  revokedAt?: string | null
}

export interface InboxItem {
  id: number
  taskId?: string | null
  kind: string
  title: string
  body: string
  read: boolean
  createdAt: string
}

export interface Availability {
  available: boolean
  reason?: string
  note?: string
}

export interface AgentOverview {
  profiles: Array<{ profile: AgentProfile; revision: number }>
  projects: Project[]
  tasks: AgentTask[]
  pendingApprovals: Approval[]
  inbox: InboxItem[]
  stopped: boolean
  timeZone: string
  settings: Record<string, unknown>
  capabilities: Record<
    'workspace' | 'web' | 'sandbox' | 'mcpStdio' | 'mcpHttp' | 'scheduling',
    Availability
  >
  presets: Array<{ id: AutonomyPreset; description: string }>
}

export interface AgentToolInfo {
  name: string
  displayName: string
  description: string
  risk: string
  source: string
  enabled: boolean
  available: boolean
  reason?: string | null
}

export interface Skill {
  id: string
  name: string
  description: string
  version: string
  license?: string | null
  author?: string | null
  requires: string[]
  source: string
  digest: string
  enabled: boolean
  bundled: boolean
  body: string
  resources: string[]
}

export interface PluginSummary {
  id: string
  kind: 'mcp-stdio' | 'mcp-http'
  name: string
  location: string
  enabled: boolean
  revision: number
  command?: string | null
  args: string[]
  env: string[]
  url?: string | null
  hasToken: boolean
  access: string
  state: {
    status: 'not-connected' | 'connected' | 'error'
    message?: string | null
    era?: 'modern' | 'legacy' | null
    protocolVersion?: string | null
    serverInfo?: { name?: string; version?: string } | null
    rejectedTools: string[]
    checkedAt?: string | null
  }
  tools: Array<{ name: string; exposedName: string; description: string }>
}

export type ScheduleRule =
  | { kind: 'once'; at: string }
  | { kind: 'daily'; time: string }
  | { kind: 'weekly'; days: number[]; time: string }
  | { kind: 'monthly'; day: number; time: string }
  | { kind: 'interval'; minutes: number }

export interface ScheduleSpec {
  title: string
  action: 'agent-task' | 'reminder'
  prompt: string
  assistantId: string
  projectId?: string | null
  timezone: string
  rule: ScheduleRule
  missedPolicy: 'run-once' | 'skip'
  retryLimit: number
  anchor?: string | null
}

export interface Schedule {
  id: string
  spec: ScheduleSpec
  enabled: boolean
  nextRunAt?: string | null
  upcoming: string[]
  revision: number
  createdAt: string
  updatedAt: string
}

export interface AgentEvent {
  taskId?: string | null
  kind: string
}
