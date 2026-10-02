import catalogJson from '../../config/models/catalog.json'
import type { ArtifactQualification, RuntimeDescriptor, SupportLevel } from '../types'

export interface CatalogArtifactFile {
  path: string
  sizeBytes: number
  sha256: string
  url?: string
}

/** A concrete runtime representation. A bundle is represented by `files`, not a fake file. */
export interface CatalogArtifact {
  id: string
  runtimeId: string
  format: string
  platforms: string[]
  architectures: string[]
  quantization?: string
  sizeBytes: number
  sha256?: string
  sourceUrl?: string
  sourceRevision: string
  files: CatalogArtifactFile[]
  minimumRuntimeVersion?: string
  maturity: 'stable' | 'beta' | 'experimental'
  qualification: ArtifactQualification
  /** A qualified native backend profile this artifact runs with. */
  backendProfile?: string
  /** Capabilities established by qualification; absent means unknown. */
  capabilities?: {
    tools: SupportLevel
    thinking: SupportLevel
    generationParameters: string[]
  }
  /** @deprecated Compatibility fields for older UI consumers. */
  fileName?: string
  /** @deprecated Compatibility fields for older UI consumers. */
  url?: string
}

/** @deprecated Kept as an in-memory compatibility alias for pre-v2 callers. */
export type ModelVariant = CatalogArtifact

export interface CatalogModel {
  id: string
  displayName: string
  organization: string
  family: string
  parameterCount: number
  description: string
  useCases: string[]
  instructionTuned: boolean
  architecture: string
  sourceRepository: string
  sourceRevision: string
  originalModel?: string
  license: string
  licenseUrl: string
  attribution: string
  chatTemplate: string
  contextLength: number
  minimumRecommendedRamBytes: number
  recommendedRamBytes: number
  minimumStorageBytes: number
  supportedArchitectures: string[]
  tags: string[]
  releaseStatus: 'available' | 'deprecated'
  artifacts: CatalogArtifact[]
  /** Compatibility view; new code must use artifacts. */
  variants: CatalogArtifact[]
}

export interface ModelCatalog {
  version: 2
  minimumAppVersion: string
  models: CatalogModel[]
}

export interface DeviceCapabilities {
  os: string
  deviceName: string
  architecture: string
  cpuArchitecture: string
  logicalCores: number
  totalMemoryBytes?: number
  availableMemoryBytes?: number
  memoryPressure: 'low' | 'medium' | 'high' | 'unknown'
  totalStorageBytes?: number
  freeStorageBytes?: number
  modelDirectory: string
  gpu: 'available' | 'not-detected' | 'unknown'
  acceleration: 'available' | 'not-detected' | 'unknown'
  nativeRuntimeAvailable?: boolean
  nativeRuntimeState?: 'unavailable' | 'loading' | 'ready' | 'busy' | 'failed'
  nativeAbi?: string
  nativeLowMemory?: boolean
  runtimes?: RuntimeDescriptor[]
}

export type ModelFit = 'excellent' | 'good' | 'possible' | 'not-recommended' | 'unknown'

export interface ModelRecommendation {
  model: CatalogModel
  artifact: CatalogArtifact
  /** Compatibility alias for the old variant-based market implementation. */
  variant: CatalogArtifact
  score: number
  fit: ModelFit
  reasons: string[]
  storageSafe: boolean
}

export const MODEL_CATALOG: ModelCatalog = parseCatalog(catalogJson)

const STORAGE_HEADROOM_BYTES = 512 * 1024 * 1024

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

function isSha256(value: unknown): value is string {
  return typeof value === 'string' && /^[a-f0-9]{64}$/i.test(value)
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string' && item.length > 0)
}

function legacyArtifact(raw: unknown, modelId: string): CatalogArtifact {
  if (
    !isRecord(raw) ||
    typeof raw.id !== 'string' ||
    typeof raw.fileName !== 'string' ||
    raw.fileName.length === 0 ||
    raw.fileName.includes('/') ||
    raw.fileName.includes('\\') ||
    typeof raw.quantization !== 'string' ||
    typeof raw.sizeBytes !== 'number' ||
    !Number.isSafeInteger(raw.sizeBytes) ||
    raw.sizeBytes <= 0 ||
    !isSha256(raw.sha256) ||
    typeof raw.url !== 'string' ||
    !raw.url.startsWith('https://') ||
    typeof raw.sourceRevision !== 'string'
  ) {
    throw new Error(`Catalog entry ${modelId} contains an invalid legacy artifact.`)
  }
  return {
    id: raw.id,
    runtimeId: 'llama.cpp',
    format: 'GGUF',
    // Legacy v1 records did not carry platform or architecture evidence.
    // Preserve integrity metadata without inventing compatibility.
    platforms: ['unknown'],
    architectures: ['unknown'],
    quantization: raw.quantization,
    sizeBytes: raw.sizeBytes,
    sha256: raw.sha256,
    sourceUrl: raw.url,
    sourceRevision: raw.sourceRevision,
    files: [
      {
        path: raw.fileName,
        sizeBytes: raw.sizeBytes,
        sha256: raw.sha256,
        url: raw.url,
      },
    ],
    maturity: 'experimental',
    qualification: 'unknown',
    fileName: raw.fileName,
    url: raw.url,
  }
}

function parseArtifact(raw: unknown, modelId: string): CatalogArtifact {
  if (!isRecord(raw))
    throw new Error(`Catalog entry ${modelId} contains an invalid variant/artifact.`)
  const files = raw.files
  if (
    typeof raw.id !== 'string' ||
    typeof raw.runtimeId !== 'string' ||
    typeof raw.format !== 'string' ||
    !isStringArray(raw.platforms) ||
    !isStringArray(raw.architectures) ||
    typeof raw.sizeBytes !== 'number' ||
    !Number.isSafeInteger(raw.sizeBytes) ||
    raw.sizeBytes <= 0 ||
    typeof raw.sourceRevision !== 'string' ||
    !['stable', 'beta', 'experimental'].includes(String(raw.maturity)) ||
    !['qualified', 'package-only', 'not-qualified', 'unknown'].includes(
      String(raw.qualification),
    ) ||
    !Array.isArray(files) ||
    files.length === 0
  ) {
    throw new Error(`Catalog entry ${modelId} contains an invalid variant/artifact.`)
  }
  const parsedFiles = files.map((file) => {
    if (
      !isRecord(file) ||
      typeof file.path !== 'string' ||
      file.path.length === 0 ||
      file.path.includes('/') ||
      file.path.includes('\\') ||
      typeof file.sizeBytes !== 'number' ||
      !Number.isSafeInteger(file.sizeBytes) ||
      file.sizeBytes <= 0 ||
      !isSha256(file.sha256) ||
      (file.url !== undefined && (typeof file.url !== 'string' || !file.url.startsWith('https://')))
    ) {
      throw new Error(`Catalog entry ${modelId} contains an invalid variant/artifact file.`)
    }
    return file as unknown as CatalogArtifactFile
  })
  if (
    raw.sourceUrl !== undefined &&
    (typeof raw.sourceUrl !== 'string' || !raw.sourceUrl.startsWith('https://'))
  ) {
    throw new Error(`Catalog entry ${modelId} contains an invalid variant/artifact source.`)
  }
  if (raw.sha256 !== undefined && !isSha256(raw.sha256)) {
    throw new Error(`Catalog entry ${modelId} contains an invalid variant/artifact hash.`)
  }
  const levels = ['supported', 'unsupported', 'unknown']
  if (
    raw.capabilities !== undefined &&
    (!isRecord(raw.capabilities) ||
      !levels.includes(String(raw.capabilities.tools)) ||
      !levels.includes(String(raw.capabilities.thinking)) ||
      !isStringArray(raw.capabilities.generationParameters))
  ) {
    throw new Error(`Catalog entry ${modelId} declares invalid capabilities.`)
  }
  if (parsedFiles.reduce((sum, file) => sum + file.sizeBytes, 0) !== raw.sizeBytes) {
    throw new Error(
      `Catalog entry ${modelId} variant/artifact size does not match its file manifest.`,
    )
  }
  return {
    ...raw,
    files: parsedFiles,
    fileName: parsedFiles[0]?.path,
    url: parsedFiles[0]?.url ?? (typeof raw.sourceUrl === 'string' ? raw.sourceUrl : undefined),
  } as unknown as CatalogArtifact
}

/** Validate catalog data before it is allowed into the UI or downloader. */
export function parseCatalog(value: unknown): ModelCatalog {
  if (
    !isRecord(value) ||
    ![1, 2].includes(Number(value.version)) ||
    typeof value.minimumAppVersion !== 'string' ||
    !value.minimumAppVersion ||
    !Array.isArray(value.models)
  ) {
    throw new Error('The model catalog is malformed.')
  }
  const ids = new Set<string>()
  const artifactIds = new Set<string>()
  const models = value.models.map((raw): CatalogModel => {
    if (!isRecord(raw) || typeof raw.id !== 'string' || ids.has(raw.id)) {
      throw new Error('The model catalog contains a duplicate or invalid model id.')
    }
    ids.add(raw.id)
    if (
      typeof raw.displayName !== 'string' ||
      typeof raw.organization !== 'string' ||
      typeof raw.family !== 'string' ||
      typeof raw.description !== 'string' ||
      typeof raw.architecture !== 'string' ||
      typeof raw.sourceRepository !== 'string' ||
      typeof raw.license !== 'string' ||
      typeof raw.chatTemplate !== 'string' ||
      !isStringArray(raw.useCases) ||
      !isStringArray(raw.supportedArchitectures) ||
      !isStringArray(raw.tags) ||
      !['available', 'deprecated'].includes(String(raw.releaseStatus)) ||
      typeof raw.parameterCount !== 'number' ||
      typeof raw.minimumRecommendedRamBytes !== 'number' ||
      typeof raw.recommendedRamBytes !== 'number' ||
      typeof raw.minimumStorageBytes !== 'number'
    ) {
      throw new Error(`Catalog entry ${raw.id} is missing required metadata.`)
    }
    const rawArtifacts =
      Number(value.version) === 1
        ? raw.variants
        : Array.isArray(raw.variants)
          ? raw.variants
          : raw.artifacts
    if (!Array.isArray(rawArtifacts) || rawArtifacts.length === 0) {
      throw new Error(`Catalog entry ${raw.id} must provide at least one artifact.`)
    }
    const modelId = raw.id
    const artifacts = rawArtifacts.map((item) =>
      Number(value.version) === 1 ? legacyArtifact(item, modelId) : parseArtifact(item, modelId),
    )
    for (const artifact of artifacts) {
      if (artifactIds.has(artifact.id)) {
        throw new Error(`The model catalog contains a duplicate artifact id: ${artifact.id}.`)
      }
      artifactIds.add(artifact.id)
    }
    return {
      ...raw,
      artifacts,
      variants: artifacts,
    } as unknown as CatalogModel
  })
  return { version: 2, minimumAppVersion: value.minimumAppVersion, models }
}

/** True when Juniper has no download source and the file must be imported. */
export function isImportOnly(artifact: CatalogArtifact): boolean {
  return !artifact.sourceUrl && artifact.files.every((file) => !file.url)
}

export function formatBytes(bytes: number | undefined): string {
  if (!bytes || !Number.isFinite(bytes) || bytes < 0) return 'Unknown size'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let index = 0
  while (value >= 1000 && index < units.length - 1) {
    value /= 1000
    index += 1
  }
  return `${value >= 10 || index === 0 ? Math.round(value) : value.toFixed(1)} ${units[index]}`
}

function memoryBudget(device: DeviceCapabilities): number | undefined {
  const available = device.availableMemoryBytes ?? device.totalMemoryBytes
  if (!available) return undefined
  const reserve = Math.max(512 * 1024 * 1024, available * 0.2)
  return Math.max(0, available - reserve)
}

function selectedArtifact(model: CatalogModel): CatalogArtifact {
  return [...model.artifacts].sort((left, right) => left.sizeBytes - right.sizeBytes)[0]!
}

function normalizedArchitecture(architecture: string): string {
  switch (architecture) {
    case 'arm64-v8a':
      return 'arm64'
    case 'armeabi-v7a':
      return 'armv7'
    default:
      return architecture
  }
}

function artifactSupportsDevice(artifact: CatalogArtifact, device: DeviceCapabilities): boolean {
  const architecture = normalizedArchitecture(device.architecture)
  const platform = device.os.toLowerCase()
  return (
    (artifact.platforms.includes(platform) ||
      (artifact.platforms.includes('desktop') && ['linux', 'windows'].includes(platform))) &&
    (artifact.architectures.includes(device.architecture) ||
      artifact.architectures.includes(architecture) ||
      device.architecture === 'unknown')
  )
}

export function recommendModel(
  model: CatalogModel,
  device: DeviceCapabilities,
): ModelRecommendation {
  const artifact =
    model.artifacts.find((item) => artifactSupportsDevice(item, device)) ?? selectedArtifact(model)
  const budget = memoryBudget(device)
  const reasons: string[] = []
  let score = 45
  let storageSafe = true
  if (!artifactSupportsDevice(artifact, device) && device.architecture !== 'unknown') {
    return {
      model,
      artifact,
      variant: artifact,
      score: 0,
      fit: 'not-recommended',
      reasons: [`${device.architecture} is not listed for the ${artifact.runtimeId} artifact`],
      storageSafe: false,
    }
  }
  if (budget === undefined) {
    reasons.push('Available memory could not be measured')
    score -= 15
  } else if (budget >= model.recommendedRamBytes) {
    score += 35
    reasons.push(
      `${formatBytes(device.availableMemoryBytes ?? device.totalMemoryBytes)} available memory leaves headroom`,
    )
  } else if (budget >= model.minimumRecommendedRamBytes) {
    score += 18
    reasons.push('Fits the measured memory budget with moderate headroom')
  } else {
    score -= 35
    reasons.push('Available memory is below the comfortable range')
  }
  const requiredStorage = Math.max(
    model.minimumStorageBytes,
    artifact.sizeBytes + STORAGE_HEADROOM_BYTES,
  )
  if (device.freeStorageBytes === undefined) {
    reasons.push('Free storage could not be measured')
    score -= 10
  } else if (device.freeStorageBytes >= requiredStorage) {
    score += 15
    reasons.push(`${formatBytes(device.freeStorageBytes)} free storage is enough for the download`)
  } else {
    score -= 40
    storageSafe = false
    reasons.push(
      `Needs about ${formatBytes(requiredStorage)} free storage including safety headroom`,
    )
  }
  if (device.memoryPressure === 'high') {
    score -= 18
    reasons.push('The device currently reports high memory pressure')
  } else if (device.memoryPressure === 'low') {
    score += 5
    reasons.push('The device currently reports low memory pressure')
  }
  if (model.tags.includes('small')) score += 4
  const fit: ModelFit = !storageSafe
    ? 'not-recommended'
    : score >= 82
      ? 'excellent'
      : score >= 62
        ? 'good'
        : score >= 35
          ? 'possible'
          : 'not-recommended'
  return {
    model,
    artifact,
    variant: artifact,
    score: Math.max(0, Math.min(100, score)),
    fit,
    reasons,
    storageSafe,
  }
}

export function recommendModels(
  catalog: ModelCatalog,
  device: DeviceCapabilities,
): ModelRecommendation[] {
  return catalog.models
    .filter((model) => model.releaseStatus === 'available')
    .map((model) => recommendModel(model, device))
    .sort(
      (left, right) =>
        right.score - left.score || left.artifact.sizeBytes - right.artifact.sizeBytes,
    )
}

export function runtimeOptionsForModel(
  model: CatalogModel,
  runtimes: RuntimeDescriptor[] = [],
): Array<{
  runtime: RuntimeDescriptor
  artifact: CatalogArtifact | undefined
  selectable: boolean
}> {
  return runtimes.map((runtime) => {
    const artifact = model.artifacts.find((candidate) => candidate.runtimeId === runtime.id)
    return {
      runtime,
      artifact,
      selectable: Boolean(artifact && runtime.installed && runtime.state === 'available'),
    }
  })
}

export function browserDeviceCapabilities(): DeviceCapabilities {
  const deviceMemory = 'deviceMemory' in navigator ? Number(navigator.deviceMemory) : undefined
  return {
    os: navigator.platform || 'browser',
    deviceName: 'This device',
    architecture: 'unknown',
    cpuArchitecture: 'unknown',
    logicalCores: navigator.hardwareConcurrency || 1,
    totalMemoryBytes: deviceMemory ? deviceMemory * 1024 ** 3 : undefined,
    availableMemoryBytes: undefined,
    memoryPressure: 'unknown',
    modelDirectory: 'Managed by Juniper',
    gpu: 'unknown',
    acceleration: 'unknown',
    runtimes: [],
  }
}
