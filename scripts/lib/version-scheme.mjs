/**
 * Juniper's machine version scheme and the platform numbers derived from it.
 *
 * - 0.3.0-rc.N (N 1-999): Android versionCode 3000 + N, MSI 0.3.0.N (legacy).
 * - 0.MINOR.0-alpha.N / -beta.N / -rc.N for MINOR 4-9 (N 1-299): Android
 *   versionCode MINOR*1000 + {alpha 0, beta 300, rc 600} + N, and MSI
 *   0.MINOR.0.(versionCode - MINOR*1000). Every prerelease of a minor line
 *   therefore sorts above the previous line and in alpha < beta < rc order,
 *   in both numbers. MINOR*1000 + 900 is reserved for the final release.
 *
 * A bare stable version (for example 0.4.0) is rejected: the final release
 * of a line needs its own decision and its own change to this table.
 *
 * Public display names are not machine versions: "Juniper 0.4.0-Alpha" is
 * the display name of 0.4.0-alpha.1 (docs/release/).
 */
const CHANNEL_OFFSET = { alpha: 0, beta: 300, rc: 600 }

export function releaseNumbers(version) {
  const legacy = /^0\.3\.0-rc\.([1-9][0-9]*)$/.exec(version)
  if (legacy) {
    const number = Number(legacy[1])
    if (!Number.isSafeInteger(number) || number > 999) {
      throw new Error(`Release-candidate number is outside the supported range: ${version}`)
    }
    return {
      channel: 'rc',
      number,
      androidVersionCode: 3000 + number,
      msiVersion: `0.3.0.${number}`,
    }
  }
  const match = /^0\.([4-9])\.0-(alpha|beta|rc)\.([1-9][0-9]*)$/.exec(version)
  if (!match) {
    throw new Error(
      `Unsupported version: ${version}. Use 0.MINOR.0-alpha.N, -beta.N or -rc.N (MINOR 4-9), or 0.3.0-rc.N.`,
    )
  }
  const minor = Number(match[1])
  const channel = match[2]
  const number = Number(match[3])
  if (!Number.isSafeInteger(number) || number > 299) {
    throw new Error(`Prerelease number is outside the supported range 1-299: ${version}`)
  }
  const step = CHANNEL_OFFSET[channel] + number
  return {
    channel,
    number,
    androidVersionCode: minor * 1000 + step,
    msiVersion: `0.${minor}.0.${step}`,
  }
}
