import ctypes
import json
import os
import sys
import tempfile
import threading
import unittest
import zipfile
from pathlib import Path
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local import hardware
from kavychka_local.downloads import Cancelled, Components
from kavychka_local.store import Store


class HardwareTests(unittest.TestCase):
    def test_driver_and_architecture_decide_each_runtime_independently(self):
        for version, compute, speech, text in (
                (0, 0, False, False), (12040, 89, False, False),
                (12080, 89, True, False), (13000, 75, True, True),
                (13040, 61, False, False), (13040, 89, True, True)):
            with self.subTest(version=version, compute=compute):
                with patch.object(hardware, "memory_bytes", return_value=8_000_000_000), patch.object(
                        hardware, "cuda_device", return_value=dict(cuda_version=version, compute=compute)):
                    result = hardware.probe()
                self.assertEqual((speech, text), (result["speech_gpu"], result["text_gpu"]))
                self.assertEqual(8_000_000_000, result["ram_bytes"])

    def test_driver_probe_reads_64bit_memory_without_importing_torch(self):
        def put(pointer, value):
            getattr(pointer, "_obj", pointer).value = value
            return 0
        driver = Mock()
        driver.cuDriverGetVersion.side_effect = lambda p: put(p, 13040)
        driver.cuInit.return_value = 0
        driver.cuDeviceGetCount.side_effect = lambda p: put(p, 1)
        driver.cuDeviceGet.side_effect = lambda p, index: put(p, index)
        driver.cuDeviceGetName.side_effect = lambda p, size, dev: put(p, b"Test NVIDIA")
        driver.cuDeviceTotalMem_v2.side_effect = lambda p, dev: put(p, 16 * 1024 ** 3)
        driver.cuDeviceGetAttribute.side_effect = lambda p, attr, dev: put(p, 8 if attr == 75 else 9)
        loader = "WinDLL" if os.name == "nt" else "CDLL"
        before = "torch" in sys.modules
        with patch.object(ctypes, loader, return_value=driver):
            result = hardware.cuda_device()
            self.assertEqual(16 * 1024 ** 3, result["vram_bytes"])
            self.assertEqual("Test NVIDIA", result["gpu_name"])
            driver.cuInit.return_value = 100
            self.assertEqual({}, hardware.cuda_device())
        self.assertEqual(before, "torch" in sys.modules)
        with patch.object(ctypes, loader, side_effect=OSError("no driver")):
            self.assertEqual({}, hardware.cuda_device())


class CpuInstallationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.root = Path(self.temp.name)
        self.store = Store(self.root)
        self.components = Components(ROOT, self.root, self.store)
        self.components._hardware = dict(ram_bytes=16_000_000_000, cuda_version=0, gpu_name="",
                                         vram_bytes=0, compute=0, speech_gpu=False, text_gpu=False)
        self.event = threading.Event()

    def tearDown(self):
        self.store.db.close()
        self.temp.cleanup()

    def fake_archive(self, url, target, size, sha, event, progress):
        target.parent.mkdir(parents=True, exist_ok=True)
        required = ["llama-server.exe", "ggml-base.dll", "ggml-cpu-x64.dll", "ggml.dll",
                    "llama.dll", "llama-common.dll", "llama-server-impl.dll", "libomp.dll"]
        if "cuda" in url:
            required += ["ggml-cuda.dll", "cublas64_13.dll", "cublasLt64_13.dll"]
        with zipfile.ZipFile(target, "w") as archive:
            for name in required:
                archive.writestr(name, b"test binary")

    def test_fresh_cpu_install_uses_only_small_cpu_archive_and_restarts_offline(self):
        with patch("kavychka_local.downloads.download", side_effect=self.fake_archive) as fetch:
            self.components.install("llama-runtime", self.event, lambda *a: None)
            self.assertEqual(1, fetch.call_count)
            self.assertEqual(self.components.manifest["llama_cpu_runtime"]["files"][0]["url"], fetch.call_args.args[0])
            self.assertLess(fetch.call_args.args[2], 20_000_000)
            self.assertTrue(self.components.llama_ready())
            self.assertFalse(self.components.llama_ready(gpu=True))
            restarted = Components(ROOT, self.root, self.store)
            restarted._hardware = self.components._hardware
            self.assertEqual("cpu", restarted.llama_device())
            self.assertEqual("cpu", restarted.voice_device())
            self.assertTrue(restarted.install("llama-runtime", self.event, lambda *a: None)["reused"])
            self.assertEqual(1, fetch.call_count)
            (self.root / "llama/ggml-base.dll").unlink()
            self.assertFalse(restarted.llama_ready())
        self.assertFalse((self.root / "models").exists())

    def test_explicit_cpu_overrides_detected_nvidia_for_both_installs(self):
        self.components._hardware.update(speech_gpu=True, text_gpu=True)
        self.store.configure({"device": "cpu"})
        self.assertFalse(self.components.use_gpu("runtime"))
        self.assertFalse(self.components.use_gpu("llama-runtime"))
        with patch.object(self.components, "install_runtime") as install:
            self.components.install("runtime", self.event, lambda *a: None)
            self.assertIs(False, install.call_args.args[-1])

    def test_existing_cuda_runtime_is_reused_for_cpu_and_keeps_legacy_marker(self):
        with patch("kavychka_local.downloads.download", side_effect=self.fake_archive) as fetch:
            self.components.install("llama-runtime", self.event, lambda *a: None, gpu=True)
            calls = fetch.call_count
            marker = self.root / "llama/verified.json"
            marker.write_text(json.dumps({"revision": self.components.manifest["llama_runtime"]["revision"]}))
            self.assertTrue(self.components.llama_ready(gpu=True))
            self.assertTrue(self.components.install("llama-runtime", self.event, lambda *a: None, gpu=False)["reused"])
            self.assertEqual(calls, fetch.call_count)
        self.assertEqual("cpu", self.components.llama_device())

    def test_cancelled_or_incomplete_cpu_archive_never_counts_as_installed(self):
        def cancelled(*args):
            raise Cancelled("cancel")
        with patch("kavychka_local.downloads.download", side_effect=cancelled):
            with self.assertRaises(Cancelled):
                self.components.install("llama-runtime", self.event, lambda *a: None)
        self.assertFalse(self.components.llama_ready())
        def incomplete(url, target, *args):
            target.parent.mkdir(parents=True, exist_ok=True)
            with zipfile.ZipFile(target, "w") as archive:
                archive.writestr("llama-server.exe", b"partial")
        with patch("kavychka_local.downloads.download", side_effect=incomplete):
            with self.assertRaises(ValueError):
                self.components.install("llama-runtime", self.event, lambda *a: None)
        self.assertFalse((self.root / "llama/verified.json").exists())

    def test_runtime_status_requires_cuda_wheels_only_when_gpu_requested(self):
        (self.root / "runtime-ok.json").write_text(json.dumps({"torch": "2.9.1+cpu", "gpu": False}))
        with patch("importlib.util.find_spec", return_value=object()):
            self.assertTrue(self.components.status()[0]["installed"])
            self.components._hardware.update(speech_gpu=True)
            self.assertFalse(self.components.status()[0]["installed"])
            self.store.configure({"device": "cpu"})
            self.assertTrue(self.components.status()[0]["installed"])

    def test_invalid_install_markers_report_missing_components_without_blocking_repair(self):
        (self.root / "llama").mkdir()
        for invalid in ("[]", "null", "{bad-json", '{"gpu":"false"}'):
            with self.subTest(marker=invalid):
                (self.root / "runtime-ok.json").write_text(invalid)
                (self.root / "llama/verified.json").write_text(invalid)
                items = {item["id"]: item for item in self.components.status()}
                self.assertFalse(items["runtime"]["installed"])
                self.assertFalse(items["llama-runtime"]["installed"])


if __name__ == "__main__":
    unittest.main()
