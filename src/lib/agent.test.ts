import { describe, expect, it } from 'vitest'
import { initialAppData } from './defaults'
import { applyMemoryChanges, composeAgentProfile, errorText } from './agent'

describe('agent helpers', () => {
  it('applies an approved memory change once, however often it is delivered', () => {
    const data = initialAppData()
    const change = {
      id: 'change-1',
      assistantId: data.assistants[0]!.id,
      operation: 'save' as const,
      payload: { content: 'Prefers metric units.' },
      createdAt: '2026-10-15T07:00:00Z',
    }
    const once = applyMemoryChanges(data, [change])
    const twice = applyMemoryChanges(once, [change])
    expect(once.memories).toHaveLength(data.memories.length + 1)
    expect(twice).toBe(once)
    const saved = once.memories.at(-1)!
    expect(saved).toMatchObject({
      content: 'Prefers metric units.',
      source: 'agent-task',
      enabled: true,
    })
  })

  it('composes the assistant style without granting anything', () => {
    const assistant = initialAppData().assistants[0]!
    const profile = composeAgentProfile(assistant)
    expect(profile).toContain('Compiled personality controls')
    expect(profile).not.toMatch(/permission|grant/i)
  })

  it('shows the readable part of a host error', () => {
    expect(errorText(new Error('AGENT_STOPPED: Agent work is stopped.'))).toBe(
      'Agent work is stopped.',
    )
    expect(errorText('plain failure')).toBe('plain failure')
  })
})
