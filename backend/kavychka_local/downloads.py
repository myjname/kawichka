import hashlib
import json
import os
import subprocess
import sys
import threading
import urllib.error
import urllib.request
import zipfile
from collections import deque
from datetime import datetime, timezone
from pathlib import Path

BUILD_TOOLS = ("setuptools==80.9.0", "wheel==0.45.1", "packaging==26.3")

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
        response = urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=25)
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

    def path(self, key):
        custom = self.store.settings()["model_paths"].get(key)
        return Path(custom) if custom else self.root / "models" / key

    def ready(self, key):
        path = self.path(key)
        return all((path / f["path"]).is_file() and (path / f["path"]).stat().st_size == f["size"] for f in self.manifest["models"][key]["files"])

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
            return (json.loads((self.root / "cosy/verified.json").read_text(encoding="utf-8"))["revision"] == self.manifest["cosy_runtime"]["revision"]
                    and (self.root / "cosy/site/transformers/__init__.py").is_file()
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
            runtime = (not speech_gpu or runtime_gpu) and all(importlib.util.find_spec(name) for name in ("torch", "qwen_tts", "soundfile", "transformers", "tzdata"))
        except (OSError, ValueError, TypeError):
            runtime = False
        runtime_bytes = sum(file["size"] for file in self.manifest["wheels"]["cu128" if speech_gpu else "cpu"]) + 200000000
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
        folder.mkdir(exist_ok=True)
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
        python = os.path.realpath(sys.executable)
        self.prepare_build_tools(python, env, event, progress, "cosy-install.log")
        command = [python, "-m", "pip", "install", "--no-cache-dir", "--no-compile", "--no-deps", "--upgrade", "--no-build-isolation",
                   "--target", str(folder / "site"), "-r", str(self.app_root / "backend/requirements-cosy.txt")]
        self.run_install(command, env, event, progress, "cosy-install.log")
        command = [python, "-B", str(self.app_root / "backend/cosy_worker.py"), "--data", str(self.root), "--check-only"]
        self.run_install(command, env, event, progress, "cosy-install.log")
        checkpoint(event)
        (folder / "verified.json").write_text(json.dumps({"revision": spec["revision"]}), encoding="utf-8")
        if not self.cosy_ready():
            (folder / "verified.json").unlink(missing_ok=True)
            raise ValueError("Установка среды CosyVoice неполна")
        for file in spec["files"]: (self.root / "downloads" / file["path"]).unlink(missing_ok=True)
        return {"installed": "cosy-runtime"}

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
        (self.root / "runtime-ok.json").unlink(missing_ok=True)
        bootstrap = self.manifest["pip"]
        target = self.root / "downloads" / "get-pip.py"
        download(bootstrap["url"], target, bootstrap["size"], bootstrap["sha256"], event, progress)
        wheels = self.manifest["wheels"]["cu128" if gpu else "cpu"]
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
        python = os.path.realpath(sys.executable)
        commands = [
            [python, str(target), "--no-cache-dir", "--no-compile", "pip==26.2.1"],
            [python, "-m", "pip", "install", "--no-cache-dir", "--no-compile", "--only-binary=:all:", "--upgrade", *BUILD_TOOLS],
            [python, "-m", "pip", "install", "--no-cache-dir", "--no-compile", "--force-reinstall", "--no-deps"] + wheel_paths,
            [python, "-m", "pip", "install", "--no-cache-dir", "--no-compile", "--no-build-isolation", "-r", str(self.app_root / "backend" / "requirements.txt")],
        ]
        for command in commands:
            checkpoint(event)
            progress("Установка среды; подробности ниже", 0, 0)
            self.run_install(command, env, event, progress)
        import importlib
        importlib.invalidate_caches()
        check = self.run_install([python, "-B", "-c", "import torch,torchaudio,qwen_tts,soundfile,transformers,tzdata; print(torch.__version__)"],
                                 env, event, progress)
        checkpoint(event)
        (self.root / "runtime-ok.json").write_text(json.dumps({"torch": check.splitlines()[-1], "gpu": gpu}), encoding="utf-8")
        return {"installed": "runtime"}


class ReadOnlyComponents(Components):
    """Reuse existing weights/runtimes in isolated verification libraries."""
    def install(self, *args, **kwargs):
        raise ValueError("Component installation is disabled for this read-only model store")
