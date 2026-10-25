import ctypes
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import io
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.store import Store, TEXT_MODELS
from kavychka_local.downloads import Components


class TextModelTests(unittest.TestCase):
    def test_private_key_uses_ascii_relative_name_inside_unicode_job_directory(self):
        from kavychka_local.llama import Llama
        with tempfile.TemporaryDirectory(dir=ROOT / "work", prefix="папка с пробелом-") as directory:
            process=mock.Mock();process.poll.return_value=None
            http=mock.Mock();http.open.return_value=io.BytesIO(b'{"status":"ok"}')
            with mock.patch('kavychka_local.llama.subprocess.Popen',return_value=process) as popen, mock.patch('kavychka_local.llama.WindowsJob'), mock.patch('kavychka_local.llama.urllib.request.build_opener',return_value=http):
                llama=Llama(ROOT/'work/fake.exe',ROOT/'work/fake.gguf',directory,'cpu',threading.Event(),lambda *_:None)
                try:
                    args=popen.call_args.args[0]
                    key=args[args.index('--api-key-file')+1]
                    self.assertTrue(key.isascii())
                    self.assertEqual(Path(key).name,key)
                    self.assertEqual(Path(directory),popen.call_args.kwargs['cwd'])
                    self.assertNotIn(llama.token,args)
                    self.assertFalse(list(Path(directory).glob('*.key')))
                finally:
                    llama.close()
    def test_quality_and_runtime_selections_are_independent_and_persistent(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            store = Store(directory)
            try:
                self.assertEqual("text-gemma4-12b", store.settings()["text_model"])
                self.assertEqual("text-gemma4-12b", store.settings()["runtime_text_model"])
                for index, key in enumerate(TEXT_MODELS):
                    other = TEXT_MODELS[(index + 1) % len(TEXT_MODELS)]
                    store.configure({"text_model": key, "runtime_text_model": other})
                    self.assertEqual(key, store.settings()["text_model"])
                    self.assertEqual(other, store.settings()["runtime_text_model"])
                store.configure({"text_model": "text-gemma4-e4b"})
                for values in ({"text_model": "tts-1.7b"}, {"runtime_text_model": "cloud"}):
                    with self.assertRaises(ValueError):
                        store.configure(values)
                store.db.close(); store = Store(directory)
                self.assertEqual("text-gemma4-e4b", store.settings()["text_model"])
            finally:
                store.db.close()

    def test_all_seven_choices_have_pinned_weights_and_selection_creates_no_models(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            store = Store(directory)
            try:
                components = Components(ROOT, directory, store)
                self.assertEqual(7, len(TEXT_MODELS))
                for key in TEXT_MODELS:
                    spec = components.manifest["models"][key]
                    self.assertEqual("gguf", spec["format"])
                    self.assertEqual(40, len(spec["revision"]))
                    for file in spec["files"]:
                        self.assertEqual(64, len(file["sha256"]))
                        self.assertIn("/resolve/" + spec["revision"] + "/", file["url"])
                    store.configure({"text_model": key})
                    self.assertFalse(components.ready(key))
                self.assertFalse((Path(directory) / "models").exists())
            finally:
                store.db.close()

    @unittest.skipUnless(os.name == "nt", "Windows process lifetime")
    def test_native_child_is_killed_when_backend_exits_abruptly(self):
        code = (
            "import subprocess,sys; sys.path.insert(0,sys.argv[1]); "
            "from kavychka_local.llama import WindowsJob; "
            "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(120)'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,creationflags=subprocess.CREATE_NO_WINDOW); "
            "job=WindowsJob(child); print(child.pid,flush=True); sys.stdin.readline()"
        )
        process = subprocess.Popen([sys.executable, "-B", "-c", code, str(ROOT / "backend")],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, creationflags=subprocess.CREATE_NO_WINDOW)
        from ctypes import wintypes as w
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.restype = w.HANDLE
        kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
        kernel.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
        kernel.CloseHandle.argtypes = [w.HANDLE]
        handle = None
        try:
            line = process.stdout.readline()
            self.assertTrue(line, process.stderr.read() if process.poll() is not None else "no child PID")
            handle = kernel.OpenProcess(0x100000, False, int(line))
            self.assertTrue(handle)
            process.terminate(); process.wait(timeout=10)
            self.assertEqual(0, kernel.WaitForSingleObject(handle, 5000), "native model process survived backend exit")
        finally:
            if process.poll() is None: process.terminate(); process.wait(timeout=10)
            if handle: kernel.CloseHandle(handle)
            process.stdin.close(); process.stdout.close(); process.stderr.close()


if __name__ == "__main__": unittest.main()
