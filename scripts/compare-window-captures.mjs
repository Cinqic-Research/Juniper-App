/* global console, process */
import { readFile } from 'node:fs/promises'
import { decodeXwd } from './lib/images.mjs'

const [beforePath, afterPath, mode] = process.argv.slice(2)
if (!beforePath || !afterPath || (mode && mode !== '--composer')) {
  console.error('Usage: compare-window-captures.mjs BEFORE.xwd AFTER.xwd [--composer]')
  process.exit(2)
}
const before = decodeXwd(await readFile(beforePath))
const after = decodeXwd(await readFile(afterPath))
if (before.width !== after.width || before.height !== after.height) {
  console.error('Window dimensions changed during picker probe')
  process.exit(3)
}
let changed = 0
let sampled = 0
const bounds =
  mode === '--composer'
    ? { left: 380, top: before.height - 160, right: before.width - 50, bottom: before.height - 20 }
    : { left: 280, top: 60, right: before.width - 30, bottom: before.height - 80 }
for (let y = bounds.top; y < bounds.bottom; y += 8) {
  for (let x = bounds.left; x < bounds.right; x += 8) {
    const index = (y * before.width + x) * 4
    const delta =
      Math.abs(before.pixels[index] - after.pixels[index]) +
      Math.abs(before.pixels[index + 1] - after.pixels[index + 1]) +
      Math.abs(before.pixels[index + 2] - after.pixels[index + 2])
    if (delta > 45) changed += 1
    sampled += 1
  }
}
const fraction = changed / sampled
console.log(JSON.stringify({ changed, sampled, fraction, mode: mode ?? 'content' }))
process.exit(fraction >= (mode === '--composer' ? 0.02 : 0.05) ? 0 : 3)
