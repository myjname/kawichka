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
from kavychka_local.downloads import BUILD_TOOLS, SITE_OWNER, Cancelled, Components


class InstallerTests(unittest.TestCase):
    def setUp(self):
        # Production installs run in a fresh worker. Earlier attention tests can
        # load Torch into this unittest process; isolate that precondition here.
        modules = patch.dict(sys.modules)
        modules.start(); self.addCleanup(modules.stop)
        sys.modules.pop("torch", None)
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.root = Path(self.temp.name)
        self.components = Components(ROOT, self.root, None)
        self.event = threading.Event()
        self.progress = lambda *args: None
        stub = self.root / "inference.whl"; stub.write_bytes(b"wheel fixture")
        self.components.manifest["qwen_inference"].pop("derived_sha256", None)
        for fixture in (patch.object(self.components, "runtime_python", return_value=sys.executable),
                        patch("kavychka_local.qwen_package.build", return_value=stub)):
            fixture.start(); self.addCleanup(fixture.stop)

    def test_loaded_torch_refuses_install_before_touching_the_environment(self):
        marker = self.root / "runtime-ok.json"
        marker.write_text("retained marker")
        with patch.dict(sys.modules, {"torch": object()}), patch.object(self.components, "run_install") as run:
            with self.assertRaisesRegex(ValueError, "перезапустите"):
                self.components.install_runtime(self.event, self.progress, False)
        run.assert_not_called()
        self.assertEqual(marker.read_text(), "retained marker")

    def tearDown(self):
        self.temp.cleanup()

    def test_runtime_build_tools_precede_source_dependencies_and_marker_follows_check(self):
        calls = []
        def run(command, *args):
            self.assertFalse((self.root / "runtime-ok.json").exists())
            calls.append(command)
            return json.dumps(dict(torch="2.14.1", transformers="5.10.0"))
        with patch("kavychka_local.downloads.download"), patch.object(self.components, "run_install", side_effect=run):
            self.components.install_runtime(self.event, self.progress, True)
        self.assertEqual(7, len(calls))
        self.assertTrue(set(BUILD_TOOLS) <= set(calls[1]))
        self.assertIn("--only-binary=:all:", calls[1])
        self.assertIn("uninstall", calls[2])
        self.assertIn("gradio", calls[2])
        self.assertIn("--no-build-isolation", calls[4])
        self.assertIn("requirements.txt", calls[4][-1])
        self.assertEqual("check", calls[5][-1])
        self.assertTrue(any(str(arg).endswith("health_check.py") for arg in calls[6]))
        self.assertIn("--inference-runtime", calls[6])
        marker = json.loads((self.root / "runtime-ok.json").read_text())
        self.assertEqual("2.14.1", marker["torch"])
        self.assertEqual(self.components.manifest["runtime_profile"], marker["profile"])

    def test_foreign_python_is_refused_without_modifying_an_environment(self):
        with self.assertRaisesRegex(ValueError, "original environment retained"):
            Components.runtime_python(self.components)

    @unittest.skipUnless(os.name == "nt", "Windows extended paths")
    def test_managed_python_accepts_extended_and_plain_paths_in_both_directions(self):
        managed = self.root / "Среда Python"
        runtime = managed / "runtime"
        runtime.mkdir(parents=True)
        actual = runtime / "python.exe"
        actual.write_bytes(b"interpreter identity fixture")
        extended = "\\\\?\\" + str(actual)
        for executable, root in ((extended, managed),
                                 (str(actual), Path("\\\\?\\" + str(managed)))):
            with self.subTest(executable=executable), patch.object(sys, "executable", executable):
                component = Components(ROOT, root, None)
                self.assertEqual(Components.runtime_python(component), str(actual.resolve()))
                self.assertEqual(actual.read_bytes(), b"interpreter identity fixture")

    def test_mismatched_derived_wheel_starts_no_installation(self):
        self.components.manifest["qwen_inference"]["derived_sha256"] = "0" * 64
        with patch("kavychka_local.downloads.download"), patch.object(self.components, "run_install") as run:
            with self.assertRaisesRegex(ValueError, "Derived inference wheel"):
                self.components.install_runtime(self.event, self.progress, False)
            run.assert_not_called()

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

    def test_cosy_replaces_the_layer_without_retaining_old_transformers(self):
        self.components.manifest["cosy_runtime"]["files"] = []
        old = self.root / "cosy/site/transformers"; old.mkdir(parents=True)
        (old / "stale.py").write_text("old layer")
        def install(command, *unused):
            if "--target" in command:
                stage = Path(command[command.index("--target") + 1])
                (stage / "new-package.txt").write_text("new layer")
        with patch.object(self.components, "run_install", side_effect=install), patch.object(self.components, "cosy_ready", return_value=True):
            self.components.install_cosy(self.event, self.progress)
        self.assertFalse(old.exists())
        self.assertEqual("new layer", (self.root / "cosy/site/new-package.txt").read_text())
        self.assertFalse(list((self.root / "cosy").glob("site-*-*")))

    def test_failed_staged_cosy_check_keeps_the_old_layer_and_cleans_stage(self):
        self.components.manifest["cosy_runtime"]["files"] = []
        old = self.root / "cosy/site/transformers"; old.mkdir(parents=True)
        (old / "stale.py").write_text("old layer")
        with patch.object(self.components, "run_install", side_effect=["", "", RuntimeError("check failed")]):
            with self.assertRaisesRegex(RuntimeError, "check failed"):
                self.components.install_cosy(self.event, self.progress)
        self.assertEqual("old layer", (old / "stale.py").read_text())
        self.assertFalse(list((self.root / "cosy").glob("site-install-*")))

    def test_retry_cleans_only_marker_owned_abandoned_stages(self):
        parent = self.root / "cosy"; parent.mkdir()
        owned = parent / "site-install-owned"; owned.mkdir()
        (owned / ".kavychka-site.owner").write_bytes(SITE_OWNER)
        (owned / "partial-package.txt").write_text("interrupted installation")
        unknown = parent / "site-install-unknown"; unknown.mkdir()
        (unknown / "retain.txt").write_text("not owned")
        self.components.clean_old_sites(parent, "site-install-*")
        self.assertFalse(owned.exists())
        self.assertEqual("not owned", (unknown / "retain.txt").read_text())

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
