"""Greedy reference generations from the Python token-ID client, for the App's live parity check.

Starts ``llama-server`` with the same flags Juniper App uses for Juniper 1 (CPU only, context 2048,
one slot), drives it only through Juniper-1-Data's ``j1data.runtime_client.TokenIdClient``, and
writes, per GGUF, the greedy 24-token continuation of the 48 evaluation-bundle prompts that
``qual/a02_token_id_qual.py`` (Juniper-1-Train) uses: ``split == "dev"``, ``variant == "chat"``,
every 7th, first 48. The prompts come from a private bundle, so write ``--out`` outside this
repository; ``live_juniper1`` reads it through ``JUNIPER1_PARITY_FIXTURE``.

    PYTHONPATH=<Juniper-1-Data>/src python tests/juniper1/python_parity.py \\
        --server <llama.cpp b11270 llama-server> --tokenizer <tokenizer.json> \\
        --bundle <sft eval bundle dir> --gguf <base.gguf> --gguf <candidate.gguf> --out <path.json>
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def serve(server: Path, gguf: Path) -> tuple[subprocess.Popen, int, str]:
    port, key = free_port(), secrets.token_hex(16)
    keyfile = tempfile.NamedTemporaryFile("w", delete=False, prefix="j1key-")  # noqa: SIM115
    Path(keyfile.name).chmod(0o600)
    keyfile.write(key)
    keyfile.close()
    cmd = [str(server), "--host", "127.0.0.1", "--port", str(port), "--model", str(gguf),
           "--api-key-file", keyfile.name, "--no-webui", "--no-slots", "--offline", "--no-mmproj",
           "--no-context-shift", "--fit", "off", "-c", "2048", "-np", "1", "-ngl", "0", "-b", "512",
           "--cache-ram", "256"]  # fmt: skip
    env = {**os.environ, "CUDA_VISIBLE_DEVICES": ""}
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env, start_new_session=True)
    deadline = time.time() + 120
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2) as r:
                if r.status == 200:
                    break
        except OSError:
            time.sleep(0.25)
    else:
        os.killpg(proc.pid, signal.SIGKILL)
        raise SystemExit("llama-server did not become healthy")
    Path(keyfile.name).unlink()
    return proc, port, key


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", type=Path, required=True)
    ap.add_argument("--tokenizer", type=Path, required=True)
    ap.add_argument("--bundle", type=Path, required=True)
    ap.add_argument("--gguf", type=Path, action="append", required=True)
    ap.add_argument("--prompts", type=int, default=48)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args()
    from j1data import jtok
    from j1data.runtime_client import Sampling, TokenIdClient

    tok = jtok.load(a.tokenizer.read_text())
    rows = [json.loads(x) for x in (a.bundle / "prompts.jsonl").read_bytes().split(b"\n") if x]
    sel = [p for p in rows if p["split"] == "dev" and p["variant"] == "chat"][::7][: a.prompts]
    manifest = json.loads((a.bundle / "manifest.json").read_text())
    out = {
        "schema": "juniper-app.juniper1-python-parity.v1",
        "bundle_sha256": manifest["bundle_sha256"],
        "selection": "split == dev, variant == chat, every 7th, first 48 (qual/a02_token_id_qual.py)",
        "sampling": "greedy (j1data.runtime_client.Sampling())",
        "max_new_tokens": 24,
        "models": [],
    }
    for gguf in a.gguf:
        proc, port, key = serve(a.server, gguf)
        try:
            client = TokenIdClient(f"http://127.0.0.1:{port}", tok, api_key=key)
            props_req = urllib.request.Request(
                f"http://127.0.0.1:{port}/props", headers={"Authorization": f"Bearer {key}"}
            )
            with urllib.request.urlopen(props_req, timeout=10) as r:
                build = json.load(r).get("build_info")
            self_test = client.self_test()
            gens = [{"id": p["id"], "prompt_ids": p["prompt_ids"],
                     "ids": client.complete(p["prompt_ids"], 24, Sampling()).ids} for p in sel]  # fmt: skip
            out["models"].append({
                "gguf_sha256": sha256_file(gguf), "runtime_build": build, "self_test": self_test,
                "paths_called": sorted(set(client.paths_called)), "generations": gens,
            })  # fmt: skip
        finally:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait(20)
    a.out.write_text(json.dumps(out, indent=1))
    print(f"wrote {a.out}: {[len(m['generations']) for m in out['models']]} generations")
    return 0


if __name__ == "__main__":
    sys.exit(main())
