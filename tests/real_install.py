"""Fresh, isolated embedded-Python + CPU runtime installation (no model weights)."""
import argparse
import json
import os
import shutil
import subprocess
import sys
import threading
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.downloads import Components, digest
from kavychka_local.store import Store

parser = argparse.ArgumentParser()
parser.add_argument("--work", type=Path, required=True)
parser.add_argument("--python-archive", type=Path)
parser.add_argument("--inside", action="store_true")
parser.add_argument("--resume", action="store_true")
parser.add_argument("--cosy", action="store_true")
args = parser.parse_args()
sys.stdout.reconfigure(encoding="utf-8")
root = args.work.resolve()
if not args.inside:
    root.mkdir(parents=True, exist_ok=args.resume)
    manifest = json.loads((ROOT / "assets/components.json").read_text(encoding="utf-8"))
    archive = args.python_archive
    if archive is None or archive.stat().st_size != manifest["python"]["size"] or digest(archive) != manifest["python"]["sha256"]:
        parser.error("Provide the verified embedded Python archive")
    runtime = root / "runtime"
    runtime.mkdir(exist_ok=args.resume)
    with zipfile.ZipFile(archive) as source:
        source.extractall(runtime)
    (runtime / "Lib/site-packages").mkdir(parents=True, exist_ok=True)
    (runtime / "python312._pth").write_text("python312.zip\n.\nLib/site-packages\nimport site\n", encoding="utf-8")
    (root / "downloads").mkdir(exist_ok=True)
    cached_pip = ROOT / "work/downloads/get-pip.py"
    if cached_pip.is_file() and digest(cached_pip) == manifest["pip"]["sha256"]:
        shutil.copyfile(cached_pip, root / "downloads/get-pip.py")
    temporary = root / "tmp"; temporary.mkdir(exist_ok=True)
    env = dict(os.environ, TMP=str(temporary), TEMP=str(temporary), PYTHONDONTWRITEBYTECODE="1",
               PYTHONIOENCODING="utf-8", CUDA_VISIBLE_DEVICES="", HF_HOME=str(root / "hf"),
               TORCH_HOME=str(root / "torch"), NUMBA_CACHE_DIR=str(root / "numba"), PIP_CACHE_DIR=str(root / "pip-cache"))
    command = [str(runtime / "python.exe"), "-B", str(Path(__file__).resolve()), "--work", str(root), "--inside"]
    if args.cosy:
        command.append("--cosy")
    raise SystemExit(subprocess.call(command, env=env))

store = Store(root)
try:
    store.configure({"device": "cpu"})
    components = Components(ROOT, root, store)
    started = time.monotonic()
    last = [0]
    def progress(message, done=0, total=0):
        if total and time.monotonic() - last[0] < 2:
            return
        last[0] = time.monotonic()
        print(message, flush=True)
    components.install("runtime", threading.Event(), progress, gpu=False)
    if args.cosy:
        components.install("cosy-runtime", threading.Event(), progress, gpu=False)
    check = subprocess.run([sys.executable, "-B", "-m", "pip", "check"], capture_output=True, text=True, encoding="utf-8")
    if check.returncode:
        raise RuntimeError(check.stdout + check.stderr)
    marker = json.loads((root / "runtime-ok.json").read_text(encoding="utf-8"))
    assert marker["gpu"] is False and "+cpu" in marker["torch"]
    assert all(item["installed"] for item in components.status() if item["id"] in ("runtime", "cosy-runtime") and (args.cosy or item["id"] == "runtime"))
    assert not (root / "models").exists()
    assert not any("cu128" in item.name for item in (root / "downloads").iterdir())
    document = dict(passed=True, embedded_python=sys.version, runtime=marker, cosy=args.cosy,
                    pip_check=check.stdout.strip(), no_model_weights=True, elapsed_seconds=time.monotonic() - started)
    (root / "result.json").write_text(json.dumps(document, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(document, ensure_ascii=False), flush=True)
finally:
    store.db.close()
