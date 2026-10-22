import importlib.util
import json
import sqlite3
import sys
import tempfile
import threading
import unittest
import zipfile
from concurrent.futures import ThreadPoolExecutor
from types import SimpleNamespace
from unittest.mock import Mock, patch
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.store import Store
from kavychka_local.jobs import Jobs
from kavychka_local.profiles import Profiles
from kavychka_local.character import messages


class FakeEngine:
    profile_hash = "neutral-reference"
    def __init__(self, store):
        self.store, self.text_calls, self.audio_calls, self.fail_audio = store, [], [], False
        self.text_models = []
        self.direction_options = []
    def text(self, category, event, progress, followup=False, model=None):
        self.text_calls.append((category, followup))
        self.text_models.append(model)
        return "Distinct reminder number %d to stand up and stretch." % len(self.text_calls)
    def direction(self, *unused, **kwargs):
        self.direction_options.append(kwargs)
        return {"voice_style": "calm", "voice_instruction": "Speak calmly."}
    def speech(self, row, model, event, progress):
        self.audio_calls.append(row["id"])
        if self.fail_audio:
            raise RuntimeError("temporary synthesis failure")
        name = row["id"] + ".flac"
        (self.store.root / "audio" / name).write_bytes(b"audio")
        return self.store.audio(row["id"], model, row["id"], name)


class PreparationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.store = Store(self.temp.name)
        self.engine = FakeEngine(self.store)
        self.jobs = Jobs(self.store, None, self.engine)
        self.args = {"day": "2026-10-22", "tasks": [{"slot": "10:00", "category": "break", "followup": False}, {"slot": "10:05", "category": "break", "followup": True}]}
    def tearDown(self):
        self.store.db.close(); self.temp.cleanup()
    def prepare(self, **values):
        args = dict(self.args, **values)
        return self.jobs.prepare(args, threading.Event(), lambda *unused: None)
    def test_partial_audio_failure_keeps_text_and_resumes_after_restart(self):
        self.engine.fail_audio = True
        first = self.prepare()
        self.assertEqual(0, first["ready"])
        self.assertEqual(2, len(self.store.rows()))
        self.assertEqual(2, len(self.engine.text_calls))
        self.prepare()  # Backoff prevents a hot retry loop.
        self.assertEqual(2, len(self.engine.audio_calls))
        self.store.db.close(); self.store = Store(self.temp.name)
        self.engine = FakeEngine(self.store); self.jobs = Jobs(self.store, None, self.engine)
        result = self.prepare(retry=True)
        self.assertEqual(2, result["ready"])
        self.assertEqual([], self.engine.text_calls)
        self.assertEqual(2, len(self.store.rows()))
        self.assertEqual(2, self.prepare()["ready"])
        self.assertEqual(2, len(self.engine.audio_calls))
    def test_followups_language_and_one_time_consume_are_independent(self):
        self.store.configure({"speech_language": "en"})
        self.prepare()
        self.assertIsNone(self.store.consume("break", self.args["day"], False, "ru"))
        first = self.store.consume("break", self.args["day"], False, "en")
        self.assertFalse(first["followup"])
        second = self.store.consume("break", self.args["day"], True, "en")
        self.assertTrue(second["followup"])
        self.assertIsNone(self.store.consume("break", self.args["day"], True, "en"))
    def test_ready_made_text_with_system_speech_needs_no_model(self):
        for task in self.args["tasks"]:
            task["text"] = "Stand up and take a short break."
        result = self.prepare(system_text=True, system_voice=True)
        self.assertEqual(2, result["ready"])
        self.assertEqual([], self.engine.text_calls)
        self.assertEqual([], self.engine.audio_calls)
        self.assertIsNotNone(self.store.consume("break", self.args["day"], allow_text=True))
        self.assertTrue(self.store.known(self.args["tasks"][0]["text"]))
    def test_invalid_plan_does_not_write_tasks(self):
        with self.assertRaises(ValueError):
            self.prepare(tasks=[{"slot": "bad", "category": "unknown"}])
        self.assertEqual(0, self.store.db.execute("select count(*) from preparation").fetchone()[0])
    def test_clear_library_clears_preparation_without_resetting_preferences(self):
        self.prepare(); self.store.configure({"speech_language": "en"})
        self.store.clear_library()
        self.assertEqual(0, self.store.db.execute("select count(*) from preparation").fetchone()[0])
        self.assertEqual("en", self.store.settings()["speech_language"])
    def test_missing_ready_audio_is_repaired_without_rewriting_text(self):
        self.prepare()
        row = self.store.rows()[0]
        (self.store.root / "audio" / row["audio_path"]).unlink()
        self.assertEqual(2, self.prepare()["ready"])
        self.assertEqual(2, len(self.engine.text_calls))
        self.assertEqual(3, len(self.engine.audio_calls))

    def test_disabled_automatic_plan_does_not_generate_but_manual_preparation_works(self):
        self.store.configure({"auto_daily": False})
        self.assertTrue(self.prepare(automatic=True)["disabled"])
        self.assertEqual([], self.store.rows())
        self.assertEqual(0, self.store.db.execute("select count(*) from preparation").fetchone()[0])
        self.assertEqual(2, self.prepare(automatic=False)["ready"])

    def test_manual_review_does_not_regenerate_candidates_after_restart(self):
        self.store.configure({"daily_auto_keep": False})
        self.assertEqual(2, self.prepare()["ready"])
        original = {row["id"] for row in self.store.rows()}
        self.assertTrue(all(not row["approved"] and not row["rejected"] for row in self.store.rows()))
        self.assertIsNone(self.store.consume("break", self.args["day"]))
        self.store.db.close(); self.store = Store(self.temp.name)
        self.engine = FakeEngine(self.store); self.jobs = Jobs(self.store, None, self.engine)
        self.assertEqual(2, self.prepare()["ready"])
        self.assertEqual(original, {row["id"] for row in self.store.rows()})
        self.assertEqual([], self.engine.text_calls)
        self.assertEqual([], self.engine.audio_calls)
        regular = next(row for row in self.store.rows() if not row["followup"])
        self.store.select([regular["id"]], "keep")
        self.assertEqual(regular["id"], self.store.consume("break", self.args["day"])["id"])

    def test_explicitly_rejected_candidate_gets_one_replacement_then_waits_for_review(self):
        self.store.configure({"daily_auto_keep": False})
        self.prepare()
        rejected = self.store.rows()[0]
        self.store.select([rejected["id"]], "exclude")
        self.assertTrue(self.store.get(rejected["id"])["rejected"])
        self.assertEqual(2, self.prepare()["ready"])
        self.assertEqual(3, len(self.engine.text_calls))
        self.prepare()
        self.assertEqual(3, len(self.engine.text_calls))
        self.assertEqual(3, len(self.store.rows()))
        self.store.select([rejected["id"]], "keep")
        self.assertFalse(self.store.get(rejected["id"])["rejected"])

    def test_rejecting_text_with_failed_audio_replaces_it_instead_of_retrying_rejected_audio(self):
        self.engine.fail_audio = True
        self.prepare()
        rejected = self.store.rows()[0]
        self.store.select([rejected["id"]], "exclude")
        self.engine.fail_audio = False
        self.assertEqual(2, self.prepare(retry=True)["ready"])
        self.assertEqual(3, len(self.engine.text_calls))
        self.assertEqual(1, self.engine.audio_calls.count(rejected["id"]))

    def test_used_recording_is_not_synthesized_again_when_its_file_is_removed(self):
        self.prepare()
        row = self.store.consume("break", self.args["day"])
        (self.store.root / "audio" / row["audio_path"]).unlink()
        self.assertEqual(2, self.prepare()["ready"])
        self.assertEqual(2, len(self.engine.audio_calls))

    def test_revised_plan_counts_only_remaining_slots(self):
        self.prepare()
        result = self.prepare(tasks=self.args["tasks"][:1])
        self.assertEqual({"ready": 1, "total": 1, "pending": 0}, result)
        self.assertEqual(2, len(self.store.rows()))

    def test_revised_plan_uses_new_nearest_order_instead_of_old_database_order(self):
        old = {"slot": "later", "category": "dinner"}
        self.engine.fail_audio = True
        self.prepare(tasks=[old])
        old_row = self.store.rows()[0]
        self.engine.fail_audio = False
        result = self.prepare(tasks=[{"slot": "nearest", "category": "break"}, {"slot": "next", "category": "lunch"}, old], retry=True)
        self.assertEqual(2, result["ready"])
        self.assertEqual([("dinner", False), ("break", False), ("lunch", False)], self.engine.text_calls)
        self.assertEqual(1, self.engine.audio_calls.count(old_row["id"]))


class MixedBatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.store = Store(self.temp.name); self.engine = FakeEngine(self.store); self.jobs = Jobs(self.store, None, self.engine)
    def tearDown(self):
        self.store.db.close(); self.temp.cleanup()
    def batch(self, **options):
        args = dict(day="2026-10-22", count=2, categories=["break"], **options)
        return self.jobs.execute({"kind": "batch", "args": args, "event": threading.Event()}, lambda *unused: None)
    def test_stock_text_and_local_voice_need_no_text_model(self):
        result = self.batch(system_text=True, texts=["Stand up and enjoy a quiet pause.", "Take a short walk away from your desk."])
        self.assertEqual(2, result["count"])
        self.assertEqual([], self.engine.text_calls)
        self.assertEqual(2, len(self.engine.audio_calls))
        self.assertTrue(all(option["use_text_model"] is False for option in self.engine.direction_options))
        self.assertTrue(all(not row["approved"] for row in self.store.rows()))
    def test_generated_followups_and_system_voice_use_the_selected_model(self):
        self.store.configure({"speech_language": "en"})
        self.batch(system_voice=True, text_model="text-qvikhr3-8b", followup=True)
        self.assertEqual(["text-qvikhr3-8b"] * 2, self.engine.text_models)
        self.assertEqual([("break", True)] * 2, self.engine.text_calls)
        self.assertEqual([], self.engine.audio_calls)
        rows = self.store.rows()
        self.assertTrue(all(row["language"] == "en" and row["model"] == "system" and row["followup"] and not row["approved"] for row in rows))
        self.store.select([rows[0]["id"]], "keep")
        self.assertEqual(rows[0]["id"], self.store.consume("break", "2026-10-22", True, "en", allow_text=True)["id"])
    def test_invalid_batch_inputs_write_no_candidates(self):
        with self.assertRaises(ValueError): self.batch(system_text=True, texts=[])
        with self.assertRaises(ValueError): self.batch(text_model="unknown")
        self.assertEqual([], self.store.rows())


class NativeHistoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.root = Path(self.temp.name); self.store = Store(self.root)
        self.path = self.root / "builtin-days/2026-10-19.en.json"; self.path.parent.mkdir()
    def tearDown(self):
        self.store.db.close(); self.temp.cleanup()
    def row(self, number=1, **options):
        return dict(Id="stock-" + str(number) * 32, Text="A distinct native reminder number %d to take a break." % number, Category="break", Followup=False, CreatedAt="2026-10-19T09:00:00.0000000Z", UsedAt="", Kind="planned", **options)
    def write(self, rows):
        self.path.write_text(json.dumps(dict(Version=1, Day="2026-10-19", Language="en", Rows=rows)), encoding="utf-8-sig")
    def test_native_text_is_known_but_never_enters_the_playback_queue(self):
        rows = [self.row(1), self.row(2)]; self.write(rows)
        self.assertTrue(self.store.known(rows[0]["Text"]))
        self.assertIn(rows[1]["Text"], self.store.recent())
        self.assertEqual([], self.store.rows())
        self.assertIsNone(self.store.consume("break", "2026-10-19", language="en", allow_text=True))
        self.assertEqual(2, self.store.db.execute("select count(*) from phrases").fetchone()[0])
    def test_changed_native_file_imports_once_and_survives_restart(self):
        self.write([self.row(1)]); self.store.recent()
        self.write([self.row(1), self.row(2)]); self.store.recent(); self.store.recent()
        self.assertEqual(2, self.store.db.execute("select count(*) from phrases").fetchone()[0])
        self.store.db.close(); self.store = Store(self.root)
        self.assertTrue(self.store.known(self.row(2)["Text"]))
        self.assertEqual(2, self.store.db.execute("select count(*) from phrases").fetchone()[0])
    def test_invalid_file_is_retained_without_partial_import_or_blocking_generation(self):
        self.write([self.row(1), {"Id": "bad"}]); original = self.path.read_bytes()
        self.assertFalse(self.store.known(self.row(1)["Text"]))
        self.assertEqual(0, self.store.db.execute("select count(*) from phrases").fetchone()[0])
        self.assertTrue(self.store.builtin_warnings)
        self.assertEqual(original, self.path.read_bytes())
    def test_explicit_library_clear_also_clears_native_history_and_keeps_unrelated_files(self):
        self.write([self.row(1)]); unrelated = self.path.parent / "notes.json"; unrelated.write_text("keep")
        self.store.recent(); self.store.clear_library()
        self.assertFalse(self.path.exists())
        self.assertEqual("keep", unrelated.read_text())
        self.assertFalse(self.store.known(self.row(1)["Text"]))
    def test_model_playback_mirror_does_not_duplicate_existing_model_text(self):
        row = self.row(1); row["Kind"] = "external"; self.write([row])
        self.store.add(row["Text"], "break", "tts-cosyvoice3", "voice", "2026-10-19")
        self.store.recent()
        self.assertEqual(1, self.store.db.execute("select count(*) from phrases").fetchone()[0])


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.store = Store(self.temp.name)
        self.engine = FakeEngine(self.store)
        self.jobs = Jobs(self.store, None, self.engine)
        self.day = "2026-10-22"
    def tearDown(self):
        self.store.db.close(); self.temp.cleanup()
    def runtime(self, **options):
        args = dict(day=self.day, category="break", reserve=False, **options)
        return self.jobs.execute({"kind": "runtime", "args": args, "event": threading.Event()}, lambda *unused: None)
    def repair(self, **options):
        return self.jobs.repair(dict(day=self.day, **options), threading.Event(), lambda *unused: None)
    def test_runtime_uses_language_followup_and_its_selected_model(self):
        self.store.configure({"runtime_text_model": "text-qvikhr3-8b", "speech_language": "en"})
        row = self.runtime(followup=True, system_voice=True)
        self.assertEqual(["text-qvikhr3-8b"], self.engine.text_models)
        self.assertEqual([("break", True)], self.engine.text_calls)
        self.assertEqual(("en", 1, "system", None), (row["language"], row["followup"], row["model"], row["used_at"]))
    def test_settings_cannot_change_language_mid_generation(self):
        self.jobs.items["busy"] = {"state": "running"}
        with self.assertRaises(ValueError): self.jobs.configure({"speech_language": "en"})
        self.assertEqual("ru", self.store.settings()["speech_language"])
        self.jobs.configure({"speech_language": "ru"})
        self.jobs.items["busy"]["state"] = "done"
        self.jobs.configure({"speech_language": "en"})
        self.assertEqual("en", self.store.settings()["speech_language"])
    def test_late_runtime_result_remains_available_until_claimed_once(self):
        row = self.runtime(system_voice=True)
        with ThreadPoolExecutor(max_workers=8) as pool:
            results = list(pool.map(lambda unused: self.store.claim(row["id"], allow_text=True), range(16)))
        self.assertEqual(1, sum(result is not None for result in results))
        self.assertIsNone(self.store.consume("break", self.day, allow_text=True))
    def test_runtime_can_use_builtin_text_without_loading_a_text_model(self):
        row = self.runtime(system_text=True, text="Take a short break now.", system_voice=True)
        self.assertEqual("Take a short break now.", row["text"])
        self.assertEqual([], self.engine.text_calls)
        self.assertEqual([], self.engine.audio_calls)
    def test_failed_runtime_speech_keeps_text_and_repairs_after_restart(self):
        self.engine.fail_audio = True
        with self.assertRaises(RuntimeError): self.runtime()
        row = self.store.rows()[0]
        self.assertTrue(row["approved"])
        self.assertIsNone(row["audio_path"])
        self.repair()
        attempts = len(self.engine.audio_calls)
        self.repair()
        self.assertEqual(attempts, len(self.engine.audio_calls))
        self.store.db.close(); self.store = Store(self.temp.name)
        self.engine = FakeEngine(self.store); self.jobs = Jobs(self.store, None, self.engine)
        self.assertEqual(1, self.repair(retry=True)["repaired"])
        self.assertEqual([], self.engine.text_calls)
        self.assertEqual(row["id"], self.store.consume("break", self.day)["id"])
    def test_backoff_on_old_rows_does_not_starve_new_pending_audio(self):
        for i in range(2): self.store.add("Pending audio %d." % i, "break", "tts-cosyvoice3", self.engine.profile_hash, self.day, True)
        self.engine.fail_audio = True; self.repair()
        third = self.store.add("Another pending audio.", "break", "tts-cosyvoice3", self.engine.profile_hash, self.day, True)
        self.engine.fail_audio = False
        self.assertEqual(1, self.repair()["repaired"])
        self.assertTrue(self.store.get(third["id"])["audio_path"])


class PresetTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.root = Path(self.temp.name)
        self.profiles = Profiles(ROOT, self.root)
    def tearDown(self): self.temp.cleanup()
    def test_character_roundtrip_strips_other_preferences_and_credentials(self):
        package = self.root / "character.kavychka"
        with zipfile.ZipFile(package, "w") as archive:
            archive.writestr("preset.json", json.dumps({"version": 1, "kind": "character", "name": "Captain", "description": "A friendly captain.", "work_days": [0], "api_key": "must-not-import"}))
            archive.writestr("credentials.json", "must not be copied")
        item = self.profiles.import_file("character", package)
        folder, loaded = self.profiles.get(item["id"], "character")
        self.assertEqual({"version", "kind", "name", "description"}, set(loaded))
        self.assertEqual(["preset.json"], [p.name for p in folder.iterdir()])
        exported = self.root / "export.kavychka"
        self.profiles.export(item["id"], "character", exported)
        with zipfile.ZipFile(exported) as archive:
            self.assertEqual(["preset.json"], archive.namelist())
    def test_archive_cannot_escape_preset_directory_or_import_programs(self):
        for name in ("../outside.txt", "program.exe"):
            package = self.root / "bad.kavychka"
            with zipfile.ZipFile(package, "w") as archive:
                archive.writestr(name, "no")
            with self.assertRaises(ValueError): self.profiles.import_file("visual", package)
        self.assertFalse((self.root / "outside.txt").exists())
    def test_language_and_followup_are_explicit_in_generation_request(self):
        settings = {"character_description": "Friendly and witty", "swear": False, "speech_language": "en"}
        request = messages("break", settings, followup=True)
        self.assertIn("English", request[0]["content"])
        self.assertIn("follow-up", request[1]["content"])
        self.assertIn("stand up", request[1]["content"])
    @unittest.skipUnless(importlib.util.find_spec("soundfile"), "Requires the installed voice runtime")
    def test_real_voice_import_normalizes_and_validates_audio(self):
        import numpy as np
        import soundfile as sf
        wav = self.root / "reference.wav"
        sf.write(wav, np.sin(np.arange(48000) / 20) * 0.1, 24000)
        item = self.profiles.import_file("voice", wav)
        refs = self.profiles.voice(item["id"])
        self.assertEqual(1, len(refs))
        self.assertEqual("FLAC", sf.info(refs[0]["path"]).format)
        sf.write(wav, np.zeros(48000), 24000)
        with self.assertRaises(ValueError): self.profiles.import_file("voice", wav)


@unittest.skipUnless(importlib.util.find_spec("soundfile"), "Requires installed audio reader")
class QwenReferenceTests(unittest.TestCase):
    def test_optional_transcript_selects_supported_qwen_prompt_mode_and_changes_cache(self):
        from kavychka_local.engine import Engine
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as temporary:
            root = Path(temporary)
            store = Store(root)
            folder = root / "presets/voice1"; folder.mkdir(parents=True)
            import shutil
            shutil.copy2(ROOT / "assets/voice/default-en.wav", folder / "reference.wav")
            components = SimpleNamespace(manifest={"models": {"tts-0.6b": {"format": "qwen"}}}, ready=lambda key: True, path=lambda key: root / "model", voice_device=lambda: "cpu")
            hashes = []
            try:
                for transcript in ("", "Exact reference words"):
                    (folder / "preset.json").write_text(json.dumps({"version": 1, "kind": "voice", "name": "Voice", "references": [{"file": "reference.wav", "transcript": transcript}]}))
                    store.configure({"voice_profile": "voice1", "device": "cpu"})
                    engine = Engine(ROOT, store, components); hashes.append(engine.profile_hash)
                    model = SimpleNamespace(create_voice_clone_prompt=Mock(return_value=[]))
                    fake_torch = SimpleNamespace(set_num_threads=Mock(), cuda=SimpleNamespace(is_available=lambda: False), float32="float32")
                    fake_qwen = SimpleNamespace(Qwen3TTSModel=SimpleNamespace(from_pretrained=lambda *a, **kw: model))
                    with patch.dict(sys.modules, {"torch": fake_torch, "qwen_tts": fake_qwen}):
                        engine.load("tts-0.6b", threading.Event(), lambda *unused: None)
                    prompt = model.create_voice_clone_prompt.call_args.kwargs
                    self.assertEqual(not bool(transcript), prompt["x_vector_only_mode"])
                    self.assertEqual(transcript or None, prompt["ref_text"])
                self.assertNotEqual(*hashes)
            finally:
                store.db.close()


if __name__ == "__main__": unittest.main()
