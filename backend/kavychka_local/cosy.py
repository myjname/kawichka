import contextlib
import json
import os
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path
from .downloads import checkpoint
from .llama import WindowsJob

def prepare_precision(llm, cuda_available):
    """CPU inputs are float32; pretrained Qwen's config may default to BF16."""
    if not cuda_available:
        llm.float()

@contextlib.contextmanager
def inference_errors():
    """Surface a failed private LLM thread before publishing a partial wave."""
    previous = threading.excepthook
    failures = []
    def capture(args):
        failures.append(args.exc_value)
        previous(args)  # Preserve the original traceback in the private log.
    threading.excepthook = capture
    try:
        try:
            yield
        except BaseException:
            if not failures:
                raise
        if failures:
            error = failures[0]
            raise RuntimeError("CosyVoice generation thread failed: " + type(error).__name__ + ": " + str(error)) from error
    finally:
        threading.excepthook = previous


class Cosy:
    def __init__(self, app, root, model, device, event, runtime=None):
        self.process = self.job = self.log = None
        self.messages = queue.Queue()
        self.log_path = Path(root) / "cosy.log"
        try:
            self.log = self.log_path.open("w", encoding="utf-8")
            self.process = subprocess.Popen([sys.executable, "-B", str(Path(app) / "backend/cosy_worker.py"),
                                             "--data", str(root), "--runtime", str(runtime or root), "--model", str(model), "--device", device],
                                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
                                            text=True, encoding="utf-8", env=dict(os.environ, PYTHONIOENCODING="utf-8"),
                                            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            self.job = WindowsJob(self.process)
            def read():
                for line in self.process.stdout:
                    self.messages.put(line)
                self.messages.put(None)
            self.reader = threading.Thread(target=read, daemon=True); self.reader.start()
            self.receive(event, 240)
        except BaseException:
            self.close(); raise

    def receive(self, event, timeout):
        deadline = time.monotonic() + timeout
        while True:
            checkpoint(event)
            try:
                line = self.messages.get(timeout=0.1)
            except queue.Empty:
                if time.monotonic() > deadline:
                    raise RuntimeError("CosyVoice не ответила вовремя. Повторите генерацию.")
                continue
            if line is None:
                raise RuntimeError("CosyVoice остановилась: " + self.log_path.read_text(encoding="utf-8", errors="replace")[-1200:])
            result = json.loads(line)
            if result.get("error"):
                raise ValueError("CosyVoice: " + result["error"])
            if not result.get("ready"):
                raise ValueError("Неверный ответ CosyVoice")
            return result

    def generate(self, text, instruction, reference, target, seed, event):
        checkpoint(event)
        self.process.stdin.write(json.dumps(dict(text=text, instruction=instruction, reference=str(reference),
                                                target=str(target), seed=seed), ensure_ascii=False) + "\n")
        self.process.stdin.flush()
        return self.receive(event, 600)

    def close(self):
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try: self.process.wait(timeout=10)
            except subprocess.TimeoutExpired: self.process.kill(); self.process.wait(timeout=10)
        if self.job:
            self.job.close(); self.job = None
        if self.process is not None:
            if self.process.stdin: self.process.stdin.close()
            if self.process.stdout: self.process.stdout.close()
        if self.log:
            self.log.close(); self.log = None
