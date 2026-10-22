import hashlib
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from http_server_fixture import stop_server

ROOT = Path(__file__).resolve().parents[1]
(ROOT / "work").mkdir(exist_ok=True)
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.store import Store, cache_key
from kavychka_local.jobs import Jobs, distribution
from kavychka_local.downloads import Cancelled, checkpoint, download

class StoreFixture:
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.store = Store(self.temp.name)

    def tearDown(self):
        self.store.db.close()
        self.temp.cleanup()

    def ready(self, text="Пора отдохнуть, Кавычка.", day="2026-10-18", approved=True):
        row = self.store.add(text, "break", "tts-0.6b", "profile", day, approved)
        name = row["id"] + ".flac"
        (self.store.root / "audio" / name).write_bytes(b"test")
        return self.store.audio(row["id"], "tts-0.6b", "hash", name)

class StoreTests(StoreFixture, unittest.TestCase):
    def test_expire_audio_preserves_text_history_and_current_audio(self):
        old = self.ready("Старый перерыв.", "2026-10-18")
        self.store.consume("break", "2026-10-18")
        today = self.ready("Сегодняшний перерыв.", "2026-10-19")
        result = self.store.expire_audio("2026-10-19")
        self.assertEqual(1, result["expired_rows"])
        previous = self.store.get(old["id"])
        self.assertIsNone(previous["audio_path"])
        self.assertIsNone(previous["cache_key"])
        self.assertTrue(previous["used_at"])
        self.assertTrue(self.store.known(old["text"]))
        self.assertFalse((self.store.root / "audio" / old["audio_path"]).exists())
        self.assertTrue((self.store.root / "audio" / today["audio_path"]).exists())
        self.assertEqual(0, self.store.expire_audio("2026-10-19")["expired_rows"])

    def test_clear_library_removes_history_audio_plans_preserves_settings_models(self):
        row = self.ready()
        self.store.consume("break", "2026-10-18")
        self.store.plan("2026-10-18", 13, "ready")
        self.store.configure({"batch_count": 7})
        model = self.store.root / "models/fixture.bin"
        model.parent.mkdir()
        model.write_bytes(b"model")
        result = self.store.clear_library()
        self.assertEqual(1, result["removed_rows"])
        self.assertEqual([], self.store.rows())
        self.assertEqual([], self.store.recent())
        self.assertIsNone(self.store.plan("2026-10-18"))
        self.assertEqual([], list((self.store.root / "audio").iterdir()))
        self.assertEqual(7, self.store.settings()["batch_count"])
        self.assertEqual(b"model", model.read_bytes())
        self.assertEqual(0, self.store.clear_library()["removed_rows"])

    def test_concurrent_consume_never_repeats(self):
        for i in range(8):
            self.ready("Пора отдохнуть, Кавычка %d." % i)
        with ThreadPoolExecutor(max_workers=12) as pool:
            rows = list(pool.map(lambda _: self.store.consume("break", "2026-10-18"), range(20)))
        ids = [row["id"] for row in rows if row]
        self.assertEqual(8, len(ids))
        self.assertEqual(8, len(set(ids)))

    def test_restart_and_midnight(self):
        row = self.ready()
        self.assertEqual(row["id"], self.store.consume("break", "2026-10-18")["id"])
        self.store.db.close()
        self.store = Store(self.temp.name)
        self.assertIsNone(self.store.consume("break", "2026-10-18"))
        self.assertIsNone(self.store.consume("break", "2026-10-19"))
        tomorrow = self.ready("Завтра другой перерыв.", "2026-10-19")
        self.assertEqual(tomorrow["id"], self.store.consume("break", "2026-10-19")["id"])

    def test_candidates_preview_missing_audio_and_cleanup(self):
        row = self.ready(approved=False)
        self.assertIsNone(self.store.consume("break", "2026-10-18"))
        self.assertIsNone(self.store.get(row["id"])["used_at"])
        self.assertEqual(0, self.store.cleanup())
        self.store.select([row["id"]], "keep")
        path = self.store.root / "audio" / row["audio_path"]
        path.unlink()
        self.assertIsNone(self.store.consume("break", "2026-10-18"))
        path.write_bytes(b"test")
        self.store.select([row["id"]], "delete")
        self.assertEqual(4, self.store.cleanup())
        self.assertTrue(self.store.known(row["text"]))

    def test_cache_key_and_settings_validation(self):
        key = cache_key("Текст", "0.6", "voice", "rev")
        for values in [("Другой текст", "0.6", "voice", "rev"), ("Текст", "1.7", "voice", "rev"), ("Текст", "0.6", "new", "rev"), ("Текст", "0.6", "voice", "new")]:
            self.assertNotEqual(key, cache_key(*values))
        for value in [{"batch_count": 0}, {"runtime": "yes"}, {"categories": []}, {"model_paths": []}]:
            with self.assertRaises(ValueError):
                self.store.configure(value)

class FakeEngine:
    profile_hash = "test"
    def __init__(self, store):
        self.store, self.counter = store, 0
    def text(self, category, event, progress, model=None):
        self.counter += 1
        return "Новая фраза %d для %s" % (self.counter, category)
    def direction(self, text, category, event, progress, text_model=None):
        from kavychka_local.delivery import fixed, select
        return fixed(self.store.settings()) or select("warm", category)
    def speech(self, row, model, event, progress, seed=None):
        seed = row.get("seed", 42) if seed is None else seed
        name = row["id"] + str(seed) + ".flac"
        (self.store.root / "audio" / name).write_bytes(b"test")
        return self.store.audio(row["id"], model, str(seed), name, seed)
    def unload(self):
        pass

class DailyTests(StoreFixture, unittest.TestCase):
    def test_runtime_uses_its_own_selected_model(self):
        self.store.configure({"text_model": "text-gemma4-12b", "runtime_text_model": "text-qvikhr3-8b"})
        engine = FakeEngine(self.store)
        original, calls = engine.text, []
        def record(category, event, progress, model=None):
            calls.append(model or self.store.settings()["text_model"])
            return original(category, event, progress, model)
        engine.text = record
        jobs = Jobs(self.store, None, engine)
        for kind, args in (("batch", {"day": "2026-10-18", "count": 1}),
                           ("runtime", {"day": "2026-10-18", "category": "break"})):
            job = jobs.submit(kind, args); jobs.queue.join()
            self.assertEqual("done", jobs.snapshot(job["id"])["state"])
        self.assertEqual(["text-gemma4-12b", "text-qvikhr3-8b"], calls)

    def test_regenerate_creates_candidate_and_keeps_original(self):
        row = self.ready()
        jobs = Jobs(self.store, None, FakeEngine(self.store))
        job = jobs.submit("rewrite", {"id": row["id"]})
        jobs.queue.join()
        result = jobs.snapshot(job["id"])
        self.assertEqual("done", result["state"], result.get("error"))
        self.assertFalse(result["result"]["approved"])
        self.assertTrue(self.store.get(row["id"])["approved"])
        self.assertNotEqual(row["id"], result["result"]["id"])

    def test_force_speech_changes_cache_and_retains_old_until_complete(self):
        row = self.ready()
        jobs = Jobs(self.store, None, FakeEngine(self.store))
        job = jobs.submit("speech", {"id": row["id"], "force": True})
        jobs.queue.join()
        result = jobs.snapshot(job["id"])
        self.assertEqual("done", result["state"])
        self.assertNotEqual(row["cache_key"], result["result"]["cache_key"])
        self.assertTrue((self.store.root / "audio" / row["audio_path"]).exists())

    def test_running_cancellation_releases_queue(self):
        engine = FakeEngine(self.store)
        started = threading.Event()
        original = engine.text
        def slow(category, event, progress, model=None):
            started.set(); event.wait(5); checkpoint(event)
            return original(category, event, progress, model=model)
        engine.text = slow
        jobs = Jobs(self.store, None, engine)
        job = jobs.submit("runtime", {"category": "break", "day": "2026-10-18"})
        self.assertTrue(started.wait(2))
        jobs.cancel(job["id"]); jobs.queue.join()
        self.assertEqual("cancelled", jobs.snapshot(job["id"])["state"])
        engine.text = original
        next_job = jobs.submit("batch", {"day": "2026-10-18", "count": 1})
        jobs.queue.join()
        self.assertEqual("done", jobs.snapshot(next_job["id"])["state"])

    def test_daily_idempotent_next_day_and_interrupted_resume(self):
        engine = FakeEngine(self.store)
        jobs = Jobs(self.store, None, engine)
        def execute(day):
            job = jobs.submit("daily", {"day": day})
            if job["id"]:
                jobs.queue.join()
                job = jobs.snapshot(job["id"])
            self.assertEqual("done", job["state"], job.get("error"))
        execute("2026-10-18")
        ids = {row["id"] for row in self.store.rows("2026-10-18")}
        self.assertEqual(13, len(ids))
        execute("2026-10-18")
        self.assertEqual(ids, {row["id"] for row in self.store.rows("2026-10-18")})
        execute("2026-10-19")
        self.assertTrue(all(row["audio_path"] is None for row in self.store.rows("2026-10-18")))
        self.assertTrue(all(row["audio_path"] for row in self.store.rows("2026-10-19")))
        self.assertEqual(26, engine.counter)
        self.assertFalse(ids & {row["id"] for row in self.store.rows("2026-10-19")})
        self.store.plan("2026-10-20", 3, "building")
        self.ready("Часть плана после прерывания.", "2026-10-20")
        execute("2026-10-20")
        self.assertEqual(3, len(self.store.rows("2026-10-20")))
        self.assertEqual(28, engine.counter)

    def test_daily_distribution(self):
        result = distribution(13, ["break", "breakfast", "lunch", "dinner", "exercise"])
        self.assertEqual(9, result.count("break"))
        self.assertEqual(1, result.count("lunch"))

class DownloadTests(unittest.TestCase):
    def test_resume_cancel_checksum_and_server_ignoring_range(self):
        content = b"content" * 200000
        observed = []
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_GET(self):
                offset = int(self.headers.get("Range", "bytes=0-").split("=")[1].split("-")[0])
                observed.append(offset)
                if self.path == "/416" and offset >= len(content):
                    self.send_error(416);return
                if self.path == "/ignore": offset = 0
                self.send_response(206 if offset else 200)
                if offset: self.send_header("Content-Range", "bytes %d-%d/%d" % (offset, len(content)-1, len(content)))
                self.end_headers()
                try: self.wfile.write(content[offset:])
                except (BrokenPipeError, ConnectionResetError): pass
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
                path = Path(directory) / "model.bin"
                partial = Path(str(path) + ".part")
                partial.write_bytes(content[:12345])
                sha = hashlib.sha256(content).hexdigest()
                event = threading.Event()
                url = "http://127.0.0.1:%d" % server.server_port
                download(url, path, len(content), sha, event, lambda *a: None)
                self.assertIn(12345, observed)
                self.assertEqual(content, path.read_bytes())
                path.unlink();partial.write_bytes(content[:999])
                download(url + "/ignore", path, len(content), sha, event, lambda *a: None)
                self.assertEqual(content, path.read_bytes())
                path.unlink()
                with self.assertRaises(ValueError): download(url, path, len(content), "0"*64, event, lambda *a: None)
                partial.write_bytes(b"x"*len(content))
                with self.assertRaises(ValueError): download(url+"/416", path, len(content), sha, event, lambda *a: None)
                self.assertFalse(partial.exists())
                with self.assertRaises(Cancelled): download(url, path, len(content), sha, event, lambda *a: event.set())
                self.assertTrue(partial.exists())
        finally:
            server.shutdown();server.server_close()

class ApiTests(unittest.TestCase):
    def test_loopback_auth_and_no_browser_origin(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            token = "test-" + "x"*40
            process = subprocess.Popen([sys.executable, "-B", str(ROOT / "backend/server.py"), "--root", str(ROOT), "--data", directory], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=dict(os.environ, KAVYCHKA_TOKEN=token))
            address = None
            try:
                line = process.stdout.readline()
                self.assertTrue(line, process.stderr.read() if process.poll() is not None else "not ready")
                port = json.loads(line)["port"]
                address = "http://127.0.0.1:%d" % port
                url = address + "/settings"
                with self.assertRaises(urllib.error.HTTPError): urllib.request.urlopen(url)
                request = urllib.request.Request(url, headers={"Authorization": "Bearer " + token})
                initial = json.load(urllib.request.urlopen(request))
                self.assertFalse(initial["runtime"])
                self.assertIn("Кавычка", initial["character_description"])
                payload = json.dumps({"character_description": "Капитан. Спокойный и немногословный."}).encode()
                update = urllib.request.Request(url, data=payload, headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
                self.assertEqual("Капитан. Спокойный и немногословный.", json.load(urllib.request.urlopen(update))["character_description"])
                self.assertEqual("Капитан. Спокойный и немногословный.", json.load(urllib.request.urlopen(request))["character_description"])
                request.add_header("Origin", "https://example.com")
                with self.assertRaises(urllib.error.HTTPError): urllib.request.urlopen(request)
            finally:
                stop_server(process, address, token)

if __name__ == "__main__": unittest.main(verbosity=2)
