# Juniper local runtime slot

Release and development bundles place the Juniper-owned `llama-server` executable
in this directory before Tauri packages the application. The executable is not
checked into git and is never downloaded at runtime.

Use `scripts/build-llama-runtime.sh` on a supported desktop build runner. It
checks out the pinned `llama.cpp` source revision, builds a CPU-safe server, and
writes only the resulting executable here. `JUNIPER_LLAMA_SERVER` remains
available as a developer override for local testing.

Models with a qualified backend profile (`config/backends/`) need the separately
pinned CUDA server. `JUNIPER_LLAMA_VARIANT=cuda scripts/build-llama-runtime.sh`
builds it from `desktopCuda` in `config/llama-cpp.json` and writes
`llama-server-cuda` here; `JUNIPER_LLAMA_SERVER_CUDA` is the developer
override. Juniper checks the build's reported commit before loading such a
model and never falls back to the CPU server for it. Release workflows do not
build the CUDA server yet.

`llama.cpp` and ggml are MIT-licensed (see `THIRD_PARTY_NOTICES.md`). Model
weights are separate user-owned files: downloaded only from the HTTPS URLs and
SHA-256 pins in `config/models/catalog.json`, or imported from a local file that
must match those pins.
