/* global URL, console */

import { readFile } from 'node:fs/promises'

const read = (path) => readFile(new URL(`../${path}`, import.meta.url), 'utf8')
const [
  configText,
  rust,
  typescript,
  gradle,
  cmake,
  apkCheck,
  validation,
  release,
  gptOssText,
  catalogText,
] = await Promise.all([
  read('config/llama-cpp.json'),
  read('src-tauri/src/runtime_registry.rs'),
  read('src/lib/runtime-registry.ts'),
  read('src-tauri/plugins/juniper-local/android/build.gradle.kts'),
  read('src-tauri/plugins/juniper-local/android/src/main/cpp/CMakeLists.txt'),
  read('scripts/verify-android-apk.sh'),
  read('.github/workflows/validation.yml'),
  read('.github/workflows/release.yml'),
  read('config/backends/gpt-oss-20b.json'),
  read('config/models/catalog.json'),
])
const config = JSON.parse(configText)
const expectedPins = new Map([
  ['llama.cpp', config.commit],
  ['litert-lm', 'v0.16.1'],
  ['executorch', 'v1.4.1'],
  ['mlc-llm', '9fa644f54b04983adea4d0168f49fc6af4a893ba'],
  ['onnxruntime-genai', 'v0.15.2'],
])

for (const [id, pin] of expectedPins) {
  if (!typescript.includes(`id: '${id}'`) || !rust.includes(`id: "${id}"`)) {
    throw new Error(`Runtime ${id} is missing from a registry boundary.`)
  }
  if (!typescript.includes(pin) || !rust.includes(pin)) {
    throw new Error(`Runtime ${id} pin ${pin} is inconsistent across registry boundaries.`)
  }
}

if (!Array.isArray(config.androidAbis) || config.androidAbis.join(' ') !== 'arm64-v8a x86_64') {
  throw new Error('Android ABI policy must be exactly arm64-v8a and x86_64.')
}
for (const abi of config.androidAbis) {
  for (const [path, contents] of [
    ['Gradle', gradle],
    ['CMake', cmake],
    ['APK verifier', apkCheck],
  ]) {
    if (!contents.includes(abi)) throw new Error(`${path} does not enforce Android ABI ${abi}.`)
  }
}
for (const workflow of [validation, release]) {
  if (!workflow.includes(`'ndk;${config.ndkVersion}'`)) {
    throw new Error(`Workflow NDK pin does not match ${config.ndkVersion}.`)
  }
  if (!workflow.includes(`'cmake;${config.cmakeVersion}'`)) {
    throw new Error(`Workflow CMake pin does not match ${config.cmakeVersion}.`)
  }
}
if (!gradle.includes(`ndkVersion = "${config.ndkVersion}"`)) {
  throw new Error('Gradle NDK pin differs from config/llama-cpp.json.')
}
if (!gradle.includes(`version = "${config.cmakeVersion}"`)) {
  throw new Error('Gradle CMake pin differs from config/llama-cpp.json.')
}

// A qualified backend profile, the CUDA build pin, and the catalog artifact
// that names the profile must describe the same runtime and file.
const gptOss = JSON.parse(gptOssText)
if (config.desktopCuda?.commit !== gptOss.runtime.commit) {
  throw new Error('config/llama-cpp.json desktopCuda.commit differs from the GPT-OSS profile.')
}
if (config.desktopCuda?.tag !== gptOss.runtime.tag) {
  throw new Error('config/llama-cpp.json desktopCuda.tag differs from the GPT-OSS profile.')
}
const profiled = JSON.parse(catalogText)
  .models.flatMap((model) => model.artifacts)
  .filter((artifact) => artifact.backendProfile === gptOss.id)
if (
  profiled.length !== 1 ||
  profiled[0].id !== gptOss.artifact.id ||
  profiled[0].sha256 !== gptOss.artifact.sha256 ||
  profiled[0].sizeBytes !== gptOss.artifact.sizeBytes
) {
  throw new Error('The GPT-OSS backend profile and its catalog artifact disagree.')
}

console.log(
  `Runtime contract passed: ${expectedPins.size} pins; Android ABIs ${config.androidAbis.join(', ')}`,
)
