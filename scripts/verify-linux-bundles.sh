#!/usr/bin/env bash
# Verifies Juniper's Linux release artifacts as a user would run them.
#
#   scripts/verify-linux-bundles.sh <artifact-dir> <version> [environment-label]
#
# Must run inside an X11 display (the release workflow uses xvfb-run). Each
# launch goes through scripts/linux-launch-probe.sh, which requires positive
# frontend readiness, a rendered window, and a clean exit; surviving a timeout
# is never success. Launch modes are distinct and all required:
#   appimage          ./Juniper-<version>-linux-x86_64.AppImage (normal FUSE path)
#   appimage-extract  APPIMAGE_EXTRACT_AND_RUN=1 fallback for hosts without FUSE
#   deb               installed package, launched through its desktop entry
# JUNIPER_LINUX_SMOKE_MODES may narrow the list for local use (for example on a
# machine without sudo); excluded modes are reported as NOT VERIFIED and the
# release workflow never sets it.
# Each mode starts from empty XDG data/config/cache directories, then relaunches
# with stored state that enables an Ollama provider backed by a loopback
# stand-in, the exact state that left rc.31 with a blank window.
#
# Writes PACKAGE-linux.txt next to the artifacts and evidence under
# <artifact-dir>/linux-smoke-<environment-label>/.
set -euo pipefail

artifact_dir=$(realpath "${1:?artifact directory is required}")
version=${2:?version is required}
environment_label=${3:-$(. /etc/os-release && printf '%s-%s' "$ID" "$VERSION_ID")}
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
deb="$artifact_dir/Juniper-$version-linux-x86_64.deb"
appimage="$artifact_dir/Juniper-$version-linux-x86_64.AppImage"
evidence="$artifact_dir/linux-smoke-$environment_label"
probe="$repo_root/scripts/linux-launch-probe.sh"
regression="$repo_root/tests/linux-probe/ollama-startup-regression.py"
runtime_relative='usr/lib/Juniper/runtime/llama-server'
work=$(mktemp -d)
fake_ollama_pid=''

cleanup() {
  [[ -n $fake_ollama_pid ]] && kill "$fake_ollama_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }
die() { printf 'Linux bundle verification failed: %s\n' "$*" >&2; exit 1; }

rm -rf "$evidence"
mkdir -p "$evidence"
{
  echo "environment=$environment_label"
  uname -srm
  grep -E '^(PRETTY_NAME|VERSION_ID)=' /etc/os-release
  echo "display=${DISPLAY:-unset} wayland=${WAYLAND_DISPLAY:-unset}"
  dpkg-query --showformat='${Package} ${Version}\n' --show \
    libwebkit2gtk-4.1-0 libgtk-3-0 libgtk-3-0t64 libfuse2 libfuse2t64 2>/dev/null || true
} > "$evidence/environment.txt"
cat "$evidence/environment.txt"

step 'Static artifact checks'
[[ -f $deb ]] || die "missing $deb"
[[ -f $appimage ]] || die "missing $appimage"
file "$deb" "$appimage" | tee "$evidence/file.txt"
file -b "$appimage" | grep -Eq 'ELF 64-bit LSB (pie )?executable, x86-64' || die 'AppImage is not an x86-64 ELF executable'
dpkg-deb --info "$deb" | tee "$artifact_dir/PACKAGE-linux.txt"
[[ $(dpkg-deb --field "$deb" Package) == juniper ]] || die 'DEB package name is not juniper'
[[ $(dpkg-deb --field "$deb" Version) == "$version" ]] || die "DEB version is not $version"
[[ $(dpkg-deb --field "$deb" Architecture) == amd64 ]] || die 'DEB architecture is not amd64'
# Materialize the listing before matching: with pipefail, grep -q can close
# early and make dpkg-deb report SIGPIPE even when the match is valid.
dpkg-deb --contents "$deb" > "$evidence/deb-contents.txt"
require_entry() {
  grep -Eq "$1" "$evidence/deb-contents.txt" || die "DEB is missing $2"
}
require_entry '^-rwxr-xr-x .* (\./)?usr/bin/juniper$' 'an executable /usr/bin/juniper'
require_entry "^-rwxr-xr-x .* (\\./)?$runtime_relative\$" "an executable /$runtime_relative"
require_entry ' (\./)?usr/lib/Juniper/LICENSE$' 'LICENSE'
require_entry ' (\./)?usr/lib/Juniper/THIRD_PARTY_NOTICES\.md$' 'THIRD_PARTY_NOTICES.md'
require_entry ' (\./)?usr/share/applications/Juniper\.desktop$' 'its desktop entry'

step 'Inspect packaged trees'
# Downloads (and GitHub workflow artifacts) drop the executable bit; users are
# told to `chmod +x` first, so the smoke does the same before any execution.
chmod +x "$appimage"
appimage_abs=$(realpath "$appimage")
dpkg-deb -x "$deb" "$work/deb-root"
(cd "$work" && "$appimage_abs" --appimage-extract > /dev/null)
mv "$work/squashfs-root" "$work/appimage-root"
for tree in deb-root appimage-root; do
  root="$work/$tree"
  file -b "$root/usr/bin/juniper" | grep -q 'ELF 64-bit LSB .*x86-64' || die "$tree juniper is not x86-64"
  [[ -x $root/$runtime_relative ]] || die "$tree $runtime_relative is not executable"
  file -b "$root/$runtime_relative" | grep -q 'ELF 64-bit LSB .*x86-64' || die "$tree llama-server is not x86-64"
  timeout 30s "$root/$runtime_relative" --version > "$evidence/$tree-llama-server-version.txt" 2>&1 ||
    die "$tree bundled llama-server does not execute: $(tail -n 5 "$evidence/$tree-llama-server-version.txt")"
  [[ -f $root/usr/lib/Juniper/LICENSE && -f $root/usr/lib/Juniper/THIRD_PARTY_NOTICES.md ]] ||
    die "$tree is missing LICENSE or THIRD_PARTY_NOTICES.md"
  desktop="$root/usr/share/applications/Juniper.desktop"
  desktop-file-validate "$desktop" || die "$tree desktop entry is invalid"
  grep -Fxq 'Exec=juniper' "$desktop" || die "$tree desktop entry does not launch juniper"
  grep -Fxq 'Icon=juniper' "$desktop" || die "$tree desktop entry does not use the juniper icon"
  grep -Fxq 'Name=Juniper' "$desktop" || die "$tree desktop entry is not named Juniper"
done
cmp -s "$work/appimage-root/Juniper.desktop" "$work/appimage-root/usr/share/applications/Juniper.desktop" ||
  die 'AppImage top-level desktop entry differs from the packaged one'
node "$repo_root/scripts/verify-desktop-branding.mjs" \
  "$work/deb-root/usr/share/icons" \
  "$work/appimage-root/usr/share/icons" \
  "$work/appimage-root/.DirIcon" \
  "$work/appimage-root/juniper.png" | tee "$evidence/branding.txt"

xdg_home() {
  local base="$work/xdg-$1"
  rm -rf "$base"
  mkdir -p "$base/data" "$base/config" "$base/cache"
  printf '%s\n' "$base"
}

modes=${JUNIPER_LINUX_SMOKE_MODES:-appimage,appimage-extract,deb}
mode_enabled() { [[ ",$modes," == *",$1,"* ]]; }

# Launch twice from one isolated profile: first from empty state, then with
# stored state that enables Ollama discovery against a loopback stand-in.
launch_mode() {
  local mode=$1 runtime_pattern=$2
  shift 2
  local base
  base=$(xdg_home "$mode")
  local -a env_args=(env "XDG_DATA_HOME=$base/data" "XDG_CONFIG_HOME=$base/config" "XDG_CACHE_HOME=$base/cache")

  step "Launch $mode (fresh profile)"
  "$probe" "$mode-fresh" "$evidence" -- "${env_args[@]}" "$@" || die "$mode fresh launch failed readiness"
  grep -Fq "[juniper-startup] Juniper $version starting on linux/x86_64" "$evidence/$mode-fresh.log" ||
    die "$mode did not report Juniper $version"
  grep -Eq "^\\[juniper-startup\\] local runtime: $runtime_pattern\$" "$evidence/$mode-fresh.log" ||
    die "$mode did not resolve its bundled runtime matching $runtime_pattern"
  local database="$base/data/com.cinqic.juniper/juniper.db"
  [[ -f $database ]] || die "$mode did not create $database"

  step "Launch $mode (stored Ollama provider with installed models)"
  rm -f "$work/ollama-port" "$work/ollama-requests.log"
  python3 "$regression" serve "$work/ollama-port" "$work/ollama-requests.log" &
  fake_ollama_pid=$!
  for _ in $(seq 1 50); do [[ -s $work/ollama-port ]] && break; sleep 0.1; done
  [[ -s $work/ollama-port ]] || die 'Ollama stand-in did not start'
  python3 "$regression" seed "$database" "$(cat "$work/ollama-port")"
  "$probe" "$mode-ollama" "$evidence" -- "${env_args[@]}" "$@" || die "$mode launch with stored Ollama state failed readiness"
  kill "$fake_ollama_pid" 2>/dev/null || true
  wait "$fake_ollama_pid" 2>/dev/null || true
  fake_ollama_pid=''
  grep -Fxq 'GET /api/tags' "$work/ollama-requests.log" || die "$mode never queried the Ollama stand-in"
  python3 "$regression" check "$database" | tee -a "$evidence/$mode-ollama-result.txt"
}

if mode_enabled appimage; then
  step 'Normal AppImage execution (FUSE)'
  [[ -c /dev/fuse ]] || die '/dev/fuse is unavailable; the normal AppImage path cannot be exercised here'
  launch_mode appimage "/.*/\\.mount_[^/]+/$runtime_relative" "$appimage_abs"
  grep -Fq 'appimage=true' "$evidence/appimage-fresh.log" || die 'AppImage launch did not run inside the AppImage runtime'
fi

if mode_enabled appimage-extract; then
  step 'AppImage extract-and-run fallback'
  # Stopping Juniper with SIGTERM prevents the AppImage runtime from removing
  # its extraction directory; extract to a private TMPDIR so it is cleaned up.
  mkdir -p "$work/extract-tmp"
  launch_mode appimage-extract "$work/extract-tmp/appimage_extracted_[^/]+/$runtime_relative" \
    env "TMPDIR=$work/extract-tmp" APPIMAGE_EXTRACT_AND_RUN=1 "$appimage_abs"

  step 'Exercise native chat attachment dialog and cancellation'
  picker_home=$(xdg_home picker)
  python3 "$repo_root/scripts/linux-picker-probe.py" "$evidence/picker" -- \
    env "XDG_DATA_HOME=$picker_home/data" "XDG_CONFIG_HOME=$picker_home/config" \
    "XDG_CACHE_HOME=$picker_home/cache" "TMPDIR=$work/extract-tmp" \
    APPIMAGE_EXTRACT_AND_RUN=1 "$appimage_abs"
fi

if mode_enabled deb; then
  step 'Install DEB'
  sudo apt-get install -y "$(realpath "$deb")" | tee "$evidence/deb-install.txt"
  dpkg-query --show juniper | grep -Fxq "juniper	$version" || die "installed juniper is not $version"
  dpkg -L juniper > "$evidence/deb-installed-files.txt"
  [[ $(command -v juniper) == /usr/bin/juniper ]] || die 'juniper is not installed at /usr/bin/juniper'
  [[ -x /$runtime_relative ]] || die "/$runtime_relative is not executable after install"
  desktop-file-validate /usr/share/applications/Juniper.desktop
  # Launch exactly what the desktop entry runs (it has no field codes).
  exec_line=$(sed -n 's/^Exec=//p' /usr/share/applications/Juniper.desktop)
  [[ $exec_line == juniper ]] || die "unexpected desktop Exec line: $exec_line"
  launch_mode deb "/$runtime_relative" "$(command -v "$exec_line")"
  user_data="$work/xdg-deb/data/com.cinqic.juniper/juniper.db"

  step 'Purge DEB'
  sudo apt-get purge -y juniper | tee "$evidence/deb-purge.txt"
  if command -v juniper >/dev/null; then die 'juniper executable remained after purge'; fi
  while IFS= read -r path; do
    if [[ -f $path ]]; then die "package-owned file remained after purge: $path"; fi
  done < "$evidence/deb-installed-files.txt"
  [[ -f $user_data ]] || die 'purging the package removed user application data'
  echo "User data preserved after purge: $user_data"
fi

for mode in appimage appimage-extract deb; do
  mode_enabled "$mode" || echo "NOT VERIFIED: launch mode $mode was explicitly excluded" | tee -a "$evidence/SUMMARY.txt"
done

printf '\nLinux bundle verification passed on %s for launch modes: %s\n' "$environment_label" "$modes" |
  tee -a "$evidence/SUMMARY.txt"
