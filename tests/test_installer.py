import ctypes
import json
import os
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.downloads import BUILD_TOOLS, Cancelled, Components


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.root = Path(self.temp.name)
        self.components = Components(ROOT, self.root, None)
        self.event = threading.Event()
        self.progress = lambda *args: None

    def tearDown(self):
        self.temp.cleanup()

    def test_runtime_build_tools_precede_source_dependencies_and_marker_follows_check(self):
        calls = []
        def run(command, *args):
            self.assertFalse((self.root / "runtime-ok.json").exists())
            calls.append(command)
            return "2.9.1"
        with patch("kavychka_local.downloads.download"), patch.object(self.components, "run_install", side_effect=run):
            self.components.install_runtime(self.event, self.progress, True)
        self.assertEqual(5, len(calls))
        self.assertTrue(set(BUILD_TOOLS) <= set(calls[1]))
        self.assertIn("--only-binary=:all:", calls[1])
        self.assertIn("--no-build-isolation", calls[3])
        self.assertIn("requirements.txt", calls[3][-1])
        self.assertIn("import torch,torchaudio", calls[4][-1])
        self.assertEqual("2.9.1", json.loads((self.root / "runtime-ok.json").read_text())["torch"])

    def test_failed_reinstall_removes_old_runtime_marker(self):
        (self.root / "runtime-ok.json").write_text("{}")
        with patch("kavychka_local.downloads.download"), patch.object(self.components, "run_install", side_effect=RuntimeError("build failed")):
            with self.assertRaisesRegex(RuntimeError, "build failed"):
                self.components.install_runtime(self.event, self.progress, False)
        self.assertFalse((self.root / "runtime-ok.json").exists())

    def test_cosy_prepares_build_tools_before_no_isolation_install_and_check(self):
        self.components.manifest["cosy_runtime"]["files"] = []
        calls = []
        def run(command, env, event, progress, log_name):
            self.assertFalse((self.root / "cosy/verified.json").exists())
            self.assertEqual("cosy-install.log", log_name)
            calls.append(command)
        with patch.object(self.components, "run_install", side_effect=run), patch.object(self.components, "cosy_ready", return_value=True):
            self.components.install_cosy(self.event, self.progress)
        self.assertEqual(3, len(calls))
        self.assertTrue(set(BUILD_TOOLS) <= set(calls[0]))
        self.assertIn("--only-binary=:all:", calls[0])
        self.assertIn("--no-build-isolation", calls[1])
        self.assertIn("--no-deps", calls[1])
        self.assertIn("--target", calls[1])
        self.assertIn("--check-only", calls[2])
        self.assertTrue((self.root / "cosy/verified.json").exists())

    def test_failed_cosy_check_has_no_ready_marker(self):
        self.components.manifest["cosy_runtime"]["files"] = []
        folder = self.root / "cosy"
        folder.mkdir(); (folder / "verified.json").write_text("{}")
        with patch.object(self.components, "run_install", side_effect=["", "", RuntimeError("import failed")]):
            with self.assertRaisesRegex(RuntimeError, "import failed"):
                self.components.install_cosy(self.event, self.progress)
        self.assertFalse((folder / "verified.json").exists())

    def test_incomplete_cosy_files_do_not_leave_ready_marker(self):
        self.components.manifest["cosy_runtime"]["files"] = []
        with patch.object(self.components, "run_install", return_value=""), patch.object(self.components, "cosy_ready", return_value=False):
            with self.assertRaisesRegex(ValueError, "неполна"):
                self.components.install_cosy(self.event, self.progress)
        self.assertFalse((self.root / "cosy/verified.json").exists())

    def test_complete_utf8_log_appends_and_error_contains_tail_and_log_path(self):
        command = [sys.executable, "-c", "import sys; print('ранняя причина'); [print('line', i) for i in range(30)]; print('финальная ошибка', file=sys.stderr); sys.exit(7)"]
        for _ in range(2):
            with self.assertRaises(RuntimeError) as error:
                self.components.run_install(command, os.environ, self.event, self.progress)
            self.assertIn("финальная ошибка", str(error.exception))
            self.assertIn(str(self.root / "logs/runtime-install.log"), str(error.exception))
        log = (self.root / "logs/runtime-install.log").read_text(encoding="utf-8")
        self.assertEqual(2, log.count("Exit code: 7"))
        self.assertIn("ранняя причина\n", log)
        self.assertIn("line 0\n", log)
        self.assertIn("line 29\n", log)

    def test_cancelled_before_command_does_not_start_process(self):
        self.event.set()
        with patch("kavychka_local.downloads.subprocess.Popen") as popen:
            with self.assertRaises(Cancelled):
                self.components.run_install([sys.executable], os.environ, self.event, self.progress)
            popen.assert_not_called()

    @unittest.skipUnless(os.name == "nt", "Windows process tree")
    def test_cancellation_stops_installer_child_process(self):
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handles = []
        def progress(line, *unused):
            if line.startswith("child:"):
                handle = kernel.OpenProcess(0x100000, False, int(line.split(":")[1]))
                self.assertTrue(handle)
                handles.append(handle)
                self.event.set()
        code = "import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); print('child:'+str(p.pid),flush=True); time.sleep(60)"
        try:
            with self.assertRaises(Cancelled):
                self.components.run_install([sys.executable, "-c", code], os.environ, self.event, progress)
            self.assertEqual(1, len(handles))
            self.assertEqual(0, kernel.WaitForSingleObject(handles[0], 5000))
        finally:
            for handle in handles: kernel.CloseHandle(handle)


if __name__ == "__main__":
    unittest.main()
