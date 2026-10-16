#!/usr/bin/env bash
# Exact-source live transfer check: a resumed single-file download above
# 2 GiB and a sharded bundle, through Juniper's own transfer code.
#
#   scripts/live-transfer-check.sh <work-dir> [artifact-ids] [resume-artifact-id]
#
# Refuses a dirty tree, stamps the commit into the test build, and refuses to
# start unless the largest download (4.7 GB) still leaves JUNIPER_MIN_FREE_MB
# (default 1500) free on the work folder's disk. Writes
# <work-dir>/evidence.jsonl; downloaded files are deleted after each
# artifact. Needs network access and a llama-server build
# (JUNIPER_LLAMA_SERVER, default /usr/lib/Juniper/runtime/llama-server).
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=${1:?A work folder on a disk with space is required}
artifacts=${2:-qwen3-4b-q4-k-m,qwen2.5-7b-instruct-q4-k-m}
resume=${3:-qwen3-4b-q4-k-m}
min_free_mb=${JUNIPER_MIN_FREE_MB:-1500}
largest_mb=4700

if [[ -n $(git -C "$repo_root" status --porcelain) ]]; then
  echo "The source tree has uncommitted changes; commit them first." >&2
  exit 1
fi
commit=$(git -C "$repo_root" rev-parse HEAD)
mkdir -p "$work/models"

export JUNIPER_LIVE_SOURCE_COMMIT=$commit
binary=$(cargo test --locked --manifest-path "$repo_root/src-tauri/Cargo.toml" --lib --no-run \
  --message-format=json 2>/dev/null |
  node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{for(const l of s.split("\n")){try{const m=JSON.parse(l);if(m.profile?.test&&m.target?.kind?.includes("lib")&&m.executable)console.log(m.executable)}catch{}}})' |
  tail -1)
[[ -x $binary ]] || {
  echo "The test build did not produce a binary." >&2
  exit 1
}
free_mb=$(df --output=avail -BM "$work" | tail -1 | tr -dc '0-9')
if ((free_mb < min_free_mb + largest_mb)); then
  echo "Only ${free_mb} MB free under $work; need $((min_free_mb + largest_mb)) MB." >&2
  exit 1
fi
echo "source $commit"
echo "binary $binary"

JUNIPER_LLAMA_SERVER=${JUNIPER_LLAMA_SERVER:-/usr/lib/Juniper/runtime/llama-server} \
  JUNIPER_LIVE_MODELS_DIR="$work/models" \
  JUNIPER_LIVE_EVIDENCE="$work/evidence.jsonl" \
  JUNIPER_LIVE_ARTIFACTS=$artifacts \
  JUNIPER_LIVE_RESUME=$resume \
  "$binary" --ignored --nocapture --exact live_catalog::live_catalog
