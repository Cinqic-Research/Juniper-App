import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatRequest, ProviderProfile } from '../types'

const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke, addPluginListener: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))

type Runtime = typeof import('./runtime')
const operations: Array<{
  name: string
  run: (runtime: Runtime, signal: AbortSignal) => Promise<void>
}> = [
  {
    name: 'chat',
    run: (runtime, signal) =>
      runtime.streamChat({ requestId: 'test' } as ChatRequest, () => undefined, signal),
  },
  {
    name: 'provider pull',
    run: (runtime, signal) =>
      runtime.pullProviderModel({} as ProviderProfile, 'model', () => undefined, signal),
  },
  {
    name: 'managed download',
    run: (runtime, signal) =>
      runtime.downloadManagedModel('model', 'artifact', () => undefined, signal),
  },
  {
    name: 'managed import',
    run: (runtime, signal) =>
      runtime.importManagedModel('selection', 'model', 'artifact', () => undefined, signal),
  },
  {
    name: 'GGUF import',
    run: (runtime, signal) => runtime.importGguf('selection', 'model', () => undefined, signal),
  },
]

beforeEach(() => {
  vi.resetModules()
  invoke.mockReset().mockResolvedValue(undefined)
  listen.mockReset()
  Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} })
})

afterEach(() => {
  Reflect.deleteProperty(window, '__TAURI_INTERNALS__')
})

describe.each(operations)('$name cancellation before native execution', ({ run }) => {
  it('does not start an operation with an already aborted signal', async () => {
    const runtime = await import('./runtime')
    const controller = new AbortController()
    controller.abort()
    await expect(run(runtime, controller.signal)).rejects.toMatchObject({ name: 'AbortError' })
    expect(listen).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalled()
  })

  it('does not start when cancellation arrives while the event listener is registering', async () => {
    const runtime = await import('./runtime')
    const controller = new AbortController()
    const unlisten = vi.fn()
    let register!: (stop: () => void) => void
    listen.mockImplementation(() => new Promise<() => void>((resolve) => (register = resolve)))
    const pending = run(runtime, controller.signal)
    const rejected = expect(pending).rejects.toMatchObject({ name: 'AbortError' })
    expect(listen).toHaveBeenCalledOnce()
    controller.abort()
    register(unlisten)
    await rejected
    expect(invoke).not.toHaveBeenCalled()
    expect(unlisten).toHaveBeenCalledOnce()
  })
})

it('still cancels a managed download after native execution has started', async () => {
  const runtime = await import('./runtime')
  const controller = new AbortController()
  const unlisten = vi.fn()
  listen.mockResolvedValue(unlisten)
  let finish!: () => void
  invoke.mockImplementation((command: string) =>
    command === 'download_managed_model'
      ? new Promise<void>((resolve) => (finish = resolve))
      : Promise.resolve(),
  )
  const pending = runtime.downloadManagedModel(
    'model',
    'artifact',
    () => undefined,
    controller.signal,
  )
  const rejected = expect(pending).rejects.toMatchObject({ name: 'AbortError' })
  await vi.waitFor(() =>
    expect(invoke).toHaveBeenCalledWith('download_managed_model', expect.anything()),
  )
  controller.abort()
  expect(invoke).toHaveBeenCalledWith('cancel_managed_model', expect.anything())
  finish()
  await rejected
  expect(unlisten).toHaveBeenCalledOnce()
})
