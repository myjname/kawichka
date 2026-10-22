import argparse
import io
import json
import os
import sys
import threading
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse

parser = argparse.ArgumentParser()
parser.add_argument("--root", required=True)
parser.add_argument("--data", required=True)
parser.add_argument("--parent", type=int)
parser.add_argument("--components-readonly", help="Reuse already installed components without installing into their source")
args = parser.parse_args()
root, data = Path(args.root).resolve(), Path(args.data).resolve()
os.environ.update(HF_HOME=str(data / "hf"), TORCH_HOME=str(data / "torch"), NUMBA_CACHE_DIR=str(data / "numba"),
                  HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1", HF_HUB_DISABLE_TELEMETRY="1", PYTHONDONTWRITEBYTECODE="1")
sys.path.insert(0, str(root / "backend"))
from kavychka_local.store import Store
from kavychka_local.downloads import Components, ReadOnlyComponents
from kavychka_local.engine import Engine
from kavychka_local.jobs import Jobs
from kavychka_local.profiles import Profiles
from kavychka_local.locale import response as localize_response

store = Store(data)
components = ReadOnlyComponents(root, Path(args.components_readonly).resolve(), store) if args.components_readonly else Components(root, data, store)
engine = Engine(root, store, components)
jobs = Jobs(store, components, engine)
profiles = Profiles(root, data)
token = os.environ.get("KAVYCHKA_TOKEN")
if not token or len(token) < 32:
    raise SystemExit("A private session token is required")

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *unused):
        pass

    def reply(self, value, status=200, content_type="application/json; charset=utf-8"):
        if not isinstance(value, bytes):
            value = localize_response(value, urlparse(self.path).path, self.headers.get("X-Kavychka-Language", "ru"))
        payload = value if isinstance(value, bytes) else json.dumps(value, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        self.handle_request(False)

    def do_POST(self):
        self.handle_request(True)

    def handle_request(self, post):
        if self.headers.get("Authorization") != "Bearer " + token or self.headers.get("Origin"):
            self.reply({"error": "Unauthorized"}, 401)
            return
        try:
            path = urlparse(self.path).path
            size = int(self.headers.get("Content-Length", 0))
            if size < 0 or size > 1024 * 1024:
                raise ValueError("Слишком большой запрос")
            body = json.loads(self.rfile.read(size)) if post and size else {}
            if path == "/settings":
                return self.reply(jobs.configure(body) if post else store.settings())
            if path == "/claim" and post:
                return self.reply({"row": store.claim(body["id"], bool(body.get("allow_text", False)))})
            if path == "/status" and not post:
                return self.reply({"components": components.status(), "stats": store.stats(), "hardware": components.hardware(),
                                   "installation": {"speech_gpu": components.use_gpu("runtime"), "text_gpu": components.use_gpu("llama-runtime")}})
            if path == "/presets" and not post:
                return self.reply({"items": profiles.list()})
            if path == "/presets/import" and post:
                if any(j["state"] in ("queued", "running") for j in jobs.items.values()):
                    raise ValueError("Wait until generation finishes before importing a preset")
                return self.reply(profiles.import_file(body["kind"], body["file"], body.get("name", ""), body.get("transcript", "")))
            if path == "/presets/apply" and post:
                if any(j["state"] in ("queued", "running") for j in jobs.items.values()):
                    raise ValueError("Wait until generation finishes before changing a preset")
                kind, ident = body["kind"], body["id"]
                folder, item = profiles.get(ident, kind)
                if kind == "voice":
                    jobs.configure({"voice_profile": ident})
                elif kind == "character":
                    jobs.configure({"character_description": item["description"]})
                return self.reply({"ok": True, "file": str(folder / item.get("file", "preset.json"))})
            if path == "/presets/export" and post:
                return self.reply(profiles.export(body["id"], body["kind"], Path(body["file"])))
            if path == "/library" and not post:
                return self.reply({"rows": store.rows()})
            if path == "/preview" and post:
                from kavychka_local.store import normalize
                text = body["text"]
                with store.lock:
                    previous = store.db.execute("SELECT id FROM phrases WHERE normalized=? AND deleted=0", (normalize(text),)).fetchone()
                row = store.get(previous[0]) if previous else store.add(text, "break", store.settings()["tts_model"], engine.profile_hash)
                return self.reply(row)
            if path.startswith("/jobs/"):
                ident = path.split("/")[2]
                if post:
                    jobs.cancel(ident)
                return self.reply(jobs.snapshot(ident))
            if path == "/jobs" and post:
                return self.reply(jobs.submit(body["kind"], body.get("args", {})))
            if path == "/select" and post:
                jobs_in_progress = any(j["state"] in ("queued", "running") and j["kind"] != "install" for j in jobs.items.values())
                if jobs_in_progress and body["action"] == "delete":
                    raise ValueError("Дождитесь окончания генерации или отмените её перед удалением")
                store.select(body["ids"], body["action"])
                return self.reply({"ok": True})
            if path == "/consume" and post:
                store.plan(body["day"])
                return self.reply({"row": store.consume(body["category"], body["day"], body.get("followup", False), body.get("language"), body.get("allow_text", False))})
            if path == "/library/clear" and post:
                if body.get("confirm") != "clear-library":
                    raise ValueError("Требуется подтверждение очистки библиотеки")
                if any(j["state"] in ("queued", "running") for j in jobs.items.values()):
                    raise ValueError("Дождитесь завершения очереди или отмените её перед очисткой")
                return self.reply(store.clear_library())
            if path == "/cleanup" and post:
                if any(j["state"] in ("queued", "running") for j in jobs.items.values()):
                    raise ValueError("Очистка доступна после завершения очереди")
                return self.reply({"removed_bytes": store.cleanup()})
            if path.startswith("/audio/") and not post:
                import soundfile as sf
                row = store.get(path.split("/")[2])
                if not row["audio_path"]:
                    raise ValueError("Запись пока не готова")
                audio, rate = sf.read(data / "audio" / row["audio_path"], dtype="int16")
                stream = io.BytesIO()
                sf.write(stream, audio, rate, format="WAV", subtype="PCM_16")
                return self.reply(stream.getvalue(), content_type="audio/wav")
            if path == "/export" and post:
                rows = [store.get(i) for i in body["ids"]] if body.get("ids") else store.rows()
                stream = io.BytesIO()
                with zipfile.ZipFile(stream, "w", zipfile.ZIP_DEFLATED) as archive:
                    archive.writestr("phrases.json", json.dumps(rows, ensure_ascii=False, indent=2))
                    for row in rows:
                        if row["audio_path"] and (data / "audio" / row["audio_path"]).exists():
                            archive.write(data / "audio" / row["audio_path"], "audio/" + row["audio_path"])
                return self.reply(stream.getvalue(), content_type="application/zip")
            if path == "/shutdown" and post:
                for job in jobs.items.values():
                    job["event"].set()
                self.reply({"ok": True})
                threading.Thread(target=server.shutdown, daemon=True).start()
                return
            self.reply({"error": "Not found"}, 404)
        except (ValueError, KeyError, TypeError) as error:
            self.reply({"error": str(error)}, 400)
        except Exception as error:
            self.reply({"error": str(error)}, 500)

server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
server.daemon_threads = True
print(json.dumps({"port": server.server_port}), flush=True)
if args.parent and os.name == "nt":
    def watch_parent():
        import ctypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.restype = ctypes.c_void_p
        handle = kernel.OpenProcess(0x100000, False, args.parent)
        if handle:
            kernel.WaitForSingleObject(ctypes.c_void_p(handle), 0xFFFFFFFF)
            kernel.CloseHandle(ctypes.c_void_p(handle))
        os._exit(0)
    threading.Thread(target=watch_parent, daemon=True).start()
server.serve_forever(poll_interval=0.25)
