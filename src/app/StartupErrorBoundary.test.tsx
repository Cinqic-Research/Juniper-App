import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'

afterEach(() => {
  Reflect.deleteProperty(window, '__TAURI_INTERNALS__')
  vi.restoreAllMocks()
})

it('replaces a crashed interface with a visible error and a local native report', async () => {
  const reports: unknown[] = []
  Object.defineProperty(window, '__TAURI_INTERNALS__', {
    configurable: true,
    value: {
      transformCallback: () => 0,
      invoke: async (command: string, args?: Record<string, unknown>) => {
        if (command === 'frontend_fatal') reports.push(args?.report)
        return null
      },
    },
  })
  vi.spyOn(console, 'error').mockImplementation(() => undefined)
  const { StartupErrorBoundary } = await import('./StartupErrorBoundary')
  function Broken(): never {
    throw new ReferenceError('modelProfileFromDiscovery is not defined')
  }
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <StartupErrorBoundary>
        <Broken />
      </StartupErrorBoundary>,
    )
  })

  expect(container.textContent).toContain('Juniper could not display its interface')
  expect(container.textContent).toContain(
    'ReferenceError: modelProfileFromDiscovery is not defined',
  )
  expect(reports).toHaveLength(1)
  expect(reports[0]).toBe('interface ReferenceError')
  act(() => root.unmount())
  container.remove()
})
