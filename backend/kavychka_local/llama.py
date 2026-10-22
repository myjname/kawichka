"""Private, cancellable llama.cpp process. No weights or native DLLs in the app payload."""
import ctypes
import json
import os
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path
from .downloads import checkpoint


class WindowsJob:
    """Kill the model server even if the desktop app force-stops its Python backend."""
    def __init__(self, process):
        self.handle = None
        if os.name != "nt":
            return
        from ctypes import wintypes as w
        class Basic(ctypes.Structure):
            _fields_ = [("ProcessTime", ctypes.c_longlong), ("JobTime", ctypes.c_longlong),
                        ("Flags", w.DWORD), ("MinWorkingSet", ctypes.c_size_t), ("MaxWorkingSet", ctypes.c_size_t),
                        ("ActiveProcessLimit", w.DWORD), ("Affinity", ctypes.c_size_t),
                        ("PriorityClass", w.DWORD), ("SchedulingClass", w.DWORD)]
        class Counters(ctypes.Structure):
            _fields_ = [(name, ctypes.c_ulonglong) for name in ("ReadCount", "WriteCount", "OtherCount", "ReadBytes", "WriteBytes", "OtherBytes")]
        class Extended(ctypes.Structure):
            _fields_ = [("Basic", Basic), ("Io", Counters), ("ProcessMemory", ctypes.c_size_t),
                        ("JobMemory", ctypes.c_size_t), ("PeakProcessMemory", ctypes.c_size_t), ("PeakJobMemory", ctypes.c_size_t)]
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateJobObjectW.restype = w.HANDLE
        kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, w.LPCWSTR]
        kernel.SetInformationJobObject.argtypes = [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD]
        kernel.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
        kernel.CloseHandle.argtypes = [w.HANDLE]
        handle = kernel.CreateJobObjectW(None, None)
        if not handle:
            raise ctypes.WinError(ctypes.get_last_error())
        info = Extended(); info.Basic.Flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if (not kernel.SetInformationJobObject(handle, 9, ctypes.byref(info), ctypes.sizeof(info))
                or not kernel.AssignProcessToJobObject(handle, w.HANDLE(int(process._handle)))):
            reason = ctypes.get_last_error(); kernel.CloseHandle(handle)
            raise ctypes.WinError(reason)
        self.handle, self.kernel = handle, kernel

    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle); self.handle = None


class Llama:
    def __init__(self, executable, model, data, device, event, progress):
        self.process = self.job = self.log = None
        self.keyfile = None
        self.root = Path(data)
        self.token = secrets.token_hex(32)
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0)); port = sock.getsockname()[1]
        self.address = "http://127.0.0.1:" + str(port)
        self.keyfile = self.root / ("llama-" + secrets.token_hex(8) + ".key")
        self.log_path = self.root / "llama.log"
        try:
            self.keyfile.write_text(self.token, encoding="ascii")
            self.log = self.log_path.open("w", encoding="utf-8")
            args = [str(executable), "-m", str(model), "--host", "127.0.0.1", "--port", str(port),
                    "--api-key-file", str(self.keyfile), "--no-webui", "--ctx-size", "4096", "--parallel", "1",
                    "--n-gpu-layers", "0" if device == "cpu" else "99", "--batch-size", "512", "--ubatch-size", "256",
                    "--flash-attn", "off", "--jinja", "--chat-template-kwargs", '{"enable_thinking":false}']
            if device == "cpu":
                args += ["--device", "none", "--no-kv-offload", "--no-op-offload"]
            elif device == "cuda":
                args += ["--device", "CUDA0"]
            env = dict(os.environ, GGML_CUDA_DISABLE_GRAPHS="1")
            self.process = subprocess.Popen(args, cwd=Path(executable).parent, stdout=self.log, stderr=subprocess.STDOUT,
                                            env=env, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            self.job = WindowsJob(self.process)
            deadline = time.monotonic() + 180
            while True:
                checkpoint(event)
                if self.process.poll() is not None:
                    raise RuntimeError("Модель не запустилась. Проверьте среду llama.cpp, файл модели и доступную память"
                                       + ("; для CUDA также драйвер NVIDIA. " if device != "cpu" else ". ") + self.error_tail())
                try:
                    request = urllib.request.Request(self.address + "/health", headers={"Authorization": "Bearer " + self.token})
                    with self.http.open(request, timeout=1) as response:
                        if json.load(response).get("status") == "ok":
                            break
                except (OSError, ValueError):
                    pass
                if time.monotonic() > deadline:
                    raise RuntimeError("Модель текста не загрузилась за 180 секунд. " + self.error_tail())
                event.wait(0.15)
            self.keyfile.unlink(missing_ok=True)
        except BaseException:
            self.close(); raise

    def error_tail(self):
        return self.log_path.read_text(encoding="utf-8", errors="replace")[-1200:]

    def generate(self, messages, event, max_tokens=180):
        checkpoint(event)
        body = dict(messages=messages, max_tokens=max_tokens, temperature=0.7, top_p=0.8, top_k=20, min_p=0.0,
                    presence_penalty=0.0, repeat_penalty=1.0, stream=True, cache_prompt=False,
                    chat_template_kwargs={"enable_thinking": False}, seed=secrets.randbelow(2**31))
        request = urllib.request.Request(self.address + "/v1/chat/completions", data=json.dumps(body, ensure_ascii=False).encode("utf-8"),
                                         headers={"Authorization": "Bearer " + self.token, "Content-Type": "application/json"})
        parts = []
        try:
            with self.http.open(request, timeout=120) as response:
                for line in response:
                    checkpoint(event)
                    if not line.startswith(b"data: "):
                        continue
                    payload = line[6:].strip()
                    if payload == b"[DONE]":
                        break
                    item = json.loads(payload)
                    if item.get("error"):
                        raise ValueError("Ошибка генератора текста")
                    if item.get("choices"):
                        parts.append(item["choices"][0].get("delta", {}).get("content") or "")
        except (OSError, ValueError) as error:
            checkpoint(event)
            raise RuntimeError("Генерация текста прервалась. Повторите задание. " + self.error_tail()) from error
        checkpoint(event)
        text = "".join(parts).strip()
        if not text:
            raise ValueError("Текстовая модель вернула пустой ответ")
        return text

    def close(self):
        if self.job:
            self.job.close(); self.job = None
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try: self.process.wait(timeout=10)
            except subprocess.TimeoutExpired: self.process.kill(); self.process.wait(timeout=10)
        if self.log:
            self.log.close(); self.log = None
        if self.keyfile:
            self.keyfile.unlink(missing_ok=True)
