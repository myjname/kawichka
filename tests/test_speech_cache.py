import sys
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.engine import Engine
from kavychka_local.store import cache_key


class SpeechCacheTests(unittest.TestCase):
    def test_revoice_bypasses_old_broken_cache_then_reuses_fixed_recording(self):
        import numpy as np
        import soundfile as sf
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            root = Path(directory)
            (root / "audio").mkdir()
            row = dict(id="fixture", text="Встань и немного разомнись.", category="exercise", seed=42,
                       voice_style="calm", voice_instruction="Speak calmly.")
            old_key = cache_key(row["text"], "tts-cosyvoice3", "reference", "revision", 42,
                                "cosy-instruct2-v1:" + row["voice_instruction"], language="ru")
            old = root / "audio" / (old_key + ".flac")
            old.write_bytes(b"retained old recording")
            engine = Engine.__new__(Engine)
            engine.profile_hash = "profile"
            engine.reference = root / "reference.wav"
            engine.references = {"reference": {"path": engine.reference}}
            engine.refresh_voice = lambda: None
            engine.components = SimpleNamespace(manifest={"models": {"tts-cosyvoice3": {"revision": "revision"}}})
            engine.store = SimpleNamespace(root=root,
                settings=lambda: dict(cosy_style="auto", speech_language="ru"),
                assign_reference=lambda *unused: "reference",
                audio=lambda ident, model, key, filename, seed, delivery: dict(audio_path=filename))
            def generate(text, instruction, reference, target, seed, event):
                sf.write(target, np.sin(np.arange(12000) * .05).astype(np.float32) * .1, 24000)
            engine.model = SimpleNamespace(generate=Mock(side_effect=generate))
            engine.load = Mock()
            event = threading.Event()
            first = engine.speech(row, "tts-cosyvoice3", event, lambda *unused: None)
            self.assertNotEqual(first["audio_path"], old.name)
            self.assertTrue((root / "audio" / first["audio_path"]).is_file())
            second = engine.speech(row, "tts-cosyvoice3", event, lambda *unused: None)
            self.assertEqual(second, first)
            engine.model.generate.assert_called_once()
            self.assertEqual(old.read_bytes(), b"retained old recording")
