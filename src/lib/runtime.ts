import { addPluginListener, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  AppData,
  Attachment,
  ChatRequest,
  ChatStreamEvent,
  DiscoveredModel,
  GgufSelection,
  LocalRuntimeStatus,
  ModelInspection,
  ModelProfile,
  ModelPullProgress,
  PermissionDecision,
  ProviderProfile,
  RuntimeLogEntry,
  RuntimeDescriptor,
} from '../types'
import {
  MODEL_CATALOG,
  browserDeviceCapabilities,
  parseCatalog,
  type DeviceCapabilities,
  type ModelCatalog,
} from './model-catalog'
import { randomUuid } from './ids'
import { normalizeAppData } from './storage'

export const runningInTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
export const browserPreviewEnabled = import.meta.env.DEV

let pendingNativeSave: Promise<void> = Promise.resolve()

function abortError(): DOMException {
  return new DOMException('Generation cancelled', 'AbortError')
}

export async function streamChat(
  request: ChatRequest,
  onEvent: (event: ChatStreamEvent) => void,
  signal: AbortSignal,
): Promise<void> {
  if (signal.aborted) throw abortError()
  if (!runningInTauri) {
    if (!browserPreviewEnabled) {
      throw new Error('Model generation requires the Juniper native runtime.')
    }
    await fakeStream(request, onEvent, signal)
    return
  }
  const topic = `juniper://chat/${request.requestId}`
  const unlisten = await listen<ChatStreamEvent>(topic, (event) => onEvent(event.payload))
  const cancel = () => {
    void cancelChat(request.requestId)
  }
  signal.addEventListener('abort', cancel, { once: true })
  try {
    await invoke('chat_stream', { request })
    if (signal.aborted) throw abortError()
  } finally {
    signal.removeEventListener('abort', cancel)
    await unlisten()
  }
}

async function fakeStream(
  request: ChatRequest,
  onEvent: (event: ChatStreamEvent) => void,
  signal: AbortSignal,
): Promise<void> {
  const latest = request.messages[request.messages.length - 1]?.content.toLowerCase() ?? ''
  let answer =
    'This is a development preview. Open the Juniper desktop or Android app and choose a local model in Models to generate a real answer.'
  if (latest.includes('who are you'))
    answer = `I’m Juniper — the assistant experience you configured, currently using ${request.model.displayName} underneath. The browser preview is deterministic and clearly marked as development-only.`
  else if (latest.includes('847291') && latest.includes('19347'))
    answer = 'The host calculator result is **16,392,538,977**.'
  else if (latest.includes('current time') || latest.includes('what time'))
    answer = `The development preview is running at ${new Date().toLocaleString()}. For authoritative host time, connect the desktop runtime.`
  const words = answer.split(/(\s+)/)
  for (const word of words) {
    if (signal.aborted) throw abortError()
    await new Promise((resolve) => window.setTimeout(resolve, 18))
    onEvent({ requestId: request.requestId, delta: word })
  }
  onEvent({ requestId: request.requestId, done: true, usage: { outputTokens: words.length } })
}

export async function cancelChat(requestId: string): Promise<void> {
  if (runningInTauri) await invoke('cancel_chat', { requestId })
}

export async function resolvePermission(
  requestId: string,
  callId: string,
  decision: PermissionDecision,
): Promise<void> {
  if (!runningInTauri) return
  await invoke('resolve_permission', { requestId, callId, decision })
}

export async function checkProviderConnection(provider: ProviderProfile): Promise<string> {
  if (!runningInTauri) throw new Error('Connection checks require the Tauri desktop runtime.')
  return invoke<string>('health_check', {
    kind: provider.kind,
    baseUrl: provider.baseUrl,
    apiKeyRef: provider.apiKeyRef,
    deviceLinkFingerprint: provider.deviceLinkFingerprint,
  })
}

export async function listProviderModels(provider: ProviderProfile): Promise<DiscoveredModel[]> {
  if (!runningInTauri) throw new Error('Model discovery requires the Tauri desktop runtime.')
  return invoke<DiscoveredModel[]>('list_models', {
    kind: provider.kind,
    baseUrl: provider.baseUrl,
    apiKeyRef: provider.apiKeyRef,
    deviceLinkFingerprint: provider.deviceLinkFingerprint,
  })
}

export async function inspectProviderModel(
  provider: ProviderProfile,
  modelId: string,
): Promise<ModelInspection> {
  if (!runningInTauri) throw new Error('Model inspection requires the Tauri desktop runtime.')
  return invoke<ModelInspection>('inspect_model', {
    kind: provider.kind,
    baseUrl: provider.baseUrl,
    modelId,
    apiKeyRef: provider.apiKeyRef,
    deviceLinkFingerprint: provider.deviceLinkFingerprint,
  })
}

export async function pullProviderModel(
  provider: ProviderProfile,
  modelReference: string,
  onProgress: (progress: ModelPullProgress) => void,
  signal: AbortSignal,
): Promise<void> {
  if (!runningInTauri) throw new Error('Model downloads require the Tauri desktop runtime.')
  const requestId = `pull-${randomUuid()}`
  const topic = `juniper://model-pull/${requestId}`
  const unlisten = await listen<ModelPullProgress>(topic, (event) => onProgress(event.payload))
  const cancel = () => void cancelModelPull(requestId)
  signal.addEventListener('abort', cancel, { once: true })
  try {
    await invoke('pull_model', {
      kind: provider.kind,
      baseUrl: provider.baseUrl,
      modelReference,
      requestId,
      apiKeyRef: provider.apiKeyRef,
    })
    if (signal.aborted) throw abortError()
  } finally {
    signal.removeEventListener('abort', cancel)
    await unlisten()
  }
}

export async function cancelModelPull(requestId: string): Promise<void> {
  if (runningInTauri) await invoke('cancel_model_pull', { requestId })
}

export async function getModelCatalog(): Promise<ModelCatalog> {
  if (!runningInTauri) return MODEL_CATALOG
  const models = await invoke<unknown>('model_catalog')
  return parseCatalog({ version: 2, minimumAppVersion: '0.3.0-rc.31', models })
}

export async function getRuntimeRegistry(): Promise<RuntimeDescriptor[]> {
  if (!runningInTauri) return []
  return invoke<RuntimeDescriptor[]>('runtime_registry')
}

export async function getDeviceCapabilities(): Promise<DeviceCapabilities> {
  if (!runningInTauri) {
    return browserDeviceCapabilities()
  }
  return invoke<DeviceCapabilities>('device_capabilities')
}

export async function getManagedModels(): Promise<import('../types').ManagedModel[]> {
  if (!runningInTauri) return []
  return invoke<import('../types').ManagedModel[]>('managed_models')
}

export async function downloadManagedModel(
  catalogId: string,
  onProgress: (progress: ModelPullProgress) => void,
  signal: AbortSignal,
): Promise<void> {
  if (!runningInTauri) throw new Error('Model downloads require the Juniper native runtime.')
  const requestId = `managed-${randomUuid()}`
  const topic = `juniper://model-download/${requestId}`
  const unlisten = await listen<ModelPullProgress>(topic, (event) => onProgress(event.payload))
  const cancel = () => void cancelManagedModel(requestId)
  signal.addEventListener('abort', cancel, { once: true })
  try {
    await invoke('download_managed_model', { catalogId, requestId })
    if (signal.aborted) throw abortError()
  } finally {
    signal.removeEventListener('abort', cancel)
    await unlisten()
  }
}

export async function cancelManagedModel(requestId: string): Promise<void> {
  if (runningInTauri) await invoke('cancel_managed_model', { requestId })
}

/**
 * Installs a catalog model from a GGUF the user picked. The native host checks
 * the file against the catalog's size and SHA-256 before it becomes usable.
 */
export async function importManagedModel(
  selectionId: string,
  catalogId: string,
  onProgress: (progress: ModelPullProgress) => void,
  signal: AbortSignal,
): Promise<void> {
  if (!runningInTauri) throw new Error('Model import requires the Juniper native runtime.')
  const requestId = `managed-import-${randomUuid()}`
  const topic = `juniper://model-download/${requestId}`
  const unlisten = await listen<ModelPullProgress>(topic, (event) => onProgress(event.payload))
  const cancel = () => void cancelManagedModel(requestId)
  signal.addEventListener('abort', cancel, { once: true })
  try {
    await invoke('import_managed_model', { selectionId, catalogId, requestId })
    if (signal.aborted) throw abortError()
  } finally {
    signal.removeEventListener('abort', cancel)
    await unlisten()
  }
}

export async function getLocalRuntimeStatus(): Promise<LocalRuntimeStatus> {
  if (!runningInTauri) return { state: 'idle' }
  return invoke<LocalRuntimeStatus>('local_runtime_status')
}

/** Frees the resident local model's memory. Refused while a reply is generating. */
export async function unloadLocalRuntime(): Promise<boolean> {
  if (!runningInTauri) return false
  return invoke<boolean>('unload_local_runtime')
}

export async function deleteManagedModel(catalogId: string): Promise<void> {
  if (!runningInTauri) throw new Error('Model management requires the Juniper native runtime.')
  await invoke('delete_managed_model', { catalogId })
}

export async function deleteProviderModel(
  provider: ProviderProfile,
  modelId: string,
): Promise<void> {
  if (!runningInTauri) throw new Error('Model deletion requires the Tauri desktop runtime.')
  await invoke('delete_model', {
    kind: provider.kind,
    baseUrl: provider.baseUrl,
    modelId,
    apiKeyRef: provider.apiKeyRef,
  })
}

export async function runningProviderModels(
  provider: ProviderProfile,
): Promise<Record<string, unknown>[]> {
  if (!runningInTauri) throw new Error('Runtime inspection requires the Tauri desktop runtime.')
  return invoke<Record<string, unknown>[]>('running_models', {
    kind: provider.kind,
    baseUrl: provider.baseUrl,
    apiKeyRef: provider.apiKeyRef,
  })
}

export async function pickAttachment(): Promise<Attachment | null> {
  if (!runningInTauri) return null
  return invoke<Attachment | null>('pick_attachment')
}

export async function readAttachment(attachmentId: string): Promise<string> {
  if (!runningInTauri) throw new Error('Attachment reads require the Tauri desktop runtime.')
  return invoke<string>('read_attachment', { attachmentId })
}

export async function pickGguf(): Promise<GgufSelection | null> {
  if (!runningInTauri) throw new Error('GGUF selection requires the Tauri desktop runtime.')
  return invoke<GgufSelection | null>('pick_gguf')
}

export async function importGguf(
  selectionId: string,
  modelName: string,
  onProgress: (progress: ModelPullProgress) => void,
  signal: AbortSignal,
): Promise<void> {
  if (!runningInTauri) throw new Error('GGUF import requires the Tauri desktop runtime.')
  const requestId = `gguf-${randomUuid()}`
  const topic = `juniper://gguf-import/${requestId}`
  const unlisten = await listen<ModelPullProgress>(topic, (event) => onProgress(event.payload))
  const cancel = () => void cancelGgufImport(requestId)
  signal.addEventListener('abort', cancel, { once: true })
  try {
    await invoke('import_gguf', { selectionId, modelName, requestId })
    if (signal.aborted) throw abortError()
  } finally {
    signal.removeEventListener('abort', cancel)
    await unlisten()
  }
}

export async function cancelGgufImport(requestId: string): Promise<void> {
  if (runningInTauri) await invoke('cancel_gguf_import', { requestId })
}

export async function loadNativeAppData(): Promise<AppData | null> {
  if (!runningInTauri) return null
  const value = await invoke<unknown>('load_app_data')
  return value ? normalizeAppData(value) : null
}

/** Tells the native host the interface mounted and hydrated stored state. */
export async function reportFrontendReady(): Promise<void> {
  if (!runningInTauri) return
  await invoke('frontend_ready')
}

/** Sends a content-free error class to local stderr; messages may contain user data. */
export async function reportFrontendFatal(error: unknown): Promise<void> {
  if (!runningInTauri) return
  const name = error instanceof Error ? error.name : 'UnknownError'
  const safeName = /^[A-Za-z][A-Za-z0-9]{0,63}$/.test(name) ? name : 'UnknownError'
  await invoke('frontend_fatal', { report: `interface ${safeName}` })
}

export async function saveNativeAppData(data: AppData): Promise<void> {
  if (!runningInTauri) return

  // App state is persisted as a full snapshot. Serialize writes so a slower
  // snapshot cannot finish after a newer one and roll the database backward.
  const write = pendingNativeSave
    .catch(() => undefined)
    .then(() => invoke<void>('save_app_data', { data }))
  pendingNativeSave = write.catch(() => undefined)
  await write
}

export async function saveProviderCredential(reference: string, secret: string): Promise<void> {
  if (!runningInTauri) throw new Error('Secure credentials require the native Juniper runtime.')
  await invoke('secure_set_credential', { reference, secret })
}

export async function deleteProviderCredential(reference: string): Promise<void> {
  if (!runningInTauri) return
  await invoke('secure_delete_credential', { reference })
}

export async function getDiagnostics(): Promise<Record<string, string>> {
  if (runningInTauri) return invoke<Record<string, string>>('system_info')
  return {
    application: 'Juniper 0.3.0-rc.33',
    runtime: browserPreviewEnabled
      ? 'Browser preview (development only)'
      : 'Native runtime unavailable',
    platform: navigator.platform,
    provider: 'Not connected',
    telemetry: 'Off',
  }
}

export async function getRuntimeLogs(): Promise<RuntimeLogEntry[]> {
  if (runningInTauri) return invoke<RuntimeLogEntry[]>('runtime_logs')
  return []
}

export function modelFromInspection(
  provider: ProviderProfile,
  inspection: ModelInspection,
  existing?: ModelProfile,
): ModelProfile {
  const executionLocation = provider.transportLocation
  const hasChat =
    inspection.capabilities.includes('completion') || inspection.capabilities.includes('chat')
  return {
    ...(existing ?? {}),
    id: existing?.id ?? `${provider.id}:${inspection.modelId}`,
    providerId: provider.id,
    modelId: inspection.modelId,
    displayName: inspection.displayName,
    locality:
      executionLocation === 'remote'
        ? 'remote'
        : executionLocation === 'unknown'
          ? 'unknown'
          : 'local',
    executionLocation,
    sourceReference: existing?.sourceReference ?? inspection.modelId,
    sizeLabel: inspection.fileSizeBytes
      ? `${Math.round(inspection.fileSizeBytes / 1_000_000)} MB`
      : existing?.sizeLabel,
    contextLength: inspection.contextLength,
    status: 'ready',
    compatibilityStatus: hasChat
      ? 'chat-compatible'
      : inspection.capabilities.length
        ? 'not-chat-compatible'
        : 'unknown',
    capabilities: {
      ...provider.capabilities,
      chat: hasChat ? 'supported' : 'unknown',
      text: hasChat ? 'supported' : 'unknown',
      streaming: 'supported',
      systemPrompt: 'supported',
      tools: inspection.capabilities.includes('tools') ? 'supported' : 'unknown',
      thinking: inspection.capabilities.includes('thinking') ? 'supported' : 'unknown',
      images: inspection.capabilities.includes('vision') ? 'supported' : 'unknown',
    },
    description: `Discovered from ${provider.name}. Runtime metadata is the source of truth for compatibility.`,
    family: inspection.family,
    architecture: inspection.architecture,
    parameterSize: inspection.parameterSize,
    fileSizeBytes: inspection.fileSizeBytes,
    quantization: inspection.quantization,
    format: inspection.format,
    license: inspection.license,
    template: inspection.template,
    metadataSource: inspection.metadataSource,
    lastInspectedAt: new Date().toISOString(),
    rawCapabilities: inspection.rawCapabilities ?? inspection.capabilities,
  }
}

/** System bar and keyboard insets in CSS pixels. Only Android reports them. */
export interface WindowInsets {
  top: number
  right: number
  bottom: number
  left: number
  keyboard: number
}

export async function getWindowInsets(): Promise<WindowInsets | null> {
  if (!runningInTauri) return null
  return invoke<WindowInsets | null>('window_insets')
}

export const runningOnAndroid =
  runningInTauri && typeof navigator !== 'undefined' && /android/i.test(navigator.userAgent)

/**
 * Receives Android back presses instead of Tauri's default handling. While a
 * listener is registered Android does not leave the app on back, so callers
 * register only while there is somewhere in Juniper to go back to.
 */
export async function listenForBackButton(handler: () => void): Promise<() => Promise<void>> {
  const listener = await addPluginListener('app', 'back-button', handler)
  return () => listener.unregister()
}
