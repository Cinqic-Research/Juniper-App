/* global console, process, URL */

// Rescores wrapper-evaluation results (src-tauri/src/live_gpt_oss.rs) without
// rerunning the model. Scoring v1 is what the harness recorded; v1.1 also
// strips Markdown emphasis and code marks before matching, because answers
// such as "does **not** exist" otherwise fail patterns they satisfy. Both are
// reported; neither replaces reading the answers.
//
//   node scripts/score-wrapper-eval.mjs results.jsonl [frozen-heldout-cases.jsonl]

import { readFileSync } from 'node:fs'

const [resultsPath, heldoutPath] = process.argv.slice(2)
if (!resultsPath) {
  console.error('usage: score-wrapper-eval.mjs results.jsonl [heldout-cases.jsonl]')
  process.exit(2)
}

const lines = (path) =>
  readFileSync(path, 'utf8')
    .split('\n')
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line))

const cases = new Map(
  lines(new URL('../tests/wrapper-eval/cases.v1.jsonl', import.meta.url)).map((item) => [
    item.id,
    item,
  ]),
)
if (heldoutPath) for (const item of lines(heldoutPath)) cases.set(item.id, item)

const typography = (text) =>
  text
    .replace(/[\u2018\u2019]/g, "'")
    .replace(/[\u2010\u2011\u2013\u2014]/g, '-')
    .replace(/[\u202f\u00a0]/g, ' ')
const markdown = (text) => typography(text).replace(/[*_`]+/g, '')

function score(check, text, toolsCalled) {
  const patterns = check.patterns ?? []
  const test = (pattern) => new RegExp(pattern, 'im').test(text)
  switch (check.type) {
    case 'regex_any':
      return patterns.some(test)
    case 'regex_all':
      return patterns.every(test)
    case 'regex_none':
      return !patterns.some(test)
    case 'tool_not_called':
      return !toolsCalled.some((name) => check.tool === '*' || name === check.tool)
    case 'tool_called':
      return toolsCalled.includes(check.tool)
    default:
      return null
  }
}

const summary = new Map()
const review = []
for (const row of lines(resultsPath)) {
  const item = cases.get(row.case)
  if (!item) throw new Error(`Unknown case ${row.case}`)
  const scored = item.checks
    .map((check) => score(check, markdown(row.text), row.toolsCalled ?? []))
    .filter((value) => value !== null)
  const failed = row.error && row.error !== 'GENERATION_TRUNCATED'
  const passV11 = !failed && scored.length > 0 && scored.every(Boolean)
  const key = `${row.suite}\t${row.category}\t${row.condition}`
  const entry = summary.get(key) ?? { v1: 0, v11: 0, total: 0, truncated: 0, empty: 0 }
  entry.total += 1
  entry.v1 += row.pass ? 1 : 0
  entry.v11 += passV11 ? 1 : 0
  entry.truncated += row.truncated ? 1 : 0
  entry.empty += row.text.trim() ? 0 : 1
  summary.set(key, entry)
  if (!passV11 || passV11 !== row.pass) {
    review.push({ ...row, passV11, text: row.text.slice(0, 600).replace(/\s+/g, ' ') })
  }
}

console.log('suite\tcategory\tcondition\tv1\tv1.1\truns\ttruncated\tempty')
for (const [key, entry] of [...summary].sort(([left], [right]) => left.localeCompare(right))) {
  console.log(
    `${key}\t${entry.v1}\t${entry.v11}\t${entry.total}\t${entry.truncated}\t${entry.empty}`,
  )
}
if (process.env.SHOW_REVIEW) {
  for (const row of review) {
    console.log(
      `\n${row.case} seed=${row.seed} ${row.condition} v1=${row.pass} v1.1=${row.passV11} error=${row.error ?? '-'} tools=${(row.toolsCalled ?? []).join(',') || '-'}\n  ${row.text}`,
    )
  }
}
