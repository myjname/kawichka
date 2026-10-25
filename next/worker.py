"""v1 stateless inference protocol. Never opens SQLite or installs components.

The parent owns the process tree, settings, queue, acceptance and file lifetime.
Stdout is JSON lines only; diagnostic/model output goes to stderr.
"""
import contextlib
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import secrets
import sys
import threading
import uuid

def normal_path(value):
    # Third-party libraries concatenate '/' onto paths. Extended Win32 paths
    # disable slash normalization, so keep them out of the inference boundary.
    value = str(value)
    if value.startswith("\\\\?\\UNC\\"):
        value = "\\\\" + value[8:]
    elif value.startswith("\\\\?\\"):
        value = value[4:]
    return Path(value)

APP = normal_path(__file__).resolve().parent.parent
sys.path.insert(0, str(APP / "backend"))
os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1", HF_HUB_DISABLE_TELEMETRY="1",
                  PYTHONDONTWRITEBYTECODE="1", TOKENIZERS_PARALLELISM="false")
for key in ("HF_TOKEN", "HUGGING_FACE_HUB_TOKEN", "OPENAI_API_KEY"):
    os.environ.pop(key, None)

def emit(kind, **data):
    print(json.dumps(dict(version=1, task=TASK, kind=kind, **data), ensure_ascii=False), flush=True)

class TaskCancellation(threading.Event):
    """Only the owning parent can place this flag inside the private job."""
    def __init__(self, root):
        super().__init__()
        self.flag = root / "cancel.request"
    def is_set(self):
        return super().is_set() or self.flag.exists()

def stage(name, event):
    from kavychka_local.downloads import checkpoint
    checkpoint(event)
    emit("stage", stage=name)

def verify_managed_tree(root):
    """Installation must not follow user-added links out of managed storage."""
    count = 0
    def visit(path, depth):
        nonlocal count
        count += 1
        if count > 100000 or depth > 64:
            raise ValueError("Managed environment tree exceeds safe inspection limits")
        if not path.exists() and not path.is_symlink():
            return
        info = path.lstat()
        if path.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError("Managed environment contains a link; external target retained")
        if path.is_dir():
            for item in path.iterdir(): visit(item, depth + 1)
    visit(root, 0)

class RecentDB:
    def __init__(self, store): self.store = store
    def execute(self, query):
        if "SELECT voice_style FROM phrases" not in query:
            raise ValueError("Worker may only query its supplied delivery history")
        return [(style,) for style in self.store.styles[:6]]

class Snapshot:
    def __init__(self, request, root):
        from kavychka_local.store import DEFAULTS
        self.root, self.values = root, dict(DEFAULTS, **request["settings"])
        self.values["model_paths"] = {key: str(normal_path(path)) for key, path in self.values["model_paths"].items()}
        self.history, self.styles = list(request.get("history", [])), list(request.get("styles", []))
        from kavychka_local.diversity import normalize
        self.known_texts = {normalize(text) for text in request.get("known_texts", self.history)}
        self.rows, self.rotation = {}, request.get("rotation", 0)
        self.lock, self.db = threading.RLock(), RecentDB(self)
    def settings(self): return dict(self.values)
    def recent(self, limit=30): return self.history[:limit]
    def known(self, text):
        from kavychka_local.diversity import normalize, too_similar, same_opening
        return normalize(text) in self.known_texts or any(normalize(text) == normalize(old) or too_similar(text, old) or
                   (i < 8 and same_opening(text, old)) for i, old in enumerate(self.history[:500]))
    def assign_reference(self, ident, references):
        row = self.rows[ident]
        if row.get("voice_reference") not in references:
            row["voice_reference"] = references[self.rotation % len(references)]
            self.rotation += 1
        return row["voice_reference"]
    def audio(self, ident, model, key, filename, seed=42, delivery=None):
        row = self.rows[ident]
        row.update(model=model, audio_path=filename, seed=seed)
        if delivery: row.update(delivery)
        elif model != "tts-cosyvoice3": row.update(voice_style="", voice_instruction="")
        return dict(row)

def run(request):
    from kavychka_local.downloads import ReadOnlyComponents, Components
    from kavychka_local.engine import Engine
    from kavychka_local.profiles import Profiles
    root, library, components = (normal_path(request[key]).resolve() for key in ("work_dir", "library_root", "components_root"))
    if root.parent != (library / "jobs").resolve() or not root.is_dir():
        raise ValueError("Worker staging must be an existing private job directory")
    if request.get("payload_file"):
        if request["payload_file"] != "payload.json": raise ValueError("Invalid task payload path")
        payload = root / "payload.json"
        if payload.stat().st_size > 64_000_000: raise ValueError("Task payload exceeds 64 MB")
        values = json.loads(payload.read_text(encoding="utf-8"))
        if not isinstance(values, dict) or set(values) - {"rows", "known_texts"}: raise ValueError("Invalid task payload")
        request.update(values)
    (root / "audio").mkdir(exist_ok=True)
    os.environ.update(HF_HOME=str(root / "hf"), TORCH_HOME=str(root / "torch"),
                      TEMP=str(root), TMP=str(root), NUMBA_CACHE_DIR=str(root / "numba"), TORCH_EXTENSIONS_DIR=str(library / "cache/torch_extensions"), TRITON_CACHE_DIR=str(library / "cache/triton"), TORCHINDUCTOR_CACHE_DIR=str(library / "cache/inductor"))
    cuda_cache=library / "cache/cuda"
    cuda_cache.mkdir(parents=True,exist_ok=True)
    os.environ["CUDA_CACHE_PATH"]=str(cuda_cache)
    store = Snapshot(request, root)
    operation = request["operation"]
    if operation == "preset_import":
        source = normal_path(request["input"])
        if not source.is_file() or source.stat().st_size > 200 * 1024 * 1024:
            raise ValueError("Choose audio up to 200 MiB, 1-30 seconds of clean speech")
        result = Profiles(APP, root).import_file("voice", source, transcript=request.get("transcript", ""))
        emit("result", preset=result)
        return
    managed = library / "components"
    managed.mkdir(exist_ok=True)
    class Hybrid(ReadOnlyComponents):
        def path(self, key):
            original = super().path(key)
            own = managed / "models" / key
            spec = self.manifest["models"][key]
            return own if all((own / f["path"]).is_file() and (own / f["path"]).stat().st_size == f["size"] for f in spec["files"]) else original
        def llama_executable(self):
            return managed / "llama/llama-server.exe" if (managed / "llama/verified.json").is_file() else super().llama_executable()
        def llama_ready(self, gpu=None):
            return Components(APP, managed, store).llama_ready(gpu) or super().llama_ready(gpu)
        def cosy_ready(self):
            return Components(APP, managed, store).cosy_ready() or super().cosy_ready()
        def cosy_root(self):
            return managed if Components(APP, managed, store).cosy_ready() else self.root
    catalog = Hybrid(APP, components, store)
    if operation == "install":
        verify_managed_tree(managed)
        os.environ.update(PIP_CONFIG_FILE=os.devnull, PIP_INDEX_URL="https://pypi.org/simple",
                          PIP_EXTRA_INDEX_URL="", PIP_NO_INPUT="1", PIP_DISABLE_PIP_VERSION_CHECK="1")
        ids = request["ids"]
        if not isinstance(ids, list) or not 1 <= len(ids) <= 2 or any(i not in catalog.manifest["models"] for i in ids):
            raise ValueError("Select one text and one voice model")
        # Installation writes only the new managed component root, never the legacy source.
        settings = dict(store.values, model_paths={})
        class InstallSettings:
            def settings(self): return settings
        repairing = request.get("repair_environment") is True
        class ManagedInstaller(Components):
            def install_llama(self, *args, **kwargs):
                if repairing: (self.root / "llama/verified.json").unlink(missing_ok=True)
                return super().install_llama(*args, **kwargs)
            def install_cosy(self, *args, **kwargs):
                if repairing: (self.root / "cosy/verified.json").unlink(missing_ok=True)
                return super().install_cosy(*args, **kwargs)
            def run_install(self, command, *args, **kwargs):
                if repairing and "pip" in command and "install" in command and "--force-reinstall" not in command:
                    command = list(command) + ["--force-reinstall"]
                return super().run_install(command, *args, **kwargs)
        installer = ManagedInstaller(APP, managed, InstallSettings())
        event = TaskCancellation(root)
        def progress(message, done=0, total=0): emit("progress", message=message, done=done, total=total)
        import shutil
        def verified_model(ident):
            from kavychka_local.downloads import checkpoint
            folder = catalog.path(ident)
            for spec in catalog.manifest["models"][ident]["files"]:
                checkpoint(event)
                path = folder / spec["path"]
                if not path.is_file() or path.stat().st_size != spec["size"]:
                    return False
                progress("Проверка SHA-256 / Verifying: " + ident)
                value = hashlib.sha256()
                with path.open("rb") as file:
                    for block in iter(lambda: file.read(1024 * 1024), b""):
                        checkpoint(event)
                        value.update(block)
                if value.hexdigest() != spec["sha256"]:
                    return False
            return True
        missing = [i for i in ids if not verified_model(i)]
        required = sum(f["size"] for i in missing for f in catalog.manifest["models"][i]["files"])
        formats = {catalog.manifest["models"][i].get("format") for i in ids}
        gpu = installer.use_gpu("runtime")
        managed_python = normal_path(sys.executable).resolve().is_relative_to((managed / "runtime").resolve())
        runtime_marker = (managed if managed_python else components) / "runtime-ok.json"
        try:
            marker = json.loads(runtime_marker.read_text(encoding="utf-8"))
            full_runtime = isinstance(marker, dict) and (not gpu or marker.get("gpu") is True or "+cu" in marker.get("torch", ""))
        except (OSError, ValueError, TypeError):
            full_runtime = False
        if formats - {"gguf"} and (repairing or not full_runtime):
            # Downloaded wheels, unpacked packages and temporary installation
            # space coexist. This is a conservative peak, not just final size.
            required += sum(f["size"] for f in catalog.manifest["wheels"]["cu128" if gpu else "cpu"]) * 3 + 2_000_000_000
        if "gguf" in formats and (repairing or not catalog.llama_ready(gpu=installer.use_gpu("llama-runtime"))):
            required += sum(f["size"] for f in catalog.manifest["llama_runtime" if installer.use_gpu("llama-runtime") else "llama_cpu_runtime"]["files"]) * 3
        if "cosyvoice3" in formats and (repairing or not catalog.cosy_ready()):
            required += 1_400_000_000
        if shutil.disk_usage(managed).free < required + 1_000_000_000:
            raise ValueError("Insufficient disk space for selected models and temporary files")
        if repairing:
            if "gguf" in formats:
                installer.install("llama-runtime", event, progress)
            if formats - {"gguf"}:
                if not managed_python:
                    raise ValueError("Repair must run in managed Python; original environment retained")
                installer.install("runtime", event, progress)
                full_runtime = True
            if "cosyvoice3" in formats:
                installer.install("cosy-runtime", event, progress)
        for ident in ids:
            spec = catalog.manifest["models"][ident]
            if spec.get("format") == "gguf" and not catalog.llama_ready(gpu=installer.use_gpu("llama-runtime")):
                installer.install("llama-runtime", event, progress)
            elif spec.get("format") != "gguf":
                managed_python = normal_path(sys.executable).resolve().is_relative_to((managed / "runtime").resolve())
                runtime_marker = (managed if managed_python else components) / "runtime-ok.json"
                if not full_runtime:
                    if not managed_python:
                        raise ValueError("Install a new voice runtime only inside the managed Python environment")
                    installer.install("runtime", event, progress)
                    full_runtime = True
                if spec.get("format") == "cosyvoice3" and not catalog.cosy_ready():
                    if not normal_path(sys.executable).resolve().is_relative_to((managed / "runtime").resolve()):
                        raise ValueError("Install CosyVoice only inside the managed Python environment")
                    installer.install("cosy-runtime", event, progress)
            if ident in missing: installer.install(ident, event, progress)
        emit("result", installed=ids, downloaded_models=missing)
        return
    if operation == "probe":
        result = []
        for ident, spec in catalog.manifest["models"].items():
            required = "llama" if spec.get("format") == "gguf" else "cosy" if spec.get("format") == "cosyvoice3" else "qwen"
            environment = catalog.llama_ready(gpu=False) if required == "llama" else catalog.cosy_ready() if required == "cosy" else (components / "runtime-ok.json").is_file()
            result.append(dict(id=ident, name=spec.get("name", ident), installed=catalog.ready(ident),
                               environment=environment, bytes=sum(f["size"] for f in spec["files"]),
                               path=str(catalog.path(ident)), kind="text" if ident.startswith("text-") else "voice"))
        emit("result", models=result)
        return
    if operation == "export":
        import zipfile
        output = root / "library.zip"
        rows = request["rows"]
        with zipfile.ZipFile(output, "x", compression=zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("phrases.json", json.dumps(dict(version=1, phrases=rows), ensure_ascii=False, indent=2))
            names = set()
            for row in rows:
                name = row.get("audio_path")
                if name and name not in names:
                    path = (library / "audio" / name).resolve()
                    if path.parent != (library / "audio").resolve(): raise ValueError("Invalid archive audio path")
                    archive.write(path, "audio/" + name)
                    names.add(name)
        emit("result", archive="library.zip")
        return
    class Inference(Engine):
        def direction(self, text, category, event, progress, text_model=None, use_text_model=True):
            # The legacy actor-label generator uses the llama wrapper's chat API.
            # Transformers models have a different generate() signature; the
            # deterministic text-aware selector provides delivery there as well.
            ident = text_model or store.values["text_model"]
            supported = catalog.manifest["models"][ident].get("format") == "gguf"
            return super().direction(text, category, event, progress, text_model=text_model,
                                     use_text_model=use_text_model and supported)
        def refresh_voice(self):
            # A text-only operation does not require an installed voice/reference.
            if store.values["tts_model"] == "system":
                self.references, self.reference, self.profile = {}, None, {"transcript": ""}
                self.profile_hash = "system"
                return
            refs = Profiles(APP, library).voice(store.values["voice_profile"])
            self.references = {ref["hash"]: ref for ref in refs}
            self.reference, self.profile = refs[0]["path"], {"transcript": refs[0]["transcript"]}
            self.profile_hash = hashlib.sha256(json.dumps([(r["hash"], r["transcript"]) for r in refs], ensure_ascii=False).encode()).hexdigest()
    event = TaskCancellation(root)
    engine = Inference(APP, store, catalog)
    def progress(message, done=0, total=0): emit("progress", message=message, done=done, total=total)
    rows = request.get("rows", [])
    try:
        for task in request.get("tasks", []):
            stage("text", event)
            text = task.get("text") or engine.text(task["category"], event, progress,
                        model=store.values["runtime_text_model"] if request.get("runtime") else None,
                        followup=task.get("followup", False), style_retries=not request.get("runtime", False))
            row = dict(id=uuid.uuid4().hex, text=text, category=task["category"],
                       model=store.values["tts_model"], profile=engine.profile_hash, day=request["day"],
                       approved=request.get("approved", False), deleted=False, rejected=False,
                       used_at=None, created_at=datetime.now(timezone.utc).isoformat(), followup=task.get("followup", False),
                       language=store.values["speech_language"], seed=secrets.randbelow(2**31),
                       audio_path=None, voice_style="", voice_instruction="", voice_reference="")
            store.history.insert(0, text)
            # Persist the text before optional delivery generation can fail/cancel.
            emit("text", row=dict(row))
            if store.values["tts_model"] == "tts-cosyvoice3":
                stage("delivery", event)
                direction = engine.direction(text, row["category"], event, progress,
                                text_model=store.values["runtime_text_model"] if request.get("runtime") else None,
                                use_text_model=store.values.get("text_enabled", True))
                row.update(direction); store.styles.insert(0, row["voice_style"])
            rows.append(row)
        if operation == "speech":
            for row in rows:
                stage("delivery", event)
                row["seed"] = secrets.randbelow(2**31)
                row["profile"] = engine.profile_hash
                if store.values["tts_model"] == "tts-cosyvoice3":
                    row.update(engine.direction(row["text"], row["category"], event, progress,
                                               use_text_model=store.values.get("text_enabled", True)))
                    store.styles.insert(0, row["voice_style"])
        stage("release_text", event)
        engine.unload()  # Never keep the text model on the GPU while loading voice.
        for row in rows:
            stage("voice", event)
            store.rows[row["id"]] = dict(row)
            if store.values["tts_model"] != "system":
                row = engine.speech(store.rows[row["id"]], store.values["tts_model"], event, progress)
            else:
                row.update(model="system",profile="system",audio_path=None,voice_style="",voice_instruction="",voice_reference="")
            stage("publish", event)
            emit("phrase", row=row, rotation=store.rotation)
        emit("result", count=len(rows), rotation=store.rotation)
    finally:
        engine.unload()

TASK = ""
if __name__ == "__main__":
    try:
        line = sys.stdin.buffer.readline(4_000_001)
        if len(line) > 4_000_000: raise ValueError("Request is too large")
        request = json.loads(line.decode("utf-8"))
        TASK = request["task"]
        if request.get("version") != 1: raise ValueError("Unsupported worker protocol")
        with contextlib.redirect_stdout(sys.stderr):
            # emit retains the original protocol stream while model libraries print.
            pass
        protocol = sys.stdout
        def emit(kind, **data):
            print(json.dumps(dict(version=1, task=TASK, kind=kind, **data), ensure_ascii=False), file=protocol, flush=True)
        with contextlib.redirect_stdout(sys.stderr): run(request)
    except Exception as error:
        emit("error", message=str(error))
        sys.exit(1)
