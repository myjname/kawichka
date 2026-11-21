import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path
from http_server_fixture import stop_server

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.locale import PAIRS, response, text
from kavychka_local.character import DEFAULT_DESCRIPTION, PROFILE, messages
from kavychka_local.store import DEFAULTS


class LocaleTests(unittest.TestCase):
    def test_templates_keep_paths_model_names_and_speech_text(self):
        self.assertEqual("Downloading: голос.flac", text("Загрузка: голос.flac", "en"))
        self.assertEqual("Selected model files are not ready. Open Voice and models and choose Set up models.",
                         text("Файлы выбранной модели не готовы. Откройте «Голос и модели» и нажмите «Подготовить модели».", "en"))
        self.assertEqual("Speech: Фраза не найдена", text("Озвучка: Фраза не найдена", "en"))
        self.assertEqual("Голосовая запись отсутствует", text("Voice reference is missing", "ru"))
        self.assertIn("wording", text("Повтор генерации: Повторяется прежняя формулировка или начало.", "en"))
        self.assertEqual("Ready in 2.5 s · 3.0 s audio", text("Готово за 2.5 с · 3.0 с аудио", "en"))

    def test_response_changes_only_service_fields(self):
        job = dict(kind="runtime", message="Озвучка: Голос", error=None, result=dict(text="Голос", name="Готово", instruction="Ошибка"))
        localized = response(job, "/jobs/one", "en")
        self.assertEqual("Speech: Голос", localized["message"])
        self.assertEqual(job["result"], localized["result"])
        self.assertEqual("Озвучка: Голос", job["message"])
        presets = {"items": [dict(name="Готово", description="Ошибка", transcript="В очереди")]}
        self.assertEqual(presets, response(presets, "/presets", "en"))
        settings = {"character_description": "Готово", "speech_language": "ru", "delivery_catalog": [{"name": "Ошибка"}]}
        self.assertEqual(settings, response(settings, "/settings", "en"))

    def test_installation_keeps_raw_pip_details_out_of_the_main_status(self):
        job = dict(kind="install", state="running", message="Requirement already satisfied: torch")
        localized = response(job, "/jobs/one", "ru")
        self.assertIn("Установка компонентов", localized["message"])
        self.assertEqual(job["message"], localized["detail"])
        self.assertIn("Установка компонентов", response(job, "/jobs/one", "unsupported")["message"])

    def test_default_personality_uses_speech_language_and_custom_text_is_unchanged(self):
        settings = dict(DEFAULTS, speech_language="en")
        self.assertIn(PROFILE["description_en"], messages("break", settings)[0]["content"])
        custom = "Мой персонаж с неизменённым русским описанием."
        settings["character_description"] = custom
        self.assertIn(custom, messages("break", settings)[0]["content"])
        settings.update(speech_language="ru", character_description=PROFILE["description_en"])
        self.assertIn(DEFAULT_DESCRIPTION, messages("break", settings)[0]["content"])

    def test_every_catalog_entry_translates_both_directions_without_format_errors(self):
        for ru, en in PAIRS:
            with self.subTest(message=ru):
                sample = ru.replace("{0}", "example").replace("{1}", "file")
                self.assertIsInstance(text(sample, "en"), str)
                self.assertIsInstance(text(en.replace("{0}", "example").replace("{1}", "file"), "ru"), str)


class LocaleApiTests(unittest.TestCase):
    def test_header_localizes_jobs_errors_and_components_without_mutating_speech(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            token = "locale-" + "x" * 40
            process = subprocess.Popen([sys.executable, "-B", str(ROOT / "backend/server.py"), "--root", str(ROOT), "--data", directory],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                       env=dict(os.environ, KAVYCHKA_TOKEN=token))
            address = None
            try:
                line = process.stdout.readline()
                self.assertTrue(line)
                address = "http://127.0.0.1:" + str(json.loads(line)["port"])
                def api(route, body=None, language="en"):
                    request = urllib.request.Request(address + route, data=None if body is None else json.dumps(body).encode(),
                                                     headers={"Authorization": "Bearer " + token, "Content-Type": "application/json", "X-Kavychka-Language": language})
                    return json.load(urllib.request.urlopen(request))
                before = api("/settings")
                with self.assertRaises(urllib.error.HTTPError) as caught:
                    api("/settings", {"device": "invalid"})
                self.assertEqual("Invalid device", json.load(caught.exception)["error"])
                self.assertEqual(before, api("/settings", language="ru"))
                status = api("/status")
                self.assertEqual("llama.cpp · text generator", next(item["name"] for item in status["components"] if item["id"] == "llama-runtime"))
                phrase = "Отложи работу на минуту и немного разомни плечи."
                job = api("/jobs", {"kind": "prepare", "args": {"day": "2026-10-22", "system_text": True, "system_voice": True,
                                  "tasks": [{"slot": "locale", "category": "exercise", "followup": False, "text": phrase}]}})
                until = time.monotonic() + 10
                while job["state"] in ("queued", "running"):
                    self.assertLess(time.monotonic(), until)
                    time.sleep(0.05)
                    job = api("/jobs/" + job["id"])
                self.assertEqual("done", job["state"], job)
                self.assertEqual("Ready", job["message"])
                self.assertEqual(phrase, api("/library")["rows"][0]["text"])
                self.assertEqual("ru", api("/settings")["speech_language"])
                self.assertEqual("Готово", api("/jobs/" + job["id"], language="ru")["message"])
            finally:
                stop_server(process, address, token)


if __name__ == "__main__":
    unittest.main()
