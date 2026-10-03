#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
runtime_dir="${JUNIPER_RUNTIME_DIR:-$repo_root/src-tauri/runtime}"
build_root="${JUNIPER_LLAMA_BUILD_DIR:-$(mktemp -d)}"
source_dir="$build_root/llama.cpp"
build_dir="$source_dir/build"
manifest="$repo_root/config/llama-cpp.json"

for tool in git cmake node; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "Missing required build tool: $tool" >&2
    exit 1
  }
done
# JUNIPER_LLAMA_VARIANT=cuda builds the separately pinned CUDA server that
# qualified backends (config/backends) require. It is never a substitute for
# the CPU server, and the CPU build never uses the CUDA pin.
variant="${JUNIPER_LLAMA_VARIANT:-cpu}"
manifest_value() {
  node -e "const fs=require('fs'); const v=process.argv[2].split('.').reduce((o,k)=>o&&o[k], JSON.parse(fs.readFileSync(process.argv[1], 'utf8'))); if (typeof v !== 'string') process.exit(1); process.stdout.write(v)" "$manifest" "$1"
}
case "$variant" in
  cpu)
    llama_commit="${JUNIPER_LLAMA_COMMIT:-$(manifest_value commit)}"
    executable=llama-server
    ;;
  cuda)
    command -v nvcc >/dev/null 2>&1 || {
      echo "The CUDA variant needs nvcc on PATH." >&2
      exit 1
    }
    llama_commit="$(manifest_value desktopCuda.commit)"
    cuda_architectures="$(manifest_value desktopCuda.cudaArchitectures)"
    executable="$(manifest_value desktopCuda.executable)"
    ;;
  *)
    echo "Unknown JUNIPER_LLAMA_VARIANT: $variant" >&2
    exit 1
    ;;
esac
[[ "$llama_commit" =~ ^[a-f0-9]{40}$ ]] || {
  echo "Invalid llama.cpp commit: $llama_commit" >&2
  exit 1
}

cleanup() {
  if [[ -z "${JUNIPER_LLAMA_BUILD_DIR:-}" ]]; then
    rm -rf "$build_root"
  fi
}
trap cleanup EXIT

if [[ -e "$source_dir" ]]; then
  git -C "$source_dir" fetch --depth 1 origin "$llama_commit"
else
  git clone --filter=blob:none --no-checkout https://github.com/ggml-org/llama.cpp.git "$source_dir"
fi
git -C "$source_dir" checkout --force "$llama_commit"

variant_flags=(-DGGML_NATIVE=OFF)
if [[ "$variant" == cuda ]]; then
  # Matches the qualified FLOWBOX build (Juniper LM 1.1, manifests/runtime):
  # CPU-side experts dominate decode speed, so the server is built for the
  # machine that runs it rather than for portability.
  variant_flags=(-DGGML_NATIVE=ON -DGGML_CUDA=ON "-DCMAKE_CUDA_ARCHITECTURES=$cuda_architectures")
fi

cmake -S "$source_dir" -B "$build_dir" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  "${variant_flags[@]}" \
  -DGGML_OPENMP=OFF \
  -DLLAMA_BUILD_COMMON=ON \
  -DLLAMA_BUILD_EXAMPLES=ON \
  -DLLAMA_BUILD_TOOLS=ON \
  -DLLAMA_BUILD_SERVER=ON \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_CURL=OFF \
  -DLLAMA_BUILD_UI=OFF \
  -DLLAMA_USE_PREBUILT_UI=OFF
# Keep hosted runners within their memory budget; override for a larger builder.
cmake --build "$build_dir" --config Release --target llama-server \
  --parallel "${JUNIPER_LLAMA_BUILD_PARALLEL:-2}"

server_path=$(find "$build_dir" -type f \( -name llama-server -o -name llama-server.exe \) -perm -u+x -print -quit)
if [[ -z "$server_path" ]]; then
  server_path=$(find "$build_dir" -type f \( -name llama-server -o -name llama-server.exe \) -print -quit)
fi
test -n "$server_path"
mkdir -p "$runtime_dir"
case "$server_path" in
  *.exe) cp "$server_path" "$runtime_dir/$executable.exe" ;;
  *) cp "$server_path" "$runtime_dir/$executable"; chmod 0755 "$runtime_dir/$executable" ;;
esac
printf 'Built %s (%s) from %s\n' "$executable" "$variant" "$llama_commit"
