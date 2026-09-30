#!/usr/bin/env bash
# Self-test for the release workflow's publication completeness guard.
#
# 0.3.0-rc.33 was published without its MSI, its DEB, and
# RELEASE-PROVENANCE.json: the publication action created the release and then
# failed part way through its uploads. The job failed, but the release stayed
# public and incomplete. release.yml now re-uploads whatever is missing and
# fails when the published release still lacks an artifact.
#
# This runs that step's script verbatim against a stub `gh`, so the guard is
# exercised rather than assumed:
#   complete - every artifact already published        -> pass, no upload
#   partial  - three assets missing, upload works      -> pass after re-upload
#   broken   - uploads never take effect               -> fail
#
#   scripts/test-release-publication-guard.sh [workflow]
set -uo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workflow=${1:-$repo_root/.github/workflows/release.yml}
step='- name: Require every verified artifact to be published'
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

grep -Fq -e "$step" "$workflow" || {
  echo "FAIL: $workflow no longer contains the publication guard step" >&2
  exit 1
}

# Extract the guard's shell script exactly as the workflow runs it.
python3 - "$workflow" "$work/guard.sh" "$step" <<'PY'
import sys
workflow, target, step = sys.argv[1], sys.argv[2], sys.argv[3]
block = open(workflow).read().split(step, 1)[1].split('run: |', 1)[1]
lines = []
for line in block.split('\n')[1:]:
    if line.strip() and not line.startswith('          '):
        break
    lines.append(line[10:])
open(target, 'w').write('\n'.join(lines))
PY

run_case() {
  local name=$1 mode=$2 expected=$3
  local dir="$work/$name"
  mkdir -p "$dir/release-artifacts" "$dir/bin" "$dir/state"
  for file in Juniper-1.0-windows-x86_64.msi Juniper-1.0-linux-x86_64.deb \
              Juniper-1.0-linux-x86_64.AppImage SHA256SUMS.txt \
              RELEASE-PROVENANCE.json LICENSE; do
    echo data > "$dir/release-artifacts/$file"
  done
  # Evidence directories travel with the bundle and are never release assets.
  mkdir -p "$dir/release-artifacts/linux-smoke"
  echo log > "$dir/release-artifacts/linux-smoke/probe.log"

  if [[ $mode == complete ]]; then
    (cd "$dir/release-artifacts" && find . -maxdepth 1 -type f -printf '%f\n') > "$dir/state/published"
  else
    printf 'Juniper-1.0-linux-x86_64.AppImage\nSHA256SUMS.txt\nLICENSE\n' > "$dir/state/published"
  fi

  cat > "$dir/bin/gh" <<EOF
#!/usr/bin/env bash
# Stub: \`gh release view\` lists published assets, \`gh release upload\` adds them.
if [[ \$2 == view ]]; then cat "$dir/state/published"; exit 0; fi
if [[ \$2 == upload ]]; then
  if [[ $mode != broken ]]; then
    for path in "\${@:5}"; do basename "\$path" >> "$dir/state/published"; done
  fi
  exit 0
fi
exit 0
EOF
  chmod +x "$dir/bin/gh"

  (cd "$dir" && PATH="$dir/bin:$PATH" GITHUB_REPOSITORY=Cinqic-Research/Juniper-App TAG=v1.0 \
    bash "$work/guard.sh") > "$dir/out.txt" 2>&1
  local code=$?
  if [[ $code -eq $expected ]]; then
    printf 'PASS  %-9s exit %d: %s\n' "$name" "$code" "$(tail -n 1 "$dir/out.txt")"
  else
    printf 'FAIL  %-9s exit %d, expected %d\n' "$name" "$code" "$expected"
    sed 's/^/      /' "$dir/out.txt"
    failures=$((failures + 1))
  fi
}

run_case complete complete 0
run_case partial partial 0
run_case broken broken 1

if ((failures)); then
  echo "Publication guard self-test failed: $failures case(s)." >&2
  exit 1
fi
echo 'Publication guard self-test passed: complete, partial, and broken publications behave as required.'
