import { describe, expect, it } from 'vitest'
import { releaseNumbers } from './lib/version-scheme.mjs'

describe('release version scheme', () => {
  it('derives Android and MSI numbers for the 0.4.0 alpha', () => {
    expect(releaseNumbers('0.4.0-alpha.1')).toEqual({
      channel: 'alpha',
      number: 1,
      androidVersionCode: 4001,
      msiVersion: '0.4.0.1',
    })
  })

  it('keeps the published 0.3.0 release candidates as they were', () => {
    expect(releaseNumbers('0.3.0-rc.33')).toMatchObject({
      androidVersionCode: 3033,
      msiVersion: '0.3.0.33',
    })
  })

  it('orders every prerelease of a line above the previous line and alpha < beta < rc', () => {
    const order = [
      '0.3.0-rc.33',
      '0.4.0-alpha.1',
      '0.4.0-alpha.2',
      '0.4.0-beta.1',
      '0.4.0-rc.1',
      '0.5.0-alpha.1',
    ]
    const codes = order.map((version) => releaseNumbers(version).androidVersionCode)
    expect([...codes].sort((a, b) => a - b)).toEqual(codes)
    expect(new Set(codes).size).toBe(codes.length)
    expect(releaseNumbers('0.4.0-beta.1').msiVersion).toBe('0.4.0.301')
    expect(releaseNumbers('0.4.0-rc.2').msiVersion).toBe('0.4.0.602')
  })

  it('rejects a bare stable version and malformed prereleases', () => {
    for (const version of [
      '0.4.0',
      '0.4.0-Alpha',
      '0.4.0-alpha',
      '0.4.0-alpha.0',
      '0.4.0-alpha.300',
      '0.4.0-gamma.1',
      '0.4.1-alpha.1',
      '1.0.0-alpha.1',
      '0.3.0-alpha.1',
      'v0.4.0-alpha.1',
    ]) {
      expect(() => releaseNumbers(version), version).toThrow()
    }
  })
})
