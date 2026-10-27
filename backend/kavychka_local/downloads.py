import hashlib
import json
import os
import subprocess
import sys
import threading
import uuid
import urllib.error
import urllib.request
import zipfile
from collections import deque
from datetime import datetime, timezone
from pathlib import Path

BUILD_TOOLS = ("setuptools==80.9.0", "wheel==0.45.1", "packaging==26.3")
SITE_OWNER = b"kavychka-inference-site-v1\n"

class Cancelled(Exception):
    pass

def checkpoint(event):
    if event.is_set():
        raise Cancelled("Отменено пользователем; загрузку можно продолжить")

def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(4 * 1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def open_download(request):
    try:
        return urllib.request.urlopen(request, timeout=25)
    except urllib.error.URLError as error:
        import ssl
        if not isinstance(error.reason, ssl.SSLCertVerificationError):
            raise
        # Some Windows root stores lack a current public issuer. Retry with
        # the qualified CA bundle, keeping certificate/hostname verification.
        # Before the initial Python bootstrap certifi may not be installed.
        try:
            import certifi
        except ImportError:
            raise error
        context = ssl.create_default_context(cafile=certifi.where())
        return urllib.request.urlopen(request, timeout=25, context=context)

def download(url, path, size, sha256, event, progress):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_file() and (not size or path.stat().st_size == size) and digest(path) == sha256:
        progress("Проверено: " + path.name, size, size)
        return
    temporary = path.with_name(path.name + ".part")
    offset = temporary.stat().st_size if temporary.exists() else 0
    if size and offset > size:
        temporary.unlink()
        offset = 0
    checkpoint(event)
    headers = {"User-Agent": "Kawichka/5.0", "Accept-Encoding": "identity"}
    if offset:
        headers["Range"] = f"bytes={offset}-"
    try:
        response = open_download(urllib.request.Request(url, headers=headers))
    except urllib.error.HTTPError as error:
        if error.code == 416 and offset == size and digest(temporary) == sha256:
            temporary.replace(path)
            return
        if error.code == 416:
            temporary.unlink(missing_ok=True)
            raise ValueError("Повреждённая докачка сброшена. Повторите загрузку.") from error
        raise
    with response:
        if offset and response.status != 206:
            offset = 0
        elif offset:
            expected = f"bytes {offset}-"
            if not response.headers.get("Content-Range", "").startswith(expected):
                raise ValueError("Сервер вернул неверный диапазон файла")
        with temporary.open("ab" if offset else "wb") as target:
            for block in iter(lambda: response.read(1024 * 1024), b""):
                checkpoint(event)
                target.write(block)
                offset += len(block)
                progress("Загрузка: " + path.name, offset, size)
    checkpoint(event)
    progress("Проверка SHA-256: " + path.name, offset, size)
    if (size and offset != size) or digest(temporary) != sha256:
        temporary.unlink(missing_ok=True)
        raise ValueError("Проверка целостности не пройдена. Повторите загрузку.")
    temporary.replace(path)

class Components:
    def __init__(self, app_root, data_root, store):
        self.app_root, self.root, self.store = Path(app_root), Path(data_root), store
        self.manifest = json.loads((self.app_root / "assets" / "components.json").read_text(encoding="utf-8"))
        self._hardware = None

    def hardware(self):
        if self._hardware is None:
            from .hardware import probe
            self._hardware = probe()
        return dict(self._hardware)

    def use_gpu(self, key):
        chosen = self.store.settings()["device"] if self.store is not None else "auto"
        if chosen == "cpu":
            return False
        if chosen == "cuda":
            return True
        return self.hardware()["text_gpu" if key == "llama-runtime" else "speech_gpu"]

    def llama_device(self):
        chosen = self.store.settings()["device"]
        if chosen != "auto":
            return chosen
        return "cuda" if self.use_gpu("llama-runtime") and self.llama_ready(gpu=True) else "cpu"

    def voice_device(self):
        chosen = self.store.settings()["device"]
        return "cpu" if chosen == "auto" and not self.use_gpu("runtime") else chosen

    def runtime_wheels(self, gpu):
        return self.manifest["wheels"][self.manifest.get("cuda_wheel_set", "cu128") if gpu else "cpu"]

    def runtime_python(self):
        self.check_package_path(self.root / "runtime")
        python = Path(os.path.realpath(sys.executable))
        if python != (self.root / "runtime/python.exe").resolve():
            raise ValueError("Install or repair must run in the managed Python; original environment retained")
        return str(python)

    def path(self, key):
        custom = self.store.settings()["model_paths"].get(key)
        return Path(custom) if custom else self.root / "models" / key

    def ready(self, key):
        path = self.path(key)
        return all((path / f["path"]).is_file() and (path / f["path"]).stat().st_size == f["size"] for f in self.manifest["models"][key]["files"])

    def verify_inference(self, key, event, progress):
        """Only catalog-pinned configurations/checkpoints may reach ML loaders.

        Size checks used by the UI are insufficient for pickle/YAML/config
        consumers. Run cancellable hashing in the worker before each load;
        never download or alter a custom/previous model directory here.
        """
        folder = self.path(key)
        for item in self.manifest["models"][key]["files"]:
            checkpoint(event)
            path = folder / item["path"]
            if not path.is_file() or path.stat().st_size != item["size"]:
                raise ValueError("Model files are incomplete; check or repair the selected model")
            progress("Проверка модели / Verifying model: " + path.name, 0, 0)
            value = hashlib.sha256()
            with path.open("rb") as source:
                for block in iter(lambda: source.read(4 * 1024 * 1024), b""):
                    checkpoint(event)
                    value.update(block)
            if value.hexdigest() != item["sha256"]:
                raise ValueError("Model integrity check failed; check or repair the selected model")
        checkpoint(event)

    def llama_executable(self):
        return self.root / "llama" / "llama-server.exe"

    def llama_ready(self, gpu=None):
        marker = self.root / "llama" / "verified.json"
        try:
            saved = json.loads(marker.read_text(encoding="utf-8"))
            if not isinstance(saved, dict) or ("gpu" in saved and not isinstance(saved["gpu"], bool)):
                return False
            installed_gpu = saved.get("gpu", True)  # Preserve the original CUDA-only marker.
            spec = self.manifest["llama_runtime" if installed_gpu else "llama_cpu_runtime"]
            required = ["ggml-base.dll", "ggml-cpu-x64.dll", "ggml.dll", "llama.dll", "llama-common.dll", "llama-server-impl.dll", "libomp.dll"]
            if installed_gpu:
                required += ["ggml-cuda.dll", "cublas64_13.dll", "cublasLt64_13.dll"]
            return (saved["revision"] == spec["revision"] and (gpu is not True or installed_gpu)
                    and self.llama_executable().is_file()
                    and all((self.root / "llama" / name).is_file() for name in required))
        except (OSError, ValueError, KeyError, TypeError):
            return False

    def cosy_ready(self):
        try:
            saved = json.loads((self.root / "cosy/verified.json").read_text(encoding="utf-8"))
            profile = self.manifest.get("runtime_profile")
            return (saved["revision"] == self.manifest["cosy_runtime"]["revision"]
                    and (not profile or saved.get("profile") == profile)
                    and ((self.root / "cosy/site/transformers/__init__.py").is_file()
                         or (self.root / "runtime/Lib/site-packages/transformers/__init__.py").is_file())
                    and (self.root / "cosy/site/numpy/__init__.py").is_file()
                    and (self.root / "cosy/Whisper/whisper/__init__.py").is_file()
                    and (self.root / "cosy/CosyVoice/cosyvoice/cli/cosyvoice.py").is_file()
                    and (self.root / "cosy/Matcha-TTS/matcha/utils/audio.py").is_file())
        except (OSError, ValueError, KeyError):
            return False

    def status(self):
        import importlib.util
        speech_gpu = self.use_gpu("runtime")
        try:
            marker = json.loads((self.root / "runtime-ok.json").read_text(encoding="utf-8"))
            if not isinstance(marker, dict) or ("gpu" in marker and not isinstance(marker["gpu"], bool)):
                raise ValueError("Invalid runtime marker")
            runtime_gpu = marker["gpu"] is True if "gpu" in marker else "+cu" in marker.get("torch", "")
            profile = self.manifest.get("runtime_profile")
            runtime = (not profile or marker.get("profile") == profile) and (not speech_gpu or runtime_gpu) and all(importlib.util.find_spec(name) for name in ("torch", "qwen_tts", "soundfile", "transformers", "tzdata"))
        except (OSError, ValueError, TypeError):
            runtime = False
        runtime_bytes = sum(file["size"] for file in self.runtime_wheels(speech_gpu)) + 200000000
        items = [{"id": "runtime", "name": "Python + PyTorch + Qwen runtime", "installed": bool(runtime), "bytes": runtime_bytes, "disk_bytes": 7000000000 if speech_gpu else 2000000000}]
        gpu = self.use_gpu("llama-runtime")
        size = sum(f["size"] for f in self.manifest["llama_runtime" if gpu else "llama_cpu_runtime"]["files"])
        items.append({"id": "llama-runtime", "name": "llama.cpp · генератор текста", "installed": self.llama_ready(gpu=gpu), "bytes": size, "disk_bytes": 1500000000 if gpu else 150000000})
        items.append({"id": "cosy-runtime", "name": "Среда CosyVoice · отдельно от Qwen", "installed": self.cosy_ready(), "bytes": 200000000, "disk_bytes": 700000000})
        for key, model in self.manifest["models"].items():
            if key == "text-1.7b":
                continue  # Retained only for legacy model path migration.
            size = sum(f["size"] for f in model["files"])
            items.append({"id": key, "name": model.get("name", model["repo"]), "installed": self.ready(key), "bytes": size, "disk_bytes": size + 200000000})
        return items

    def install(self, key, event, progress, gpu=None):
        if gpu is None:
            gpu = self.use_gpu(key)
        if key == "runtime":
            return self.install_runtime(event, progress, gpu)
        if key == "llama-runtime":
            return self.install_llama(event, progress, gpu)
        if key == "cosy-runtime":
            return self.install_cosy(event, progress)
        spec = self.manifest["models"][key]
        folder = self.path(key)
        total = sum(f["size"] for f in spec["files"])
        completed = 0
        for file in spec["files"]:
            checkpoint(event)
            download(file["url"], folder / file["path"], file["size"], file["sha256"], event,
                     lambda message, done, size, base=completed: progress(message, base + done, total))
            completed += file["size"]
        (folder / "verified.json").write_text(json.dumps({"revision": spec["revision"]}), encoding="utf-8")
        return {"installed": key}

    def install_llama(self, event, progress, gpu=None):
        if gpu is None:
            gpu = self.use_gpu("llama-runtime")
        if self.llama_ready(gpu=gpu):
            return {"installed": "llama-runtime", "reused": True}
        spec = self.manifest["llama_runtime" if gpu else "llama_cpu_runtime"]
        folder = self.root / "llama"
        folder.mkdir(exist_ok=True)
        (folder / "verified.json").unlink(missing_ok=True)
        for file in spec["files"]:
            target = self.root / "downloads" / file["path"]
            download(file["url"], target, file["size"], file["sha256"], event, progress)
            with zipfile.ZipFile(target) as archive:
                for item in archive.infolist():
                    checkpoint(event)
                    destination = (folder / item.filename).resolve()
                    if folder.resolve() not in destination.parents and destination != folder.resolve():
                        raise ValueError("Недопустимый путь в архиве среды")
                    archive.extract(item, folder)
        checkpoint(event)
        (folder / "verified.json").write_text(json.dumps({"revision": spec["revision"], "gpu": gpu}), encoding="utf-8")
        if not self.llama_ready(gpu=gpu):
            (folder / "verified.json").unlink(missing_ok=True)
            raise ValueError("Файлы генератора текста неполны. Повторите установку.")
        return {"installed": "llama-runtime"}

    def install_cosy(self, event, progress):
        spec = self.manifest["cosy_runtime"]
        folder = self.root / "cosy"
        self.check_package_path(folder)
        folder.mkdir(exist_ok=True)
        self.clean_old_sites(folder, "site-install-*")
        (folder / "verified.json").unlink(missing_ok=True)
        for file in spec["files"]:
            archive_path = self.root / "downloads" / file["path"]
            download(file["url"], archive_path, file["size"], file["sha256"], event, progress)
            destination = folder / file["directory"]
            with zipfile.ZipFile(archive_path) as archive:
                for item in archive.infolist():
                    checkpoint(event)
                    parts = Path(item.filename).parts[1:]
                    if not parts or item.is_dir(): continue
                    path = (destination / Path(*parts)).resolve()
                    if destination.resolve() not in path.parents:
                        raise ValueError("Неверный путь в архиве CosyVoice")
                    path.parent.mkdir(parents=True, exist_ok=True)
                    with archive.open(item) as source, path.open("wb") as target:
                        import shutil
                        shutil.copyfileobj(source, target)
        temporary = self.root / "tmp"; temporary.mkdir(exist_ok=True)
        env = dict(os.environ, TMP=str(temporary), TEMP=str(temporary), PIP_DISABLE_PIP_VERSION_CHECK="1",
                   PYTHONDONTWRITEBYTECODE="1", PYTHONIOENCODING="utf-8")
        # Windows AppData virtualization can resolve a package directory and its
        # launcher files differently. Start Python by its real executable path.
        python = self.runtime_python()
        self.prepare_build_tools(python, env, event, progress, "cosy-install.log")
        # A new layer must not retain the old Transformers/tokenizers packages
        # that would shadow the upgraded shared runtime after --target updates.
        site = folder / "site"
        stage = folder / ("site-install-" + uuid.uuid4().hex)
        backup = folder / ("site-before-" + uuid.uuid4().hex)
        stage.mkdir()
        (stage / ".kavychka-site.owner").write_bytes(SITE_OWNER)
        published = moved = False
        try:
            command = [python, "-m", "pip", "install", "--no-cache-dir", "--no-compile", "--no-deps", "--no-build-isolation",
                       "--target", str(stage), "-r", str(self.app_root / "backend/requirements-cosy.txt")]
            self.run_install(command, env, event, progress, "cosy-install.log")
            command = [python, "-B", str(self.app_root / "backend/cosy_worker.py"), "--data", str(self.root), "--check-only", "--site", str(stage)]
            self.run_install(command, env, event, progress, "cosy-install.log")
            checkpoint(event)
            if site.exists():
                self.check_site_tree(site, folder)
                (site / ".kavychka-site.owner").write_bytes(SITE_OWNER)
                site.rename(backup); moved = True
            stage.rename(site); published = True
            (folder / "verified.json").write_text(json.dumps({"revision": spec["revision"], "profile": self.manifest.get("runtime_profile")}), encoding="utf-8")
            if not self.cosy_ready():
                raise ValueError("Установка среды CosyVoice неполна")
        except BaseException:
            (folder / "verified.json").unlink(missing_ok=True)
            if published:
                self.remove_site_tree(site, folder)
            if moved:
                backup.rename(site)
            raise
        finally:
            if stage.exists():
                self.remove_site_tree(stage, folder)
        if moved:
            self.remove_site_tree(backup, folder)
        self.clean_old_sites(folder, "site-before-*")
        for file in spec["files"]: (self.root / "downloads" / file["path"]).unlink(missing_ok=True)
        return {"installed": "cosy-runtime"}

    @staticmethod
    def check_package_path(path):
        for item in [Path(path).absolute(), *Path(path).absolute().parents]:
            try:
                info = item.lstat()
            except FileNotFoundError:
                continue
            if item.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
                raise ValueError("Linked inference package directory retained")

    @staticmethod
    def check_site_tree(path, parent):
        # Exact managed package directory only; never models, presets or links.
        absolute, base = Path(path).absolute(), Path(parent).resolve()
        Components.check_package_path(parent)
        if absolute.parent.resolve() != base or not (absolute.name == "site" or absolute.name.startswith(("site-install-", "site-before-"))):
            raise ValueError("Unsafe inference package directory")
        count = 0
        def visit(item, depth):
            nonlocal count
            count += 1
            if depth > 64 or count > 100000:
                raise ValueError("Inference package tree exceeds safe bounds")
            info = item.lstat()
            if item.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
                raise ValueError("Linked inference package directory retained")
            if item.is_dir():
                for child in item.iterdir():
                    visit(child, depth + 1)
        visit(absolute, 0)

    @classmethod
    def clean_old_sites(cls, parent, pattern):
        for path in parent.glob(pattern):
            try:
                cls.check_site_tree(path, parent)
                owned = (path / ".kavychka-site.owner").read_bytes() == SITE_OWNER
            except (OSError, ValueError):
                continue
            if owned:
                cls.remove_site_tree(path, parent)

    @classmethod
    def remove_site_tree(cls, path, parent):
        import shutil
        cls.check_site_tree(path, parent)
        shutil.rmtree(path)

    def prepare_build_tools(self, python, env, event, progress, log_name):
        progress("Подготовка инструментов сборки Python", 0, 0)
        self.run_install([python, "-m", "pip", "install", "--no-cache-dir", "--no-compile",
                          "--only-binary=:all:", "--upgrade", *BUILD_TOOLS], env, event, progress, log_name)

    def run_install(self, command, env, event, progress, log_name="runtime-install.log"):
        checkpoint(event)
        log_path = self.root / "logs" / log_name
        log_path.parent.mkdir(parents=True, exist_ok=True)
        tail = deque(maxlen=12)
        with log_path.open("a", encoding="utf-8", buffering=1) as log:
            log.write("\n[" + datetime.now(timezone.utc).isoformat() + "] " + subprocess.list2cmdline(command) + "\n")
            proc = None
            job = None
            stop_lock = threading.Lock()
            finished = threading.Event()
            def stop():
                with stop_lock:
                    if job is not None:
                        job.close()  # Also closes inherited stdout handles in build subprocesses.
                    if proc is None or proc.poll() is not None:
                        return
                    if os.name == "nt":
                        subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True,
                                       creationflags=subprocess.CREATE_NO_WINDOW)
                    if proc.poll() is None:
                        proc.kill()
            def cancel():
                while not finished.wait(0.2):
                    if event.is_set():
                        stop(); return
            try:
                proc = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                        encoding="utf-8", errors="replace", env=dict(env, PYTHONIOENCODING="utf-8", PYTHONUNBUFFERED="1"),
                                        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
                if os.name == "nt":
                    from .llama import WindowsJob
                    try:
                        job = WindowsJob(proc)
                    except Exception:
                        stop(); proc.wait(); proc.stdout.close(); raise
                watcher = threading.Thread(target=cancel, daemon=True)
                watcher.start()
                try:
                    for line in proc.stdout:
                        log.write(line)
                        if line.strip():
                            tail.append(line.strip())
                            progress(line.strip()[-350:], 0, 0)
                    code = proc.wait()
                finally:
                    finished.set()
                    stop()
                    proc.wait()
                    watcher.join(timeout=2)
                    proc.stdout.close()
                    log.write("\nExit code: " + str(proc.returncode) + "\n")
            except Exception as error:
                log.write(type(error).__name__ + ": " + str(error) + "\n")
                raise RuntimeError("Не удалось выполнить установку: " + str(error) + "\nПолный журнал: " + str(log_path)) from error
        checkpoint(event)
        if code:
            raise RuntimeError("Установка не завершена:\n" + "\n".join(tail) + "\nПолный журнал: " + str(log_path))
        return "\n".join(tail)

    def install_runtime(self, event, progress, gpu):
        if "torch" in sys.modules:
            raise ValueError("Для замены среды сначала перезапустите локальный процесс через кнопку установки в приложении.")
        python = self.runtime_python()
        (self.root / "runtime-ok.json").unlink(missing_ok=True)
        bootstrap = self.manifest["pip"]
        target = self.root / "downloads" / "get-pip.py"
        download(bootstrap["url"], target, bootstrap["size"], bootstrap["sha256"], event, progress)
        wheels = self.runtime_wheels(gpu)
        total = sum(f["size"] for f in wheels)
        completed = 0
        wheel_paths = []
        for wheel in wheels:
            path = self.root / "downloads" / wheel["path"]
            download(wheel["url"], path, wheel["size"], wheel["sha256"], event,
                     lambda message, done, size, base=completed: progress(message, base + done, total))
            wheel_paths.append(str(path))
            completed += wheel["size"]
        temporary = self.root / "tmp"; temporary.mkdir(exist_ok=True)
        env = dict(os.environ, TMP=str(temporary), TEMP=str(temporary), PIP_CACHE_DIR=str(self.root / "pip-cache"), PYTHONDONTWRITEBYTECODE="1", PIP_DISABLE_PIP_VERSION_CHECK="1", PYTHONIOENCODING="utf-8")
        from .qwen_package import build
        qwen = self.manifest["qwen_inference"]
        upstream = self.root / "downloads" / qwen["path"]
        download(qwen["url"], upstream, qwen["size"], qwen["sha256"], event, progress)
        inference_wheel = build(upstream, self.root / "downloads/inference")
        if qwen.get("derived_sha256") and digest(inference_wheel) != qwen["derived_sha256"]:
            raise ValueError("Derived inference wheel integrity check failed")
        commands = [
            [python, str(target), "--no-cache-dir", "--no-compile", "pip==26.2.1"],
            [python, "-m", "pip", "install", "--no-cache-dir", "--no-compile", "--only-binary=:all:", "--upgrade", *BUILD_TOOLS],
            [python, "-m", "pip", "uninstall", "--yes", "qwen-tts", "gradio", "gradio-client", "hf-gradio"],
            [python, "-m", "pip", "install", "--no-cache-dir", "--no-compile", "--force-reinstall", "--no-deps"] + wheel_paths,
            [python, "-m", "pip", "install", "--no-cache-dir", "--no-compile", "--no-build-isolation", str(inference_wheel), "-r", str(self.app_root / "backend" / "requirements.txt")],
            [python, "-m", "pip", "check"],
        ]
        for command in commands:
            checkpoint(event)
            progress("Установка среды; подробности ниже", 0, 0)
            self.run_install(command, env, event, progress)
        import importlib
        importlib.invalidate_caches()
        check = self.run_install([python, "-B", str(self.app_root / "backend/health_check.py"), "--inference-runtime"] + (["--gpu"] if gpu else []),
                                 env, event, progress)
        checkpoint(event)
        checked = json.loads(check.splitlines()[-1])
        (self.root / "runtime-ok.json").write_text(json.dumps(dict(checked, gpu=gpu, profile=self.manifest["runtime_profile"])), encoding="utf-8")
        return {"installed": "runtime"}


class ReadOnlyComponents(Components):
    """Reuse existing weights/runtimes in isolated verification libraries."""
    def install(self, *args, **kwargs):
        raise ValueError("Component installation is disabled for this read-only model store")
