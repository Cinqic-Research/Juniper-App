import { useEffect, useRef, useState } from 'react'
import { modelProfileFromDiscovery } from '../lib/defaults'
import {
  deleteProviderModel,
  getDiagnostics,
  importGguf,
  inspectProviderModel,
  listProviderModels,
  modelFromInspection,
  pickGguf,
  pullProviderModel,
  runningInTauri,
  runningProviderModels,
} from '../lib/runtime'
import type { AppData, GgufSelection, ModelProfile, ProviderProfile } from '../types'
import { useDialogs } from './overlays'

type Update = (change: (current: AppData) => AppData) => void

export function isExternalModel(data: AppData, model: ModelProfile): boolean {
  return (
    data.providers.find((provider) => provider.id === model.providerId)?.kind !== 'juniper-local'
  )
}

/** Discovery, download, import, and deletion for models served by external providers. */
export function useExternalModels(data: AppData, update: Update) {
  const dialogs = useDialogs()
  const [refreshing, setRefreshing] = useState(false)
  const [pullStatus, setPullStatus] = useState<string | null>(null)
  const [pullProgress, setPullProgress] = useState<{ completed?: number; total?: number }>({})
  const pullController = useRef<AbortController | null>(null)
  const [pulling, setPulling] = useState(false)
  const [ggufSelection, setGgufSelection] = useState<GgufSelection | null>(null)
  const [ggufModelName, setGgufModelName] = useState('local-gguf')
  const [ggufStatus, setGgufStatus] = useState<string | null>(null)
  const ggufController = useRef<AbortController | null>(null)
  const ggufPickerBusy = useRef(false)
  const [importing, setImporting] = useState(false)
  const [runningModelIds, setRunningModelIds] = useState<Record<string, string[]>>({})
  const [hostMemory, setHostMemory] = useState<string | null>(null)

  useEffect(() => {
    if (!runningInTauri) return
    let active = true
    void Promise.all(
      data.providers
        .filter((provider) => provider.enabled && provider.kind !== 'juniper-local')
        .map(async (provider) => {
          try {
            const models = await runningProviderModels(provider)
            return [
              provider.id,
              models.flatMap((model) => {
                const name = model.name ?? model.model
                return typeof name === 'string' ? [name] : []
              }),
            ] as const
          } catch {
            return [provider.id, []] as const
          }
        }),
    ).then((entries) => {
      if (active) setRunningModelIds(Object.fromEntries(entries))
    })
    void getDiagnostics().then((info) => {
      if (active) setHostMemory(info.memory ?? null)
    })
    return () => {
      active = false
    }
  }, [data.providers])

  async function refreshModels() {
    setRefreshing(true)
    const discovered: Array<{
      provider: ProviderProfile
      modelIds: Awaited<ReturnType<typeof listProviderModels>>
    }> = []
    const failedProviderIds = new Set<string>()
    for (const provider of data.providers.filter(
      (item) => item.enabled && item.kind !== 'juniper-local',
    )) {
      try {
        discovered.push({ provider, modelIds: await listProviderModels(provider) })
      } catch {
        // A provider can be offline while another provider remains usable.
        failedProviderIds.add(provider.id)
      }
    }
    const normalized: ModelProfile[] = []
    for (const { provider, modelIds } of discovered) {
      for (const discoveredModel of modelIds) {
        const existing = data.models.find(
          (model) => model.providerId === provider.id && model.modelId === discoveredModel.modelId,
        )
        try {
          const inspection = await inspectProviderModel(provider, discoveredModel.modelId)
          normalized.push(modelFromInspection(provider, inspection, existing))
        } catch {
          normalized.push(
            modelProfileFromDiscovery(provider, discoveredModel.modelId, {
              ...existing,
              displayName: discoveredModel.displayName,
              fileSizeBytes: discoveredModel.sizeBytes,
            }),
          )
        }
      }
    }
    const refreshedProviderIds = new Set(discovered.map(({ provider }) => provider.id))
    const discoveredModelIds = new Set(normalized.map((model) => model.id))
    if (refreshedProviderIds.size || failedProviderIds.size) {
      update((current) => ({
        ...current,
        providers: current.providers.map((provider) =>
          refreshedProviderIds.has(provider.id)
            ? { ...provider, status: 'connected' }
            : failedProviderIds.has(provider.id)
              ? { ...provider, status: 'offline' }
              : provider,
        ),
        models: [
          ...current.models
            .filter((model) => !normalized.some((item) => item.id === model.id))
            .map((model) =>
              refreshedProviderIds.has(model.providerId) && !discoveredModelIds.has(model.id)
                ? { ...model, status: 'not-found' as const }
                : model,
            ),
          ...normalized,
        ],
      }))
    }
    setRefreshing(false)
  }

  /** Resolves true only when the pull completed. */
  async function pullModel(reference: string): Promise<boolean> {
    const trimmed = reference.trim()
    const provider = data.providers.find((item) => item.kind === 'ollama' && item.enabled)
    if (!trimmed) return false
    if (!provider) {
      setPullStatus('Add and enable an Ollama connection to download this model.')
      return false
    }
    pullController.current?.abort()
    const controller = new AbortController()
    pullController.current = controller
    setPulling(true)
    setPullStatus('Resolving')
    setPullProgress({})
    try {
      await pullProviderModel(
        provider,
        trimmed,
        (progress) => {
          setPullStatus(progress.status)
          setPullProgress({ completed: progress.completedBytes, total: progress.totalBytes })
        },
        controller.signal,
      )
      setPullStatus('Complete')
      await refreshModels()
      return true
    } catch (error) {
      setPullStatus(
        controller.signal.aborted
          ? 'Cancelled'
          : error instanceof Error
            ? error.message
            : 'Download failed',
      )
      return false
    } finally {
      pullController.current = null
      setPulling(false)
    }
  }

  function cancelPull() {
    pullController.current?.abort()
  }

  async function chooseGguf() {
    if (ggufPickerBusy.current) return
    ggufPickerBusy.current = true
    try {
      const selection = await pickGguf()
      if (selection) {
        setGgufSelection(selection)
        setGgufModelName(selection.name.replace(/\.gguf$/i, '').replace(/[^a-z0-9._/-]+/gi, '-'))
      }
    } catch (error) {
      void dialogs.notify(
        error instanceof Error ? error.message : 'Could not select that GGUF file.',
      )
    } finally {
      ggufPickerBusy.current = false
    }
  }

  async function importSelectedGguf() {
    if (!ggufSelection || !ggufModelName.trim()) return
    ggufController.current?.abort()
    const controller = new AbortController()
    ggufController.current = controller
    setImporting(true)
    setGgufStatus('Preparing import')
    try {
      await importGguf(
        ggufSelection.id,
        ggufModelName.trim(),
        (progress) => setGgufStatus(progress.status),
        controller.signal,
      )
      setGgufStatus('Complete')
      await refreshModels()
    } catch (error) {
      setGgufStatus(
        controller.signal.aborted
          ? 'Cancelled'
          : error instanceof Error
            ? error.message
            : 'Import failed',
      )
    } finally {
      ggufController.current = null
      setImporting(false)
    }
  }

  function cancelGguf() {
    ggufController.current?.abort()
  }

  async function deleteModel(model: ModelProfile) {
    const provider = data.providers.find((item) => item.id === model.providerId)
    if (!provider) return
    const confirmed = await dialogs.confirm({
      title: `Delete ${model.displayName}?`,
      message: `${model.modelId} will be deleted from ${provider.name}. Chats that use it will need another model.`,
      confirmLabel: 'Delete model',
      danger: true,
    })
    if (!confirmed) return
    try {
      await deleteProviderModel(provider, model.modelId)
      update((current) => ({
        ...current,
        models: current.models.map((item) =>
          item.id === model.id ? { ...item, status: 'not-found' } : item,
        ),
      }))
    } catch (error) {
      void dialogs.notify(error instanceof Error ? error.message : 'Could not delete the model.')
    }
  }

  return {
    refreshing,
    refreshModels,
    pullStatus,
    pullProgress,
    pulling,
    pullModel,
    cancelPull,
    ggufSelection,
    ggufModelName,
    setGgufModelName,
    ggufStatus,
    importing,
    chooseGguf,
    importSelectedGguf,
    cancelGguf,
    deleteModel,
    runningModelIds,
    hostMemory,
  }
}
