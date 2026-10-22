"""Real v2 preparation/runtime/preset checks. Reuses weights; never installs models."""
import argparse
import io
import json
import os
import queue
import secrets
import shutil
import subprocess
import sys
import threading
import time
import urllib.request
import zipfile
from datetime import date, timedelta
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.stdout.reconfigure(encoding="utf-8")
parser = argparse.ArgumentParser()
parser.add_argument("--python", type=Path, required=True)
parser.add_argument("--models-data", type=Path, required=True)
parser.add_argument("--weights-data", type=Path, help="Optional separate existing weights; runtimes stay in models-data")
parser.add_argument("--device", choices=("auto", "cpu", "cuda"), default="auto")
parser.add_argument("--work", type=Path, required=True)
parser.add_argument("--builtin-only", action="store_true", help="Check local speech + automatic delivery without a text model")
parser.add_argument("--manual-batches", action="store_true", help="Check the modern library's mixed text/voice batches")
parser.add_argument("--native-history", type=Path, help="Copy native day JSON from an isolated fixture into this test library")
args = parser.parse_args()
data = args.work.resolve()
data.mkdir(parents=True, exist_ok=False)
artifacts = data / "artifacts"; artifacts.mkdir()
native_texts = set()
if args.native_history:
    native = data / "builtin-days"; native.mkdir()
    for source in args.native_history.glob("*.json"):
        document = json.loads(source.read_text(encoding="utf-8-sig"))
        assert document["Version"] == 1
        native_texts.update(row["Text"] for row in document["Rows"])
        shutil.copy2(source, native / source.name)
token = secrets.token_hex(32)
day = date.today().isoformat()
process = None
log = None
url = None
guard = """import socket,runpy,sys
original=socket.socket.connect
original_ex=socket.socket.connect_ex
def allowed(address):
    return not isinstance(address,tuple) or address[0] in ('127.0.0.1','::1')
def connect(self,address):
    if not allowed(address):raise RuntimeError('External connections disabled by the integration check')
    return original(self,address)
def connect_ex(self,address):
    if not allowed(address):raise RuntimeError('External connections disabled by the integration check')
    return original_ex(self,address)
socket.socket.connect=connect
socket.socket.connect_ex=connect_ex
runpy.run_path(sys.argv.pop(1),run_name='__main__')
"""


def start():
    global process, url, log
    log = (data / "server.log").open("a", encoding="utf-8")
    process = subprocess.Popen([str(args.python), "-B", "-c", guard, str(ROOT / "backend/server.py"), "--root", str(ROOT), "--data", str(data), "--components-readonly", str(args.models_data.resolve())], env=dict(os.environ, KAVYCHKA_TOKEN=token, PYTHONIOENCODING="utf-8"), stdout=subprocess.PIPE, stderr=log, text=True, encoding="utf-8", creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    lines = queue.Queue()
    threading.Thread(target=lambda: lines.put(process.stdout.readline()), daemon=True).start()
    line = lines.get(timeout=30)
    if not line:
        raise RuntimeError("Backend failed to start; see " + str(data / "server.log"))
    url = "http://127.0.0.1:" + str(json.loads(line)["port"])


def stop():
    if process and process.poll() is None:
        process.terminate(); process.wait(timeout=15)
    if log:
        log.close()


def api(path, body=None, binary=False):
    request = urllib.request.Request(url + path, data=json.dumps(body).encode() if body is not None else None, headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read() if binary else json.load(response)


def job(kind, payload):
    item = api("/jobs", {"kind": kind, "args": payload})
    last = None
    deadline = time.monotonic() + 900
    while item["state"] not in ("done", "failed", "cancelled"):
        if item["message"] != last:
            print(kind, item["message"], flush=True); last = item["message"]
        if time.monotonic() > deadline:
            raise TimeoutError("Job did not finish within 15 minutes; inspect the retained test process/logs")
        time.sleep(1)
        item = api("/jobs/" + item["id"])
    if item["state"] != "done":
        raise RuntimeError(item["error"])
    return item["result"]


def save_wav(row, name):
    wav = api("/audio/" + row["id"], binary=True)
    assert wav[:4] == b"RIFF" and len(wav) > 10000
    (artifacts / name).write_bytes(wav)


try:
    start()
    overrides = {"device": args.device}
    if args.weights_data:
        manifest = json.loads((ROOT / "assets/components.json").read_text(encoding="utf-8"))
        overrides["model_paths"] = {key: str((args.weights_data / "models" / key).resolve())
                                    for key in manifest["models"] if (args.weights_data / "models" / key).is_dir()}
    api("/settings", overrides)
    available = {item["id"]: item["installed"] for item in api("/status")["components"]}
    required = ["runtime", "cosy-runtime", "tts-cosyvoice3"]
    if not args.builtin_only:
        required += ["llama-runtime", "text-gemma4-12b"]
    assert all(available[key] for key in required), available
    if args.manual_batches:
        api("/settings", {"tts_model": "tts-cosyvoice3", "text_model": "text-gemma4-12b", "speech_language": "ru", "swear": False, "cosy_style": "auto"})
        batch = job("batch", {"day": day, "categories": ["break"], "count": 2, "system_voice": True, "text_model": "text-gemma4-12b", "followup": True})
        generated = api("/library")["rows"]
        assert len(generated) == 2 and all(row["text"] not in native_texts and row["followup"] and row["model"] == "system" and not row["audio_path"] and not row["approved"] for row in generated)
        selected = generated[0]["id"]
        api("/select", {"ids": [selected], "action": "keep"})
        assert api("/consume", {"day": day, "category": "break", "followup": True, "language": "ru", "allow_text": True})["row"]["id"] == selected
        api("/settings", {"text_model": "text-qvikhr3-8b", "runtime_text_model": "text-qvikhr3-8b"})
        assert not available["text-qvikhr3-8b"]
        spoken = job("batch", {"day": day, "categories": ["exercise"], "count": 2, "system_text": True, "texts": ["Устрой плечам короткие каникулы: встань и немного разомнись.", "Клавиатура заслужила паузу. Отойди от стола и сделай несколько удобных движений."]})
        voiced = [row for row in api("/library")["rows"] if row["id"] in spoken["ids"]]
        assert len(voiced) == 2 and all(row["audio_path"] and row["voice_instruction"] and not row["approved"] for row in voiced)
        for index, row in enumerate(voiced):
            save_wav(row, "stock-local-voice-%d.wav" % index)
        archive = api("/export", {}, binary=True)
        with zipfile.ZipFile(io.BytesIO(archive)) as exported:
            assert len(exported.namelist()) == 3 and "phrases.json" in exported.namelist()
        (artifacts / "library.zip").write_bytes(archive)
        stop(); start()
        assert api("/claim", {"id": selected, "allow_text": True})["row"] is None
        assert len(api("/library")["rows"]) == 4
        expired = job("expire-audio", {"day": (date.fromisoformat(day) + timedelta(days=1)).isoformat()})
        assert expired["expired_rows"] == 2 and all(not row["audio_path"] for row in api("/library")["rows"])
        report = {"passed": True, "native_history_texts": len(native_texts), "generated_system_voice_candidates": 2, "stock_local_voice_candidates": 2, "missing_text_model_not_required_for_stock": True, "restart_and_one_time_consumption": True, "audio_expired_text_retained": True, "texts": [row["text"] for row in generated]}
        (artifacts / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False), flush=True)
        raise SystemExit(0)
    if args.builtin_only:
        api("/settings", {"tts_model": "tts-cosyvoice3", "text_model": "text-qvikhr3-8b", "runtime_text_model": "text-qvikhr3-8b", "speech_language": "ru", "cosy_style": "auto"})
        row = job("runtime", {"day": day, "category": "break", "reserve": False, "system_text": True, "text": "Отложи дела на минуту: встань, выдохни и разомни плечи."})
        save_wav(row, "builtin-local-voice.wav")
        assert row["voice_instruction"] and row["audio_path"] and not row["used_at"]
        report = {"passed": True, "builtin_text_local_voice": True, "automatic_delivery": row["voice_style"], "text_model_installed": available["text-qvikhr3-8b"]}
        (artifacts / "result.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(json.dumps(report), flush=True)
        raise SystemExit(0)
    api("/settings", {"tts_model": "tts-cosyvoice3", "text_model": "text-gemma4-12b", "runtime_text_model": "text-gemma4-12b", "speech_language": "ru", "swear": False, "cosy_style": "calm"})
    plan = {"day": day, "tasks": [{"slot": "first-break", "category": "break", "followup": False}, {"slot": "teasing-repeat", "category": "break", "followup": True}]}
    prepared = job("prepare", plan)
    assert prepared["ready"] == 2, prepared
    rows = api("/library")["rows"]
    assert len(rows) == 2 and len({row["normalized"] for row in rows}) == 2
    for row in rows:
        assert row["language"] == "ru" and row["audio_path"] and not row["text"].lower().startswith("пора")
        save_wav(row, "followup-ru.wav" if row["followup"] else "reminder-ru.wav")
    for followup in (False, True):
        assert api("/consume", {"day": day, "category": "break", "language": "ru", "followup": followup})["row"]
        assert api("/consume", {"day": day, "category": "break", "language": "ru", "followup": followup})["row"] is None
    assert job("prepare", plan)["ready"] == 2
    text_file = data / "personality.txt"
    text_file.write_text("You are a witty quotation-mark companion. Be brief and friendly, use dry humor, suggest comfortable movement. No romance or lecturing.", encoding="utf-8")
    character = api("/presets/import", {"kind": "character", "file": str(text_file)})
    api("/presets/apply", {"kind": "character", "id": character["id"]})
    assert api("/settings")["voice_profile"] == ""
    voice = api("/presets/import", {"kind": "voice", "file": str(ROOT / "assets/voice/default-en.wav"), "transcript": "Take a little time for yourself. Step away from the screen, stretch your shoulders, and take a calm breath. Your work can wait for a moment."})
    api("/presets/apply", {"kind": "voice", "id": voice["id"]})
    api("/presets/export", {"kind": "voice", "id": voice["id"], "file": str(artifacts / "voice.kavychka")})
    with zipfile.ZipFile(artifacts / "voice.kavychka") as archive:
        assert set(archive.namelist()) == {"preset.json", "reference.flac"}
    api("/settings", {"speech_language": "en", "runtime": True})
    fresh = job("runtime", {"day": day, "category": "exercise", "followup": True, "reserve": False})
    assert fresh["language"] == "en" and fresh["followup"] and fresh["audio_path"] and fresh["used_at"] is None
    save_wav(fresh, "runtime-en.wav")
    assert api("/claim", {"id": fresh["id"]})["row"]
    assert api("/claim", {"id": fresh["id"]})["row"] is None
    archive = api("/export", {}, binary=True)
    with zipfile.ZipFile(io.BytesIO(archive)) as exported:
        assert "phrases.json" in exported.namelist() and len(exported.namelist()) == 4
    (artifacts / "library.zip").write_bytes(archive)
    stop(); start()
    assert api("/claim", {"id": fresh["id"]})["row"] is None
    assert len(api("/presets")["items"]) == 2
    expired = job("expire-audio", {"day": (date.fromisoformat(day) + timedelta(days=1)).isoformat()})
    assert expired["expired_rows"] == 3 and len(api("/library")["rows"]) == 3
    assert all(row["audio_path"] is None for row in api("/library")["rows"])
    report = {"passed": True, "prepared_ru": 2, "runtime_en": True, "independent_presets": True, "one_time_consume_and_claim": True, "restart": True, "old_audio_deleted_texts_retained": True, "component_installs_disabled": True, "texts": [row["text"] for row in rows] + [fresh["text"]]}
    (artifacts / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False), flush=True)
finally:
    stop()
