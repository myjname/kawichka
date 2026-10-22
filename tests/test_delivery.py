import sys
import tempfile
import sqlite3
import subprocess
from contextlib import closing
import threading
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.store import Store, cache_key
from kavychka_local.delivery import STYLES, fixed, select, catalog
from kavychka_local.engine import Engine


class DeliveryTests(unittest.TestCase):
    def test_auto_direction_for_builtin_text_does_not_require_a_text_model(self):
        engine = Engine(ROOT, self.store, None)
        engine.load = lambda *args, **kwargs: self.fail("A text model must not load for ready-made text")
        choices = []
        for i in range(4):
            delivery = engine.direction("Встань и разомни плечи.", "break", threading.Event(), lambda *unused: None, use_text_model=False)
            choices.append(delivery["voice_style"])
            self.store.add("Ready-made reminder %d." % i, "break", "tts-cosyvoice3", "voice", delivery=delivery)
        self.assertGreater(len(set(choices)), 1)
        self.assertTrue(set(choices) <= {"confident", "calm", "amused"})
    def test_editable_catalog_defaults_add_remove_restart_and_validation(self):
        settings = self.store.settings()
        self.assertEqual("auto", settings["cosy_style"])
        self.assertEqual({"confident", "calm", "amused"}, set(catalog(settings)))
        custom = {"id": "user_test", "name": "Шёпотом", "instruction": "Speak in a quiet whisper."}
        entries = settings["delivery_catalog"] + [custom]
        self.store.configure({"delivery_catalog": entries, "cosy_style": "user_test"})
        self.assertEqual(custom["instruction"], fixed(self.store.settings())["voice_instruction"])
        self.store.db.close(); self.store = Store(self.temp.name)
        self.assertEqual("user_test", self.store.settings()["cosy_style"])
        self.assertIn("user_test", catalog(self.store.settings()))
        for invalid in ([], entries + [custom], [dict(custom, instruction="")], [dict(custom, instruction="<|bad|>")], [dict(custom, id="auto")]):
            with self.assertRaises(ValueError):
                self.store.configure({"delivery_catalog": invalid, "cosy_style": "auto"})
        self.store.configure({"delivery_catalog": [custom], "cosy_style": "auto"})
        self.assertEqual("user_test", select("invalid", "break", ["user_test", "user_test"], styles=catalog(self.store.settings()))["voice_style"])

    def test_auto_can_select_custom_delivery_and_never_deleted_one(self):
        styles = {"calm": STYLES["calm"], "user_whisper": "Speak in a whisper."}
        self.assertEqual("user_whisper", select("user_whisper", "break", styles=styles)["voice_style"])
        self.assertIn(select("warm", "dinner", styles=styles)["voice_style"], styles)

    def test_reference_rotation_persists_and_resynthesis_reuses_reference(self):
        names = ("reference-a.wav", "reference-b.wav")
        first = self.store.add("Отложи дела на минуту.", "break", "tts-cosyvoice3", "voice")
        second = self.store.add("Подойди к окну и выдохни.", "break", "tts-cosyvoice3", "voice")
        self.assertEqual(names[0], self.store.assign_reference(first["id"], names))
        self.assertEqual(names[0], self.store.assign_reference(first["id"], names))
        self.store.db.close(); self.store = Store(self.temp.name)
        self.assertEqual(names[1], self.store.assign_reference(second["id"], names))
        third = self.store.add("Разомни плечи, я подожду.", "break", "tts-cosyvoice3", "voice")
        self.assertEqual(names[0], self.store.assign_reference(third["id"], names))
        self.assertEqual(names[1], self.store.get(second["id"])["voice_reference"])

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.store = Store(self.temp.name)
    def tearDown(self):
        self.store.db.close(); self.temp.cleanup()

    def test_custom_instruction_validation_and_persistence(self):
        self.assertEqual("tts-cosyvoice3", self.store.settings()["tts_model"])
        self.store.configure({"cosy_style": "custom", "cosy_instruction": "Speak softly and slowly."})
        for value in ("", "  ", "<|endofprompt|>", "x" * 601):
            with self.assertRaises(ValueError): self.store.configure({"cosy_instruction": value})
        self.store.db.close(); self.store = Store(self.temp.name)
        self.assertEqual("Speak softly and slowly.", fixed(self.store.settings())["voice_instruction"])

    def test_intonation_changes_cache_and_archive_metadata_survives_restart(self):
        base = ("Оставь работу, немного отдохни.", "tts-cosyvoice3", "reference", "revision")
        self.assertNotEqual(cache_key(*base, direction=STYLES["calm"]), cache_key(*base, direction=STYLES["warm"]))
        row = self.store.add(base[0], "break", base[1], "reference", delivery=select("warm", "break"))
        self.store.db.close(); self.store = Store(self.temp.name)
        self.assertEqual("warm", self.store.get(row["id"])["voice_style"])
        name = row["id"] + ".flac"; (self.store.root / "audio" / name).write_bytes(b"test")
        self.store.audio(row["id"], "tts-0.6b", "key", name)
        self.assertEqual("", self.store.get(row["id"])["voice_instruction"])

    def test_auto_direction_uses_text_model_and_fixed_mode_does_not(self):
        self.store.configure({"cosy_style": "auto"})
        engine = Engine(ROOT, self.store, None); calls = []
        class Model:
            def generate(self, messages, event, max_tokens):
                calls.append((messages, max_tokens)); return "calm"
        def load(model, *unused): calls.append(model); engine.model = Model()
        engine.load = load
        result = engine.direction("Иди поужинай.", "dinner", threading.Event(), lambda *a: None, "text-gemma4-12b")
        self.assertEqual("calm", result["voice_style"])
        self.assertEqual("text-gemma4-12b", calls[0]); self.assertEqual(96, calls[1][1])
        calls.clear(); self.store.configure({"cosy_style": "calm"})
        self.assertEqual("calm", engine.direction("Встань.", "break", threading.Event(), lambda *a: None)["voice_style"])
        self.assertFalse(calls)

    def test_auto_invalid_reply_has_a_bounded_category_fallback(self):
        self.assertEqual("warm", select("not a style", "lunch")["voice_style"])
        self.assertEqual("confident", select("", "exercise")["voice_style"])

    def test_auto_cannot_repeat_one_delivery_three_times(self):
        recent = []
        for _ in range(12):
            style = select("confident", "break", recent, "Встань и разомнись.")["voice_style"]
            self.assertFalse(len(recent) >= 2 and style == recent[0] == recent[1])
            recent.insert(0, style)
        self.assertGreater(len(set(recent)), 1)

    def test_auto_ambiguous_reply_uses_context_and_recent_history(self):
        self.assertEqual("calm", select("calm, amused, warm, confident", "break", (), "Выдохни и отдохни.")["voice_style"])
        self.assertNotEqual("calm", select("calm", "break", ["calm", "calm"], "Выдохни.")["voice_style"])

    def test_relocation_verifies_database_and_remaps_only_internal_model_paths(self):
        internal = self.store.root / "models/custom"
        external = ROOT / "work/external-model"
        self.store.configure({"model_paths": {"text-gemma4-12b": str(internal), "tts-0.6b": str(external)}})
        row = self.store.add("Отложи работу и дай себе отдохнуть.", "break", "tts-cosyvoice3", "voice", delivery=select("warm", "break"))
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as folder:
            target = Path(folder)
            with closing(sqlite3.connect(target / "library.sqlite3")) as copied:
                self.store.db.backup(copied)
            subprocess.run([sys.executable, "-B", str(ROOT / "backend/relocate_storage.py"), "--source", str(self.store.root), "--target", str(target)], check=True, capture_output=True)
            moved = Store(target)
            try:
                self.assertEqual(str(target / "models/custom"), moved.settings()["model_paths"]["text-gemma4-12b"])
                self.assertEqual(str(external), moved.settings()["model_paths"]["tts-0.6b"])
                self.assertEqual("warm", moved.get(row["id"])["voice_style"])
                self.assertEqual(str(internal), self.store.settings()["model_paths"]["text-gemma4-12b"])
            finally: moved.db.close()
