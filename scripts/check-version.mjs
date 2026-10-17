/* global URL, console, process */

import { readFile } from 'node:fs/promises'
import { releaseNumbers } from './lib/version-scheme.mjs'

const packageJson = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'))
const tauri = JSON.parse(
  await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
)
const cargo = await readFile(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8')
const pluginCargo = await readFile(
  new URL('../src-tauri/plugins/juniper-local/Cargo.toml', import.meta.url),
  'utf8',
)
const commands = await readFile(new URL('../src-tauri/src/commands.rs', import.meta.url), 'utf8')
const runtime = await readFile(new URL('../src/lib/runtime.ts', import.meta.url), 'utf8')
const manifest = await readFile(
  new URL('../manifests/release-candidate.yaml', import.meta.url),
  'utf8',
)
const expected = packageJson.version
const cargoVersion = cargo.match(/^version\s*=\s*"([^"]+)"$/m)?.[1]
const pluginVersion = pluginCargo.match(/^version\s*=\s*"([^"]+)"$/m)?.[1]
const versionLabel = 'Juniper ' + expected
const diagnosticsMatch = commands.includes(versionLabel) && runtime.includes(versionLabel)
const staleReleaseMarkers = /PENDING SOL|PENDING-MERGE|published_artifacts:\s*\[\s*\]/i
const expectedArtifactNames = [
  `Juniper-${expected}-linux-x86_64.deb`,
  `Juniper-${expected}-linux-x86_64.AppImage`,
  `Juniper-${expected}-windows-x86_64.msi`,
  `Juniper-${expected}-android-universal.apk`,
  `Juniper-${expected}-android-native-symbols.zip`,
]
if (
  tauri.version !== expected ||
  cargoVersion !== expected ||
  pluginVersion !== expected ||
  !diagnosticsMatch ||
  !manifest.includes(`version: ${expected}`) ||
  !manifest.includes(`tag: v${expected}`) ||
  staleReleaseMarkers.test(manifest) ||
  expectedArtifactNames.some((name) => !manifest.includes(name))
) {
  throw new Error(
    `Version mismatch: expected ${expected} across package, Cargo, Tauri, and release manifest.`,
  )
}
// The GitHub release takes its title from the manifest's display name
// ("Juniper 0.4.0-Alpha" for 0.4.0-alpha.1) and must stay a prerelease.
const displayName = manifest.match(/^display_name:\s*(.+)$/m)?.[1]?.trim()
if (
  !displayName ||
  !displayName.startsWith(`Juniper ${expected.split('-')[0]}-`) ||
  !/^\s+prerelease: true$/m.test(manifest)
) {
  throw new Error(
    `Release manifest needs a display_name for ${expected} and prerelease: true (got ${displayName}).`,
  )
}
const releaseTag = process.env.RELEASE_TAG
if (releaseTag && releaseTag !== `v${expected}`) {
  throw new Error(`Release tag mismatch: expected v${expected}, received ${releaseTag}.`)
}
const { androidVersionCode: expectedAndroidVersionCode, msiVersion: expectedMsiVersion } =
  releaseNumbers(expected)
const androidVersionCode = tauri.bundle?.android?.versionCode
if (androidVersionCode !== expectedAndroidVersionCode) {
  throw new Error(
    `Android versionCode mismatch: ${expected} must use ${expectedAndroidVersionCode}.`,
  )
}
const msiVersion = tauri.bundle?.windows?.wix?.version
if (msiVersion !== expectedMsiVersion) {
  throw new Error(
    `MSI version mismatch: ${expected} must use Windows Installer version ${expectedMsiVersion}.`,
  )
}
console.log(`Version consistency passed: ${expected}`)
