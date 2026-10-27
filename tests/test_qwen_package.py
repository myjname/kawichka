"""The downstream wheel is a real distribution, not edited installed metadata."""
import base64
import csv
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local import qwen_package


class InferencePackageTests(unittest.TestCase):
    def test_declared_runtime_versions_match_the_installer_requirements(self):
        manifest = json.loads((ROOT / "assets/components.json").read_text(encoding="utf-8"))
        pins = {name.lower().replace("_", "-"): version for line in (ROOT / "backend/requirements.txt").read_text().splitlines() if "==" in line for name, version in [line.split("==", 1)]}
        for name, version in manifest["runtime_versions"].items():
            self.assertEqual(version, pins[name])
        self.assertEqual(qwen_package.VERSION, manifest["qwen_inference"]["version"])
        self.assertFalse({"qwen-tts", "gradio", "gradio-client", "hf-gradio"} & set(pins))

    def test_repack_is_reproducible_licensed_and_has_valid_record_without_demo(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as temporary:
            root = Path(temporary); source = root / "upstream.whl"
            with zipfile.ZipFile(source, "w") as archive:
                archive.writestr("qwen_tts/__init__.py", "# upstream attribution\n")
                archive.writestr("qwen_tts/inference/qwen3_tts_model.py", "# inference\n")
                archive.writestr("qwen_tts/cli/demo.py", "import gradio\n")
                archive.writestr("qwen_tts/__main__.py", "# demo entrypoint\n")
                archive.writestr("qwen_tts-0.1.1.dist-info/licenses/LICENSE", "Upstream Apache-2.0 license")
            with patch.object(qwen_package, "SOURCE_SHA256", hashlib.sha256(source.read_bytes()).hexdigest()):
                one = qwen_package.build(source, root / "one")
                two = qwen_package.build(source, root / "two")
            self.assertEqual(one.read_bytes(), two.read_bytes())
            with zipfile.ZipFile(one) as archive:
                self.assertFalse(any("/cli/" in n or n.endswith("__main__.py") for n in archive.namelist()))
                metadata = archive.read(f"{qwen_package.INFO}/METADATA").decode()
                self.assertIn("Name: kavychka-qwen-tts-inference", metadata)
                self.assertIn("Requires-Dist: transformers==5.10.0", metadata)
                self.assertNotIn("Requires-Dist: gradio", metadata)
                self.assertEqual(b"Upstream Apache-2.0 license", archive.read(f"{qwen_package.INFO}/licenses/LICENSE"))
                rows = csv.reader(io.StringIO(archive.read(f"{qwen_package.INFO}/RECORD").decode()))
                recorded = set()
                for name, digest, length in rows:
                    recorded.add(name)
                    if name.endswith("/RECORD"):
                        self.assertEqual((digest, length), ("", "")); continue
                    data = archive.read(name)
                    self.assertEqual(len(data), int(length))
                    value = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
                    self.assertEqual("sha256=" + value, digest)
                self.assertEqual(set(archive.namelist()), recorded)

    def test_changed_upstream_is_rejected_before_creating_outputs(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as temporary:
            root = Path(temporary); source = root / "untrusted.whl"
            source.write_bytes(b"a different source")
            with self.assertRaisesRegex(ValueError, "integrity"):
                qwen_package.build(source, root / "output")
            self.assertFalse((root / "output").exists())

    def test_changed_patch_context_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Unexpected upstream"):
            qwen_package.adapt("qwen_tts/core/models/modeling_qwen3_tts.py", b"modified upstream source")
