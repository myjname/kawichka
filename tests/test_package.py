"""Inspect the actual distribution, including outputs left by development tests."""
import hashlib
import json
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "outputs/Kavychka-portable.zip"


@unittest.skipUnless(PACKAGE.exists(), "Build the distributable with scripts/build.ps1 first")
class PackageTests(unittest.TestCase):
    def test_archive_matches_manifest_and_hashes(self):
        with zipfile.ZipFile(PACKAGE) as archive:
            manifest = json.loads(archive.read("package.json").decode("utf-8-sig"))
            expected = {row["path"] for row in manifest["files"]}
            files = {row.filename for row in archive.infolist() if not row.is_dir()}
            self.assertEqual(expected | {"package.json"}, files)
            for row in manifest["files"]:
                payload = archive.read(row["path"])
                self.assertEqual(row["bytes"], len(payload))
                self.assertEqual(row["sha256"], hashlib.sha256(payload).hexdigest())

    def test_default_assets_are_generic(self):
        with zipfile.ZipFile(PACKAGE) as archive:
            for name in ("assets/character.json", "assets/voice/profile.json"):
                item = json.loads(archive.read(name).decode("utf-8-sig"))
                self.assertTrue(item["public_default"])
            voice = json.loads(archive.read("assets/voice/profile.json").decode("utf-8-sig"))
            self.assertEqual("default-en.wav", voice["file"])

    def test_private_files_and_runtime_are_excluded(self):
        with zipfile.ZipFile(PACKAGE) as archive:
            for name in archive.namelist():
                path = Path(name)
                self.assertFalse(set(path.parts) & {"data", "models", "runtime", "user-legacy", "presets", "__pycache__", "work", ".git"}, name)
                self.assertNotIn(path.suffix.lower(), {".pyc", ".gguf", ".safetensors", ".sqlite3", ".flac", ".pt", ".onnx"}, name)
                if path.suffix.lower() == ".wav":
                    self.assertEqual("assets/voice/default-en.wav", name)

    def test_development_executables_are_excluded(self):
        with zipfile.ZipFile(PACKAGE) as archive:
            executables = [name for name in archive.namelist() if name.lower().endswith(".exe")]
            self.assertEqual(["Kavychka.exe"], executables)


if __name__ == "__main__":
    unittest.main()
