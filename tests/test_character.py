import sys
import tempfile
import unittest
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.character import DEFAULT_DESCRIPTION, MAX_DESCRIPTION, messages, starts_with_pora
from kavychka_local.store import Store
from kavychka_local.engine import Engine


class CharacterTests(unittest.TestCase):
    def test_english_uses_all_six_structural_variants(self):
        settings = dict(character_description="A concise desk companion.", swear=False, speech_language="en")
        prompts = [messages("break", settings, variant=i)[-1]["content"] for i in range(6)]
        self.assertEqual(len(set(prompts)), 6)
        self.assertIn("question", prompts[2])

    def test_opening_preference_tries_another_structure_without_losing_valid_text(self):
        self._opening_preference_case([
            "Твои плечи заслужили отдыха. Отойди от экрана.",
            "Пять минут без монитора пойдут на пользу. Встань и пройдись.",
        ], "Пять минут", 2)

    def test_opening_preference_exhaustion_keeps_the_valid_unique_reserve(self):
        self._opening_preference_case([
            "Твои плечи заслужили отдыха. Отойди от экрана.",
        ] * 6, "Твои плечи", 6)

    def test_opening_reserve_cannot_bypass_hard_rejection(self):
        self._opening_preference_case(["Пора встать и отойти от экрана."] * 6, None, 6)
        self._opening_preference_case(["Ты успешно разобрал бумаги. Теперь отложи их."] * 6, None, 6)
        self._opening_preference_case([
            "Твои плечи заслужили отдыха. Отойди от экрана.",
        ] + ["Пора встать и отойти от экрана."] * 5, "Твои плечи", 6)

    def test_cancellation_does_not_return_a_style_reserve(self):
        self._opening_preference_case(["Твои плечи заслужили отдыха. Отойди от экрана."] * 6, None, 6, cancel=True)

    def test_runtime_does_not_wait_for_style_only_retries(self):
        self._opening_preference_case(["Твои плечи заслужили отдыха. Отойди от экрана."] * 6, "Твои плечи", 1, runtime=True)

    def test_later_style_request_failure_keeps_a_valid_unique_line(self):
        for kind in (RuntimeError, OSError, ValueError):
            with self.subTest(kind=kind):
                self._opening_preference_case(["Твои плечи заслужили отдыха. Отойди от экрана.", kind("injected extra-request failure")], "Твои плечи", 2)

    def test_cancellation_during_a_failed_style_request_does_not_publish_reserve(self):
        self._opening_preference_case(["Твои плечи заслужили отдыха. Отойди от экрана."] * 5 + [RuntimeError("injected extra-request failure")], None, 6, cancel=True)

    def _opening_preference_case(self, outputs, prefix, count, cancel=False, runtime=False):
        from types import SimpleNamespace
        from kavychka_local.engine import Engine
        calls = []
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            store = Store(directory)
            try:
                store.configure({"runtime": runtime})
                for text in ("Ты успешно разобрал бумаги. Теперь отложи их.", "Твой рабочий день насыщен. Обед подождёт?", "Твоя прогулка скоро начнётся. Жду у двери."):
                    store.add(text, "break", "tts-0.6b", "voice")
                class Model:
                    def generate(self, prompt, event):
                        calls.append(prompt)
                        if cancel and len(calls) == 6:
                            event.set()
                        value = outputs[len(calls) - 1]
                        if isinstance(value, Exception):
                            raise value
                        return value
                engine = Engine(ROOT, store, SimpleNamespace(manifest={"models": {"test": {"format": "gguf"}}}))
                engine.loaded = "test"
                engine.model = Model()
                engine.load = lambda *args: None
                if cancel:
                    from kavychka_local.downloads import Cancelled
                    with self.assertRaises(Cancelled):
                        engine.text("break", threading.Event(), lambda *args: None)
                elif prefix is None:
                    with self.assertRaises(ValueError):
                        engine.text("break", threading.Event(), lambda *args: None)
                else:
                    self.assertTrue(engine.text("break", threading.Event(), lambda *args: None).startswith(prefix))
                self.assertEqual(len(calls), count)
                self.assertIn("ты / твой / твои", str(calls[-1]))
            finally:
                store.db.close()

    def test_preferred_name_reaches_both_reminder_languages(self):
        for language in ("ru", "en"):
            settings = {"character_description": "A dry-witted companion", "swear": False,
                        "speech_language": language, "user_name": "Mira"}
            with self.subTest(language=language):
                prompt = messages("break", settings, followup=True)
                self.assertIn("Mira", prompt[1]["content"])
                self.assertNotIn("Mira", prompt[0]["content"])
                self.assertIn("A dry-witted companion", prompt[0]["content"])

    def test_retry_explains_rejection_and_shows_hidden_historical_conflict(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            store = Store(directory)
            try:
                old = "Закрой вкладки и встань на короткий перерыв, я подожду у окна."
                store.add(old, "break", "tts-cosyvoice3", "profile")
                for i in range(7):
                    store.add("Другая запись %d." % i, "break", "tts-cosyvoice3", "profile")
                engine = Engine(ROOT, store, None)
                engine.components = type("Components", (), {"manifest": {"models": {"test": {"format": "gguf"}}}})()
                calls = []
                replies = iter([old, "Пора сделать перерыв.", "Дай глазам отдохнуть: отойди от монитора и посмотри в окно."])
                class Model:
                    def generate(self, messages, event):
                        calls.append(messages[-1]["content"])
                        return next(replies)
                engine.model = Model()
                engine.loaded = "test"
                engine.load = lambda *args: None
                result = engine.text("break", threading.Event(), lambda *args: None)
                self.assertIn("Дай глазам", result)
                self.assertIn("Уже было: «" + old, calls[1])
                self.assertIn("Начало «Пора» запрещено", calls[2])
            finally:
                store.db.close()

    def test_existing_library_gets_default_and_custom_description_survives_restart(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            store = Store(directory)
            try:
                # A library created before character descriptions has no such settings row.
                store.configure({"batch_count": 7, "swear": False})
                original = store.add("Пора пообедать.", "lunch", "tts-0.6b", "voice", "2026-10-19", True)
                store.plan("2026-10-19", 7, "ready")
                self.assertEqual(DEFAULT_DESCRIPTION, store.settings()["character_description"])
                description = "Ты спокойный капитан космического корабля. Говори с мягкой иронией.\nБез флирта."
                store.configure({"character_description": description})
                store.db.close()
                store = Store(directory)
                self.assertEqual(description, store.settings()["character_description"])
                self.assertEqual(7, store.settings()["batch_count"])
                self.assertEqual(original, store.get(original["id"]))
                self.assertEqual("ready", store.plan("2026-10-19")["status"])
                store.configure({"character_description": DEFAULT_DESCRIPTION})
                self.assertEqual(DEFAULT_DESCRIPTION, store.settings()["character_description"])
            finally:
                store.db.close()

    def test_invalid_description_is_rejected_without_overwriting_saved_one(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            store = Store(directory)
            try:
                for value in (None, 7, [], "", " \r\n ", "я" * (MAX_DESCRIPTION + 1)):
                    with self.assertRaises(ValueError):
                        store.configure({"character_description": value})
                    self.assertEqual(DEFAULT_DESCRIPTION, store.settings()["character_description"])
                self.assertEqual("я" * MAX_DESCRIPTION,
                                 store.configure({"character_description": "я" * MAX_DESCRIPTION})["character_description"])
            finally:
                store.db.close()

    def test_custom_character_replaces_default_in_every_category_without_fixed_flirt(self):
        for category in ("break", "breakfast", "lunch", "dinner", "exercise"):
            prompt = messages(category, {"character_description": "Ты деловой робот. Без шуток.", "swear": False})
            self.assertIn("Ты деловой робот. Без шуток.", prompt[0]["content"])
            self.assertNotIn(DEFAULT_DESCRIPTION, str(prompt))
            self.assertNotIn("флиртующий", str(prompt))
            self.assertIn("Строго без мата", prompt[0]["content"])
            self.assertIn("Верни только произносимую реплику", prompt[0]["content"])
            self.assertEqual(["system", "user"], [entry["role"] for entry in prompt])

    def test_pora_opening_rejected_but_word_in_middle_allowed(self):
        for text in ("Пора сделать перерыв.", "«Пора пообедать»", "— Ну, пора отдохнуть.", "Ну что, пора размяться!", "пора поужинать"):
            self.assertTrue(starts_with_pora(text), text)
        for text in ("Отложи работу. Пора пообедать.", "Давай сделаем перерыв.", "Порадуй себя обедом."):
            self.assertFalse(starts_with_pora(text), text)


if __name__ == "__main__":
    unittest.main()
