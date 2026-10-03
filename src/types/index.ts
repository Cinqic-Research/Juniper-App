export type Locality = 'local' | 'remote' | 'unknown'
export type ExecutionLocation = 'on-device' | 'local-network' | 'remote' | 'unknown'
export type SupportLevel = 'supported' | 'unsupported' | 'unknown'
export type PermissionDecision = 'allow-once' | 'allow-chat' | 'allow-assistant' | 'deny'
export type PermissionGrantScope = 'chat' | 'assistant'
/** Primary destinations. Everything else is nested under Settings. */
export type Page = 'chats' | 'models' | 'settings'

export type SettingsSection =
  | 'general'
  | 'appearance'
  | 'accessibility'
  | 'assistants'
  | 'models-runtime'
  | 'connections'
  | 'tools'
  | 'memory'
  | 'privacy'
  | 'advanced'
  | 'diagnostics'
  | 'about'

export type ProviderKind =
  | 'juniper-local'
  | 'juniper-network'
  | 'ollama'
  | 'openai-compatible'
  | 'llama-cpp'

export type RuntimeMaturity = 'stable' | 'beta' | 'experimental' | 'unavailable'
export type RuntimeState = 'available' | 'unavailable' | 'needs-attention' | 'not-qualified'
export type ArtifactQualification = 'qualified' | 'package-only' | 'not-qualified' | 'unknown'

export interface RuntimeDescriptor {
  id: string
  name: string
  maturity: RuntimeMaturity
  state: RuntimeState
  version: string
  sourceRevision: string
  platforms: string[]
  architectures: string[]
  artifactFormats: string[]
  capabilities: string[]
  accelerator: 'cpu' | 'gpu' | 'npu' | 'unknown'
  installed: boolean
  reason?: string
  qualification: ArtifactQualification
}

export interface RuntimeBinding {
  runtimeId: string
  artifactId?: string
  selection: 'explicit' | 'recommended' | 'auto'
}

export type DeviceLinkScope =
  | 'inference'
  | 'app-control'
  | 'model-runtime-inspection'
  | 'model-management'
  | 'data-sync'

export interface DeviceLinkPeer {
  id: string
  name: string
  fingerprint: string
  scopes: DeviceLinkScope[]
  connected: boolean
  lastSeenAt?: string
  address?: string
}

export interface DeviceLinkState {
  enabled: boolean
  hosting: boolean
  deviceId: string
  peers: DeviceLinkPeer[]
}

export interface DeviceLinkStatus {
  protocolVersion: number
  deviceId: string
  fingerprint: string
  lanOnly: boolean
  tlsRequired: boolean
  peers: DeviceLinkPeer[]
}

export interface DeviceLinkPairingOffer {
  protocolVersion: number
  sessionId: string
  hostDeviceId: string
  hostFingerprint: string
  token: string
  authString: string
  expiresAt: number
  qrPayload: string
}

export interface ProviderCapabilities {
  chat: SupportLevel
  text: SupportLevel
  streaming: SupportLevel
  systemPrompt: SupportLevel
  tools: SupportLevel
  parallelTools: SupportLevel
  thinking: SupportLevel
  structuredOutput: SupportLevel
  images: SupportLevel
  embeddings: SupportLevel
  contextLength?: number
  generationParameters: string[]
}

export interface ProviderProfile {
  id: string
  name: string
  kind: ProviderKind
  baseUrl: string
  locality: Locality
  transportLocation: ExecutionLocation
  apiKeyRef?: string
  enabled: boolean
  status: 'connected' | 'offline' | 'unknown'
  capabilities: ProviderCapabilities
  deviceId?: string
  deviceLinkFingerprint?: string
}

export interface ModelProfile {
  id: string
  providerId: string
  modelId: string
  displayName: string
  locality: Locality
  executionLocation: ExecutionLocation
  sourceReference?: string
  family?: string
  architecture?: string
  parameterSize?: string
  fileSizeBytes?: number
  quantization?: string
  format?: string
  license?: string
  template?: string
  compatibilityStatus: 'chat-compatible' | 'not-chat-compatible' | 'unknown'
  metadataSource?: string
  lastInspectedAt?: string
  rawCapabilities?: string[]
  sizeLabel?: string
  contextLength?: number
  status: 'ready' | 'not-found' | 'unknown'
  capabilities: ProviderCapabilities
  description: string
  catalogId?: string
  managedVariantId?: string
  artifactId?: string
  runtimeId?: string
  runtimeMaturity?: RuntimeMaturity
  runtimeBinding?: RuntimeBinding
}

export interface GenerationOverrides {
  temperature?: number
  topP?: number
  topK?: number
  minP?: number
  repetitionPenalty?: number
  maxOutput?: number
  thinking?: ThinkingMode
}

export type ThinkingMode = 'auto' | 'off' | 'on' | 'low' | 'medium' | 'high'

export interface PersonalityControls {
  warmth: number
  directness: number
  playfulness: number
  detail: number
  creativity: number
  formality: number
}

export interface Assistant {
  id: string
  schemaVersion: 2
  name: string
  description: string
  avatar: string
  accent: string
  modelProfileId: string | null
  systemPrompt: string
  personality: PersonalityControls
  responseLength: 'concise' | 'balanced' | 'detailed'
  generation: GenerationOverrides
  toolPolicy: 'safe-automatic' | 'ask' | 'disabled'
  memoryPolicy: 'off' | 'curated'
  welcomeMessage: string
  suggestedPrompts: string[]
  createdAt: string
  updatedAt: string
}

export type MessageRole = 'system' | 'user' | 'assistant' | 'tool'
export type MessagePartType = 'text' | 'attachment' | 'tool-call' | 'tool-result' | 'error'

export interface MessagePart {
  id: string
  type: MessagePartType
  text?: string
  name?: string
  status?: 'success' | 'error' | 'denied' | 'unavailable' | 'unsupported' | 'cancelled'
  metadata?: Record<string, string | number | boolean>
}

export interface ChatMessage {
  id: string
  conversationId: string
  role: MessageRole
  parts: MessagePart[]
  createdAt: string
  modelId?: string
  providerId?: string
  isStreaming?: boolean
  usage?: GenerationUsage
  /** What produced this answer, as reported by the native host. */
  provenance?: Provenance
}

/** Host-reported identity of the model, runtime, and behavior behind an answer. */
export interface Provenance {
  backend: string
  constitution: string
  toolProtocol: string
  providerKind: string
  modelId: string
  executionLocation: string
  reasoningEffort?: string | null
  runtime?: RuntimeIdentity | null
}

export interface RuntimeIdentity {
  artifactId: string
  artifactSha256: string
  modelRepository?: string | null
  modelRevision?: string | null
  runtimeBuild?: string | null
  templateSha256?: string | null
  contextSize?: number | null
}

export interface LocalRuntimeStatus {
  state: 'idle' | 'loading' | 'ready' | 'busy'
  artifactId?: string | null
  identity?: RuntimeIdentity | null
}

export interface Conversation {
  id: string
  title: string
  assistantId: string
  createdAt: string
  updatedAt: string
  privateChat?: boolean
  modelProfileId?: string | null
  messages: ChatMessage[]
}

export interface Memory {
  id: string
  assistantId?: string
  content: string
  source: string
  enabled: boolean
  createdAt: string
  updatedAt: string
}

export interface Attachment {
  id: string
  name: string
  sizeBytes: number
  contentType: string
}

export interface AttachmentRecord extends Attachment {
  conversationId: string
}

export interface PermissionGrant {
  id: string
  toolName: string
  scope: PermissionGrantScope
  assistantId: string
  conversationId?: string
  createdAt: string
  updatedAt: string
}

export interface PermissionRequest {
  requestId: string
  callId: string
  toolName: string
  displayName: string
  risk: ToolDefinition['risk']
  assistantId: string
  conversationId: string
  /** Host-authored description of exactly what approval allows. */
  preview?: string | null
  /** False for persistent writes, which are approved one call at a time. */
  standingGrantAllowed: boolean
}

export interface HostToolContext {
  memories: Memory[]
  conversations: Conversation[]
}

export interface GgufSelection {
  id: string
  name: string
  sizeBytes: number
}

export interface ToolDefinition {
  name: string
  displayName: string
  description: string
  risk:
    | 'automatic-safe'
    | 'user-data-read'
    | 'user-data-write'
    | 'filesystem-read'
    | 'network'
    | 'external-process'
    | 'sensitive'
  enabled: boolean
  schema: Record<string, unknown>
}

export type ThemePreference = 'system' | 'light' | 'dark'
export type ContrastPreference = 'system' | 'standard' | 'high'
export type AccentMode = 'color' | 'neutral'
export type FontChoice = 'juniper' | 'system' | 'legible' | 'dyslexic'
export type ChatTextSize = 'small' | 'medium' | 'large'
export type LineSpacing = 'compact' | 'standard' | 'relaxed'
export type Density = 'compact' | 'comfortable' | 'spacious'
export type ConversationWidth = 'narrow' | 'balanced' | 'wide'
export type SidebarPreference = 'expanded' | 'collapsed' | 'auto'
export type MessageStyle = 'bubbles' | 'minimal'
export type MotionPreference = 'system' | 'reduced' | 'full'

export interface AppSettings {
  theme: ThemePreference
  contrast: ContrastPreference
  accentMode: AccentMode
  /** Validated `#rrggbb`. Text and focus variants are derived for contrast. */
  accent: string
  fontFamily: FontChoice
  /** Interface zoom, 0.85–1.3 in 0.05 steps. */
  fontScale: number
  chatTextSize: ChatTextSize
  lineSpacing: LineSpacing
  density: Density
  conversationWidth: ConversationWidth
  sidebar: SidebarPreference
  messageStyle: MessageStyle
  showTimestamps: boolean
  showMessageDetails: boolean
  motion: MotionPreference
  /** Assistant used for new chats; falls back to the first assistant. */
  defaultAssistantId: string | null
  developerMode: boolean
  telemetry: 'off'
  onboardingComplete: boolean
}

export interface AppData {
  assistants: Assistant[]
  models: ModelProfile[]
  providers: ProviderProfile[]
  conversations: Conversation[]
  memories: Memory[]
  attachments: AttachmentRecord[]
  permissions: PermissionGrant[]
  settings: AppSettings
  deviceLink: DeviceLinkState
}

export interface ChatRequest {
  requestId: string
  assistantId: string
  assistantName: string
  conversationId: string
  privateChat: boolean
  provider: ProviderProfile
  model: ModelProfile
  messages: Array<{ role: MessageRole; content: string }>
  tools: ToolDefinition[]
  generation: GenerationOverrides
  permissionGrants: PermissionGrant[]
  hostContext: HostToolContext
  attachments?: Array<{
    id: string
    name: string
    content: string
    sizeBytes?: number
    contentType?: string
  }>
  /** Curated memories chosen for this request; the host resolves them. */
  contextMemoryIds: string[]
}

export interface DiscoveredModel {
  modelId: string
  displayName: string
  sizeBytes?: number
  modifiedAt?: string
}

export interface ModelInspection {
  modelId: string
  displayName: string
  family?: string
  architecture?: string
  parameterSize?: string
  fileSizeBytes?: number
  quantization?: string
  format?: string
  contextLength?: number
  license?: string
  template?: string
  capabilities: string[]
  metadataSource: string
  rawCapabilities?: string[]
}

export interface ModelPullProgress {
  requestId: string
  status: string
  digest?: string
  completedBytes?: number
  totalBytes?: number
  done?: boolean
  error?: { code: string; message: string }
}

export interface RuntimeLogEntry {
  timestamp: string
  event: string
  code?: string
  providerKind?: string
  modelId?: string
}

export interface ManagedModel {
  catalogId: string
  variantId?: string
  artifactId?: string
  fileName: string
  path: string
  sizeBytes: number
  sha256: string
  verified: boolean
  state: 'ready' | 'partial' | 'corrupt'
}

export interface NormalizedToolCall {
  id: string
  name: string
  arguments: Record<string, unknown>
}

export interface HostToolResult {
  protocolVersion: 'juniper-tool-protocol-v1'
  callId: string
  name: string
  status: 'success' | 'error' | 'denied' | 'unavailable' | 'unsupported' | 'cancelled'
  result: Record<string, unknown> | null
  error: { code: string; message: string } | null
}

export interface GenerationUsage {
  inputTokens?: number
  outputTokens?: number
  totalTokens?: number
  durationMs?: number
}

/**
 * Host progress while no answer text is streaming. Raw model reasoning is
 * never sent to the interface; only the fact that the model is reasoning.
 */
export type RuntimeActivity = 'loading-model' | 'warming-up' | 'restarting-model' | 'reasoning'

export interface ChatStreamEvent {
  requestId: string
  delta?: string
  activity?: RuntimeActivity | null
  toolCalls?: NormalizedToolCall[]
  toolResults?: HostToolResult[]
  permissionRequest?: PermissionRequest
  usage?: GenerationUsage
  done?: boolean
  error?: { code: string; message: string } | null
  provenance?: Provenance | null
}
