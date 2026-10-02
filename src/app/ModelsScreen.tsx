import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type {
  AppData,
  Assistant,
  LocalRuntimeStatus,
  ManagedModel,
  ModelProfile,
  SettingsSection,
} from '../types'
import { defaultProvider, modelProfileFromDiscovery } from '../lib/defaults'
import {
  deleteManagedModel,
  downloadManagedModel,
  getDeviceCapabilities,
  getLocalRuntimeStatus,
  getManagedModels,
  getModelCatalog,
  importManagedModel,
  pickGguf,
  runningInTauri,
  runningOnAndroid,
  unloadLocalRuntime,
} from '../lib/runtime'
import {
  MODEL_CATALOG,
  formatBytes,
  isImportOnly,
  recommendModels,
  runtimeOptionsForModel,
  type CatalogModel,
  type DeviceCapabilities,
  type ModelCatalog,
  type ModelRecommendation,
} from '../lib/model-catalog'
import { isExternalModel, useExternalModels } from './external-models'
import { Icon } from './icons'
import {
  defaultAssistantFor,
  isChatSelectable,
  labelCapability,
  modelFitLabel,
  modelStatusLabel,
} from './model-labels'
import { useDialogs } from './overlays'
import { EmptyState, LocationBadge, PageHeader, Segmented } from './ui'

type Update = (change: (current: AppData) => AppData) => void
type View = 'recommended' | 'all' | 'installed'

function memoryLabel(device: DeviceCapabilities | null): string {
  if (!device?.availableMemoryBytes && !device?.totalMemoryBytes) return 'Unknown'
  const available = device.availableMemoryBytes ?? device.totalMemoryBytes
  return `${formatBytes(available)} available`
}

export function nativeRuntimeLabel(device: DeviceCapabilities | null): string {
  if (!device) return 'Checking…'
  if (device.nativeRuntimeAvailable === false) return 'Unavailable for this ABI'
  if (device.nativeLowMemory) return 'Waiting for more memory'
  switch (device.nativeRuntimeState) {
    case 'ready':
      return 'Ready'
    case 'loading':
    case 'busy':
      return 'Loading or generating'
    case 'failed':
      return 'Needs attention'
    default:
      return 'Loads on first chat'
  }
}

function parameterLabel(count: number): string {
  return count >= 1_000_000_000
    ? `${(count / 1_000_000_000).toFixed(1)}B`
    : `${Math.round(count / 1_000_000)}M`
}

function fitLabel(value: ModelRecommendation['fit']): string {
  switch (value) {
    case 'excellent':
      return 'Great fit'
    case 'good':
      return 'Good fit'
    case 'possible':
      return 'May be slow'
    case 'not-recommended':
      return 'Not recommended'
    default:
      return 'Fit unknown'
  }
}

function setAssistantModel(current: AppData, assistant: Assistant, profileId: string): AppData {
  return {
    ...current,
    assistants: current.assistants.map((item) =>
      item.id === assistant.id
        ? { ...item, modelProfileId: profileId, updatedAt: new Date().toISOString() }
        : item,
    ),
  }
}

export function ModelsScreen({
  data,
  update,
  openSettings,
}: {
  data: AppData
  update: Update
  openSettings: (section: SettingsSection) => void
}) {
  const dialogs = useDialogs()
  const [catalog, setCatalog] = useState<ModelCatalog>(MODEL_CATALOG)
  const [device, setDevice] = useState<DeviceCapabilities | null>(null)
  const [installed, setInstalled] = useState<ManagedModel[]>([])
  const [view, setView] = useState<View>('recommended')
  const [query, setQuery] = useState('')
  const [activeDownload, setActiveDownload] = useState<string | null>(null)
  const [progress, setProgress] = useState<{ completed: number; total: number } | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const [runtimeStatus, setRuntimeStatus] = useState<LocalRuntimeStatus>({ state: 'idle' })
  const controller = useRef<AbortController | null>(null)
  const external = useExternalModels(data, update)
  const defaultAssistant = defaultAssistantFor(data)

  const refresh = useCallback(async () => {
    try {
      const [nextCatalog, nextDevice, nextInstalled] = await Promise.all([
        getModelCatalog(),
        getDeviceCapabilities(),
        getManagedModels(),
      ])
      setCatalog(nextCatalog)
      setDevice(nextDevice)
      setInstalled(nextInstalled)
      if (!runningOnAndroid) setRuntimeStatus(await getLocalRuntimeStatus())
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Could not inspect this device.')
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const recommendations = useMemo(
    () => (device ? recommendModels(catalog, device) : []),
    [catalog, device],
  )
  const managedIds = useMemo(() => new Set(installed.map((model) => model.catalogId)), [installed])
  const installedIds = useMemo(
    () =>
      new Set(
        installed
          .filter((model) => model.verified && model.state === 'ready')
          .map((model) => model.catalogId),
      ),
    [installed],
  )
  const visible = useMemo(() => {
    const normalized = query.trim().toLowerCase()
    return recommendations.filter((item) => {
      const matches =
        !normalized ||
        [
          item.model.displayName,
          item.model.family,
          item.model.description,
          ...item.model.tags,
        ].some((value) => value.toLowerCase().includes(normalized))
      if (!matches) return false
      if (view === 'installed') return managedIds.has(item.model.id)
      if (view === 'recommended') return item.fit !== 'not-recommended' || item.storageSafe
      return true
    })
  }, [managedIds, query, recommendations, view])

  const externalModels = data.models.filter((model) => isExternalModel(data, model))

  function profileFor(entry: CatalogModel): ModelProfile {
    const provider = data.providers.find((item) => item.kind === 'juniper-local') ?? defaultProvider
    const qualified = entry.artifacts[0]?.capabilities
    return modelProfileFromDiscovery(provider, entry.id, {
      catalogId: entry.id,
      managedVariantId: entry.artifacts[0]?.id,
      artifactId: entry.artifacts[0]?.id,
      runtimeId: entry.artifacts[0]?.runtimeId,
      displayName: entry.displayName,
      description: entry.description,
      sourceReference: entry.sourceRepository,
      family: entry.family,
      architecture: entry.architecture,
      parameterSize: parameterLabel(entry.parameterCount),
      fileSizeBytes: entry.artifacts[0]?.sizeBytes,
      quantization: entry.artifacts[0]?.quantization,
      format: entry.artifacts[0]?.format,
      license: entry.license,
      template: entry.chatTemplate,
      contextLength: entry.contextLength,
      status: 'ready',
      compatibilityStatus: 'chat-compatible',
      capabilities: {
        ...provider.capabilities,
        chat: 'supported',
        text: 'supported',
        streaming: 'supported',
        ...qualified,
      },
    })
  }

  /** Records the verified download as a model profile; optionally makes it the default. */
  function registerManagedModel(entry: CatalogModel, makeDefault: boolean) {
    const profile = profileFor(entry)
    update((current) => {
      const provider = current.providers.find((item) => item.kind === 'juniper-local')
      const providers = provider ? current.providers : [...current.providers, defaultProvider]
      const models = [...current.models.filter((model) => model.catalogId !== entry.id), profile]
      const assistant = defaultAssistantFor(current)
      const next = { ...current, providers, models }
      return makeDefault || !assistant.modelProfileId
        ? setAssistantModel(next, assistant, profile.id)
        : next
    })
    setMessage(
      makeDefault
        ? `${entry.displayName} is now ${defaultAssistant.name}’s default model.`
        : `${entry.displayName} is downloaded and verified. The local engine loads it on first use.`,
    )
  }

  async function download(entry: CatalogModel) {
    if (!runningInTauri) {
      setMessage('Model downloads are available in the Juniper desktop or Android app.')
      return
    }
    controller.current?.abort()
    const next = new AbortController()
    controller.current = next
    setActiveDownload(entry.id)
    setProgress(null)
    setMessage(null)
    try {
      await downloadManagedModel(
        entry.id,
        (event) =>
          setProgress({ completed: event.completedBytes ?? 0, total: event.totalBytes ?? 0 }),
        next.signal,
      )
      setInstalled(await getManagedModels())
      registerManagedModel(entry, false)
    } catch (error) {
      setMessage(
        next.signal.aborted
          ? 'Download paused. Resume when you are ready.'
          : error instanceof Error
            ? error.message
            : 'Download failed.',
      )
    } finally {
      if (controller.current === next) controller.current = null
      setActiveDownload(null)
      setProgress(null)
    }
  }

  /** Installs an import-only model from a file the user already has. */
  async function importFile(entry: CatalogModel) {
    if (!runningInTauri) {
      setMessage('Model import is available in the Juniper desktop app.')
      return
    }
    let selection
    try {
      selection = await pickGguf()
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Could not open the file picker.')
      return
    }
    if (!selection) return
    controller.current?.abort()
    const next = new AbortController()
    controller.current = next
    setActiveDownload(entry.id)
    setProgress(null)
    setMessage(null)
    try {
      await importManagedModel(
        selection.id,
        entry.id,
        (event) =>
          setProgress({ completed: event.completedBytes ?? 0, total: event.totalBytes ?? 0 }),
        next.signal,
      )
      setInstalled(await getManagedModels())
      registerManagedModel(entry, false)
    } catch (error) {
      setMessage(
        next.signal.aborted
          ? 'Import cancelled.'
          : error instanceof Error
            ? error.message
            : 'Import failed.',
      )
    } finally {
      if (controller.current === next) controller.current = null
      setActiveDownload(null)
      setProgress(null)
    }
  }

  async function unload() {
    try {
      await unloadLocalRuntime()
      setRuntimeStatus(await getLocalRuntimeStatus())
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Could not unload the model.')
    }
  }

  async function remove(entry: CatalogModel) {
    const confirmed = await dialogs.confirm({
      title: `Remove ${entry.displayName}?`,
      message:
        'The model file is deleted from this device. Chats that use it will need another model.',
      confirmLabel: 'Remove model',
      danger: true,
    })
    if (!confirmed) return
    try {
      await deleteManagedModel(entry.id)
      setInstalled((current) => current.filter((model) => model.catalogId !== entry.id))
      update((current) => ({
        ...current,
        models: current.models.filter((model) => model.catalogId !== entry.id),
      }))
      setMessage(`${entry.displayName} was removed.`)
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Could not remove the model.')
    }
  }

  const defaultModelId = defaultAssistant.modelProfileId

  return (
    <div className="page models-page">
      <PageHeader
        title="Models"
        description="Download a model that runs on this device, or use one from a connection."
      />

      <div className="device-line" aria-label="This device">
        <span>
          <strong>{device?.deviceName ?? 'This device'}</strong>
        </span>
        <span>Memory: {memoryLabel(device)}</span>
        <span>Free storage: {formatBytes(device?.freeStorageBytes)}</span>
        <span>Local engine: {nativeRuntimeLabel(device)}</span>
        <button className="link-button" onClick={() => void refresh()}>
          Recheck
        </button>
      </div>

      <div className="models-toolbar">
        <Segmented<View>
          label="Show models"
          value={view}
          onChange={setView}
          options={[
            { value: 'recommended', label: 'Recommended' },
            { value: 'all', label: 'All' },
            { value: 'installed', label: 'Installed' },
          ]}
        />
        <label className="search-field">
          <Icon name="search" size={18} />
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search models"
            aria-label="Search models"
          />
        </label>
      </div>

      {message && (
        <div className="notice" role="status">
          {message}
        </div>
      )}

      <div className="model-grid">
        {visible.map((recommendation) => {
          const entry = recommendation.model
          const profile = data.models.find((model) => model.catalogId === entry.id)
          return (
            <CatalogModelCard
              key={entry.id}
              recommendation={recommendation}
              installed={installedIds.has(entry.id)}
              managedState={installed.find((model) => model.catalogId === entry.id)?.state}
              downloading={activeDownload === entry.id}
              progress={activeDownload === entry.id ? progress : null}
              isDefault={Boolean(profile && profile.id === defaultModelId)}
              defaultAssistantName={defaultAssistant.name}
              runtimeOptions={runtimeOptionsForModel(entry, device?.runtimes)}
              resident={
                runtimeStatus.artifactId === entry.artifacts[0]?.id ? runtimeStatus.state : null
              }
              onDownload={() =>
                void (isImportOnly(recommendation.artifact) ? importFile(entry) : download(entry))
              }
              onUnload={() => void unload()}
              onPause={() => controller.current?.abort()}
              onMakeDefault={() => registerManagedModel(entry, true)}
              onRemove={() => void remove(entry)}
            />
          )
        })}
      </div>
      {device && visible.length === 0 && (
        <EmptyState title="No models match this view">
          Try another filter or search term.
        </EmptyState>
      )}
      {installed.length === 0 && view !== 'installed' && (
        <p className="muted small">
          No models installed yet. Download a recommended model to begin.
        </p>
      )}

      <section className="models-section" aria-labelledby="connected-models-title">
        <div className="section-heading-row">
          <div>
            <h2 id="connected-models-title">From your connections</h2>
            <p className="muted">
              Models served by Ollama or another provider you connected. Where they run is always
              labelled.
            </p>
          </div>
          <div className="button-row">
            <button
              className="button secondary"
              onClick={() => void external.refreshModels()}
              disabled={external.refreshing}
            >
              <Icon name="retry" size={18} />
              {external.refreshing ? 'Refreshing…' : 'Refresh'}
            </button>
            <button className="button ghost" onClick={() => openSettings('connections')}>
              Manage connections
            </button>
          </div>
        </div>
        {external.pullStatus && (
          <div className="notice" role="status">
            <span>
              {external.pullStatus}
              {external.pullProgress.total
                ? ` · ${Math.round(((external.pullProgress.completed ?? 0) / external.pullProgress.total) * 100)}%`
                : ''}
            </span>
            {external.pulling && (
              <button className="link-button" onClick={external.cancelPull}>
                Cancel
              </button>
            )}
          </div>
        )}
        {externalModels.length === 0 ? (
          <EmptyState
            title="No connected models"
            action={
              <button className="button ghost" onClick={() => openSettings('connections')}>
                Add a connection
              </button>
            }
          >
            Connect Ollama, a llama.cpp server, or an OpenAI-compatible endpoint to use its models.
          </EmptyState>
        ) : (
          <ul className="connected-models">
            {externalModels.map((model) => {
              const provider = data.providers.find((item) => item.id === model.providerId)
              const running = external.runningModelIds[model.providerId]?.includes(model.modelId)
              return (
                <li key={model.id} className="connected-model">
                  <div className="connected-model-main">
                    <div className="connected-model-title">
                      <h3>{model.displayName}</h3>
                      <LocationBadge location={model.executionLocation} />
                    </div>
                    <p className="muted small">
                      {provider?.name ?? 'Removed connection'} ·{' '}
                      {running ? 'Loaded in runtime' : modelStatusLabel(model)}
                      {model.fileSizeBytes ? ` · ${formatBytes(model.fileSizeBytes)}` : ''}
                    </p>
                    <details className="disclosure">
                      <summary>
                        <Icon name="chevronRight" size={16} />
                        Details
                      </summary>
                      <dl className="detail-list compact">
                        <div>
                          <dt>Tools</dt>
                          <dd>{labelCapability(model.capabilities.tools)}</dd>
                        </div>
                        <div>
                          <dt>Thinking</dt>
                          <dd>{labelCapability(model.capabilities.thinking)}</dd>
                        </div>
                        <div>
                          <dt>Context</dt>
                          <dd>{model.contextLength?.toLocaleString() ?? 'Unknown'}</dd>
                        </div>
                        <div>
                          <dt>Estimated fit</dt>
                          <dd>{modelFitLabel(model, external.hostMemory)}</dd>
                        </div>
                        <div>
                          <dt>Model ID</dt>
                          <dd className="mono">{model.modelId}</dd>
                        </div>
                        <div>
                          <dt>Used by</dt>
                          <dd>
                            {
                              data.assistants.filter(
                                (assistant) => assistant.modelProfileId === model.id,
                              ).length
                            }{' '}
                            assistants
                          </dd>
                        </div>
                        {data.settings.developerMode && (
                          <div>
                            <dt>Runtime metadata</dt>
                            <dd className="mono">
                              {[
                                model.family,
                                model.architecture,
                                model.parameterSize,
                                model.quantization,
                              ]
                                .filter(Boolean)
                                .join(' · ') || 'No additional runtime metadata'}
                              {model.template ? ` · template: ${model.template}` : ''}
                              {model.rawCapabilities?.length
                                ? ` · capabilities: ${model.rawCapabilities.join(', ')}`
                                : ''}
                            </dd>
                          </div>
                        )}
                      </dl>
                    </details>
                  </div>
                  <div className="connected-model-actions">
                    {isChatSelectable(model) &&
                      (model.id === defaultModelId ? (
                        <span className="chip">Default</span>
                      ) : (
                        <button
                          className="button secondary"
                          onClick={() =>
                            update((current) =>
                              setAssistantModel(current, defaultAssistantFor(current), model.id),
                            )
                          }
                        >
                          Set as default
                        </button>
                      ))}
                    {provider?.kind === 'ollama' && (
                      <>
                        {model.status === 'not-found' && (
                          <button
                            className="button ghost"
                            onClick={() => void external.pullModel(model.modelId)}
                          >
                            Re-download
                          </button>
                        )}
                        <button
                          className="icon-button"
                          aria-label={`Delete ${model.displayName}`}
                          title="Delete"
                          onClick={() => void external.deleteModel(model)}
                        >
                          <Icon name="trash" size={18} />
                        </button>
                      </>
                    )}
                  </div>
                </li>
              )
            })}
          </ul>
        )}
        <p className="muted small">
          Import a GGUF file or check model fit in{' '}
          <button className="link-button" onClick={() => openSettings('models-runtime')}>
            Settings › Models &amp; runtime
          </button>
          .
        </p>
      </section>
    </div>
  )
}

function CatalogModelCard({
  recommendation,
  installed,
  managedState,
  downloading,
  progress,
  isDefault,
  defaultAssistantName,
  runtimeOptions,
  resident,
  onDownload,
  onUnload,
  onPause,
  onMakeDefault,
  onRemove,
}: {
  recommendation: ModelRecommendation
  installed: boolean
  managedState?: ManagedModel['state']
  downloading: boolean
  progress: { completed: number; total: number } | null
  isDefault: boolean
  defaultAssistantName: string
  runtimeOptions: ReturnType<typeof runtimeOptionsForModel>
  resident: LocalRuntimeStatus['state'] | null
  onDownload: () => void
  onUnload: () => void
  onPause: () => void
  onMakeDefault: () => void
  onRemove: () => void
}) {
  const { model, artifact } = recommendation
  const percent = progress?.total ? Math.round((progress.completed / progress.total) * 100) : 0
  const usableEngine = runtimeOptions.find((option) => option.selectable)
  const engineUnavailable = runtimeOptions.length > 0 && !usableEngine
  const importOnly = isImportOnly(artifact)
  return (
    <article className="model-card">
      <div className="model-card-head">
        <h3>{model.displayName}</h3>
        <span className={`fit-badge ${recommendation.fit}`}>{fitLabel(recommendation.fit)}</span>
      </div>
      <p className="model-card-description">{model.description}</p>
      <p className="model-card-facts">
        {formatBytes(artifact.sizeBytes)} · Good for {model.useCases.slice(0, 3).join(', ')}
      </p>

      {!recommendation.storageSafe && (
        <p className="warning-line">
          <Icon name="info" size={16} /> Not enough free storage for a safe download.
        </p>
      )}
      {importOnly && !installed && (
        <p className="note-line">
          Juniper does not download this model. Import a copy of the exact file; Juniper checks its
          SHA-256 before using it.
        </p>
      )}
      {engineUnavailable && (
        <p className="warning-line">
          <Icon name="info" size={16} /> Juniper’s local engine for this model isn’t available on
          this device.
        </p>
      )}
      {usableEngine && usableEngine.runtime.maturity !== 'stable' && (
        <p className="note-line">
          Runs with {usableEngine.runtime.name}, which is {usableEngine.runtime.maturity} on this
          device.
        </p>
      )}
      {managedState && managedState !== 'ready' && (
        <p className="warning-line">
          <Icon name="info" size={16} />
          {managedState === 'partial'
            ? 'A partial download is saved; downloading again resumes it.'
            : 'The local file failed verification and will be replaced.'}
        </p>
      )}
      {installed && (
        <p className="success-line" role="status">
          <Icon name="check" size={16} /> {importOnly ? 'Imported' : 'Downloaded'} and verified
          {resident === 'ready' || resident === 'busy' ? ' · loaded in memory' : ''}
          {resident === 'loading' ? ' · loading' : ''}
        </p>
      )}
      {downloading && (
        <div className="progress-block" role="status">
          <div>
            <span>{importOnly ? 'Verifying the file' : 'Downloading and verifying'}</span>
            <strong>{percent}%</strong>
          </div>
          <progress value={percent} max="100" aria-label={`${model.displayName} download`} />
        </div>
      )}

      <div className="model-card-actions">
        {installed ? (
          <>
            {isDefault ? (
              <span className="chip">Default for {defaultAssistantName}</span>
            ) : (
              <button className="button primary" onClick={onMakeDefault}>
                Use as default
              </button>
            )}
            {resident === 'ready' && (
              <button className="button secondary" onClick={onUnload}>
                Unload from memory
              </button>
            )}
            <button className="button ghost" onClick={onRemove}>
              Remove
            </button>
          </>
        ) : downloading ? (
          <button className="button secondary" onClick={onPause}>
            {importOnly ? 'Cancel import' : 'Pause download'}
          </button>
        ) : (
          <>
            <button
              className="button primary"
              onClick={onDownload}
              disabled={!importOnly && !recommendation.storageSafe}
            >
              {importOnly
                ? 'Import verified file…'
                : recommendation.storageSafe
                  ? managedState === 'partial'
                    ? 'Resume download'
                    : `Download · ${formatBytes(artifact.sizeBytes)}`
                  : 'Not enough storage'}
            </button>
            {managedState && (
              <button className="button ghost" onClick={onRemove}>
                Remove
              </button>
            )}
          </>
        )}
      </div>

      <details className="disclosure">
        <summary>
          <Icon name="chevronRight" size={16} />
          Details
        </summary>
        <dl className="detail-list compact">
          <div>
            <dt>Why this fit</dt>
            <dd>{recommendation.reasons.join('. ') || 'Compatible with this device.'}</dd>
          </div>
          <div>
            <dt>Parameters</dt>
            <dd>{parameterLabel(model.parameterCount)}</dd>
          </div>
          <div>
            <dt>Format</dt>
            <dd>
              {artifact.format}
              {artifact.quantization ? ` · ${artifact.quantization}` : ''}
            </dd>
          </div>
          <div>
            <dt>Engines</dt>
            <dd>
              {runtimeOptions.length === 0
                ? 'Native runtime data unavailable'
                : runtimeOptions
                    .map(
                      ({ runtime, artifact: runtimeArtifact, selectable }) =>
                        `${runtime.name} · ${runtime.maturity}${!runtimeArtifact ? ' · no compatible artifact' : selectable ? '' : ' · unavailable'}`,
                    )
                    .join('; ')}
            </dd>
          </div>
          <div>
            <dt>Qualification</dt>
            <dd>{artifact.qualification}</dd>
          </div>
          <div>
            <dt>Context</dt>
            <dd>{model.contextLength.toLocaleString()} tokens</dd>
          </div>
          <div>
            <dt>License</dt>
            <dd>{model.license}</dd>
          </div>
          <div>
            <dt>Source</dt>
            <dd>
              <a href={model.sourceRepository} target="_blank" rel="noreferrer">
                {model.organization}
              </a>
            </dd>
          </div>
          <div>
            <dt>Revision</dt>
            <dd className="mono">{artifact.sourceRevision}</dd>
          </div>
          <div>
            <dt>SHA-256</dt>
            <dd className="mono hash">
              {artifact.sha256 ?? artifact.files.map((file) => file.sha256).join(', ')}
            </dd>
          </div>
          <div>
            <dt>File</dt>
            <dd className="mono">{artifact.files.map((file) => file.path).join(', ')}</dd>
          </div>
        </dl>
      </details>
    </article>
  )
}
