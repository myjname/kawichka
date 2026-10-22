"""CPU-only llama smoke test using an already downloaded GGUF and CPU archive."""
import argparse
import json
import shutil
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.downloads import Components
from kavychka_local.llama import Llama
from kavychka_local.store import Store

parser = argparse.ArgumentParser()
parser.add_argument("--work", type=Path, required=True)
parser.add_argument("--archive", type=Path, required=True)
parser.add_argument("--model", type=Path, required=True)
args = parser.parse_args()
sys.stdout.reconfigure(encoding="utf-8")
if not args.model.is_file() or not args.archive.is_file():
    parser.error("Provide an existing GGUF and the downloaded CPU runtime archive")
root = args.work.resolve()
root.mkdir(exist_ok=False, parents=True)
(root / "downloads").mkdir()
store = Store(root)
model = None
try:
    store.configure({"device": "cpu"})
    components = Components(ROOT, root, store)
    spec = components.manifest["llama_cpu_runtime"]["files"][0]
    shutil.copyfile(args.archive, root / "downloads" / spec["path"])
    event = threading.Event()
    install = components.install("llama-runtime", event, lambda message, *a: print(message, flush=True))
    assert components.llama_device() == "cpu" and components.llama_ready()
    assert not components.llama_ready(gpu=True)
    assert not (root / "llama/ggml-cuda.dll").exists()
    assert not (root / "models").exists()
    stamp = components.llama_executable().stat().st_mtime_ns
    assert components.install("llama-runtime", event, lambda *a: None)["reused"]
    assert stamp == components.llama_executable().stat().st_mtime_ns
    started = time.monotonic()
    model = Llama(components.llama_executable(), args.model.resolve(), root, "cpu", event, lambda *a: None)
    loaded_seconds = time.monotonic() - started
    result = model.generate([{"role": "user", "content": "Say a short greeting in Russian. One sentence."}], event, max_tokens=32)
    assert result.strip()
    model.close(); model = None
    document = dict(install=install, device="cpu", no_cuda_libraries=True, no_downloaded_models=True,
                    reused_runtime=True, load_seconds=loaded_seconds, total_seconds=time.monotonic() - started, text=result)
    (root / "result.json").write_text(json.dumps(document, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(document, ensure_ascii=False), flush=True)
finally:
    if model is not None:
        model.close()
    store.db.close()
