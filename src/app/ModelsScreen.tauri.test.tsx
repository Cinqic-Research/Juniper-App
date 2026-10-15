import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it } from 'vitest'
import { initialAppData } from '../lib/defaults'
import { MODEL_CATALOG, type DeviceCapabilities } from '../lib/model-catalog'
import { runtimeRegistryForDevice } from '../lib/runtime-registry'
import type { AppData, ManagedModel } from '../types'

Object.defineProperty(window.navigator, 'userAgent', {
  configurable: true,
  value: 'Mozilla/5.0 (X11; Linux x86_64) Juniper-test',
})

const STORAGE_KEY = 'juniper.app-data.v1'
const catalogModel = MODEL_CATALOG.models.find((model) => model.id === 'qwen3-4b')!
const firstArtifact = catalogModel.artifacts[0]!
const selectedArtifact = catalogModel.artifacts[1]!
const device: DeviceCapabilities = {
  os: 'linux',
  deviceName: 'Test desktop',
  architecture: 'x86_64',
  cpuArchitecture: 'x86_64',
  logicalCores: 8,
  totalMemoryBytes: 64 * 1024 ** 3,
  availableMemoryBytes: 48 * 1024 ** 3,
  memoryPressure: 'low',
  totalStorageBytes: 500 * 1024 ** 3,
  freeStorageBytes: 200 * 1024 ** 3,
  modelDirectory: '/models',
  gpu: 'not-detected',
  acceleration: 'not-detected',
  nativeRuntimeAvailable: true,
  nativeRuntimeState: 'ready',
  runtimes: runtimeRegistryForDevice({
    os: 'linux',
    architecture: 'x86_64',
    nativeRuntimeAvailable: true,
  }),
}

const installed: ManagedModel[] = [firstArtifact, selectedArtifact].map((artifact) => ({
  catalogId: catalogModel.id,
  artifactId: artifact.id,
  variantId: artifact.id,
  fileName: artifact.files[0]!.path,
  path: `/models/${artifact.files[0]!.path}`,
  sizeBytes: artifact.sizeBytes,
  sha256: artifact.sha256!,
  verified: true,
  state: 'ready',
  fileCount: artifact.files.length,
}))

describe('Models screen native integration', () => {
  let container: HTMLDivElement | undefined
  let root: Root | undefined

  afterEach(() => {
    act(() => root?.unmount())
    container?.remove()
    document.body.innerHTML = ''
    Reflect.deleteProperty(window, '__TAURI_INTERNALS__')
  })

  it('makes the selected installed artifact the default model', async () => {
    const data = initialAppData()
    data.settings.onboardingComplete = true
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', {
      configurable: true,
      value: { unregisterListener: () => undefined },
    })
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      value: {
        transformCallback: () => 0,
        invoke: async (command: string, args?: Record<string, unknown>) => {
          switch (command) {
            case 'load_app_data':
              return data
            case 'model_catalog':
              return MODEL_CATALOG.models
            case 'device_capabilities':
              return device
            case 'managed_models':
              return installed
            case 'juniper1_models':
              return []
            case 'local_runtime_status':
              return { state: 'idle' }
            case 'system_info':
              return { memory: '64 GiB' }
            case 'save_app_data':
              localStorage.setItem(STORAGE_KEY, JSON.stringify(args?.data))
              return null
            default:
              return null
          }
        },
      },
    })

    const { default: App } = await import('./App')
    container = document.createElement('div')
    document.body.append(container)
    await act(async () => {
      root = createRoot(container!)
      root.render(<App />)
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 30))
    })
    const modelsButton = Array.from(document.querySelectorAll('.sidebar-nav button')).find(
      (button) => button.textContent?.includes('Models'),
    )!
    await act(async () => (modelsButton as HTMLButtonElement).click())
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 30))
    })
    const all = container.querySelector<HTMLInputElement>('.segmented input[value="all"]')!
    await act(async () => all.click())
    const qwenCard = Array.from(container.querySelectorAll('article')).find(
      (card) => card.querySelector('h3')?.textContent === catalogModel.displayName,
    )!
    const version = qwenCard.querySelector<HTMLSelectElement>('.artifact-picker select')!
    await act(async () => {
      version.value = selectedArtifact.id
      version.dispatchEvent(new Event('change', { bubbles: true }))
    })
    expect(version.value).toBe(selectedArtifact.id)
    const makeDefault = Array.from(qwenCard.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Use as default',
    )!
    await act(async () => (makeDefault as HTMLButtonElement).click())

    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') as AppData
    const profile = saved.models.find((model) => model.catalogId === catalogModel.id)
    expect(profile?.artifactId).toBe(selectedArtifact.id)
    expect(profile?.runtimeBinding?.artifactId).toBe(selectedArtifact.id)
    expect(
      saved.assistants.find((assistant) => assistant.id === 'assistant-juniper')?.modelProfileId,
    ).toBe(profile?.id)
  })
})
