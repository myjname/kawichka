import queue
import secrets
import threading
import uuid
import json
import hashlib
import time
from .downloads import Cancelled, checkpoint
from .store import CATEGORIES, TEXT_MODELS, now

def distribution(count, categories):
    # One meal/exercise each, the remaining scheduled notifications are breaks.
    order = [c for c in CATEGORIES if c in categories and c != "break"]
    result = order[:count]
    while len(result) < count:
        result.append("break" if "break" in categories else categories[(len(result) - len(order)) % len(categories)])
    return result

class Jobs:
    def __init__(self, store, components, engine):
        self.store, self.components, self.engine = store, components, engine
        self.lock = threading.RLock()
        self.items = {}
        self.queue = queue.Queue()
        threading.Thread(target=self.worker, daemon=True).start()

    def submit(self, kind, args):
        with self.lock:
            if self.queue.qsize() > 100:
                raise ValueError("Очередь заполнена")
            if kind == "daily":
                day = args["day"]
                plan = self.store.plan(day)
                for item in self.items.values():
                    if item["kind"] == "daily" and item["args"].get("day") == day and item["state"] in ("queued", "running"):
                        return self.snapshot(item["id"])
            ident = uuid.uuid4().hex
            self.items[ident] = dict(id=ident, kind=kind, args=args, state="queued", message="В очереди", done=0, total=0, result=None, error=None, event=threading.Event())
            self.queue.put(ident)
            return self.snapshot(ident)

    def configure(self, values):
        # A phrase and its audio must use one settings version. Serialise changes
        # against job submission; UI saves can be retried when the job finishes.
        with self.lock:
            current = self.store.settings()
            changed = any(key not in current or current[key] != value for key, value in values.items())
            if changed and any(item["state"] in ("queued", "running") for item in self.items.values()):
                raise ValueError("Настройки генератора применятся после завершения текущей операции")
            return self.store.configure(values)

    def snapshot(self, ident):
        with self.lock:
            return {k: v for k, v in self.items[ident].items() if k not in ("event", "args")}

    def cancel(self, ident):
        with self.lock:
            self.items[ident]["event"].set()

    def worker(self):
        while True:
            ident = self.queue.get()
            job = self.items[ident]
            def progress(message, done=0, total=0):
                with self.lock:
                    job.update(message=message, done=done, total=total)
            try:
                checkpoint(job["event"])
                job["state"] = "running"
                result = self.execute(job, progress)
                checkpoint(job["event"])
                job.update(state="done", result=result, message="Готово")
            except Cancelled as error:
                job.update(state="cancelled", error=str(error), message="Отменено")
            except Exception as error:
                job.update(state="failed", error=str(error), message="Ошибка")
            finally:
                self.engine.unload()
                self.queue.task_done()

    def execute(self, job, progress):
        kind, args, event = job["kind"], job["args"], job["event"]
        settings = self.store.settings()
        if kind == "install":
            return self.components.install(args["component"], event, progress, args.get("gpu"))
        if kind == "expire-audio":
            return self.store.expire_audio(args["day"])
        if kind == "prepare":
            return self.prepare(args, event, progress)
        if kind == "repair":
            return self.repair(args, event, progress)
        model = settings["tts_model"]
        if kind == "speech":
            row = self.store.get(args["id"])
            return self.engine.speech(row, model, event, progress, seed=secrets.randbelow(2**31) if args.get("force") else None)
        if kind == "rewrite":
            old = self.store.get(args["id"])
            text = self.engine.text(old["category"], event, progress)
            delivery = self.engine.direction(text, old["category"], event, progress) if model == "tts-cosyvoice3" else None
            row = self.store.add(text, old["category"], model, self.engine.profile_hash, old["day"], False, delivery)
            return self.engine.speech(row, model, event, progress)
        if kind == "runtime":
            category, day = args["category"], args["day"]
            self.store.plan(day)  # validate date
            followup = args.get("followup", False)
            if category not in CATEGORIES or not isinstance(followup, bool):
                raise ValueError("Invalid reminder category")
            if args.get("system_text") and (not isinstance(args.get("text"), str) or not 4 <= len(args["text"].strip()) <= 400):
                raise ValueError("Invalid ready-made phrase")
            self.store.expire_audio(day)
            if hasattr(self.engine, "refresh_voice"):
                self.engine.refresh_voice()
            options = {"model": settings["runtime_text_model"]}
            if followup:
                options["followup"] = True
            text = args.get("text") if args.get("system_text") else self.engine.text(category, event, progress, **options)
            delivery_options = {"text_model": settings["runtime_text_model"]}
            if args.get("system_text"):
                delivery_options["use_text_model"] = False
            delivery = self.engine.direction(text, category, event, progress, **delivery_options) if not args.get("system_voice") and model == "tts-cosyvoice3" else None
            row = self.store.add(text, category, "system" if args.get("system_voice") else model, self.engine.profile_hash, day, True, delivery, followup, settings["speech_language"], builtin=bool(args.get("system_text")))
            job["result"] = row
            if not args.get("system_voice"):
                row = self.engine.speech(row, model, event, progress)
            if args.get("reserve", True):
                self.store.claim(row["id"], allow_text=bool(args.get("system_voice")))
            return self.store.get(row["id"])
        if kind not in ("daily", "batch"):
            raise ValueError("Неизвестное задание")
        day = args["day"]
        self.store.plan(day)
        self.store.expire_audio(day)
        count = args.get("count", settings["batch_count"])
        categories = args.get("categories", settings["categories"])
        if type(count) is not int or not 1 <= count <= 100 or not categories or not set(categories) <= set(CATEGORIES):
            raise ValueError("Некорректный объём генерации")
        texts = args.get("texts")
        if args.get("system_text") and (not isinstance(texts, list) or len(texts) != count or any(not isinstance(text, str) or not 4 <= len(text.strip()) <= 400 for text in texts)):
            raise ValueError("Invalid ready-made phrase batch")
        if "text_model" in args and args["text_model"] not in TEXT_MODELS:
            raise ValueError("Unknown batch text model")
        if hasattr(self.engine, "refresh_voice"):
            self.engine.refresh_voice()
        daily = kind == "daily"
        rows = list(reversed(self.store.rows(day))) if daily else []
        if daily:
            plan = self.store.plan(day)
            if plan and plan["status"] == "ready":
                return {"day": day, "count": len(rows), "ids": [r["id"] for r in rows]}
            if plan:
                count = plan["target"]
            self.store.plan(day, count, "building")
        desired = distribution(count, categories)
        for row in rows:
            if row["category"] in desired:
                desired.remove(row["category"])
        desired = desired[:max(0, count - len(rows))]
        for i, category in enumerate(desired):
            checkpoint(event)
            progress("Фразы: %d / %d" % (len(rows) + 1, count), len(rows), count)
            options = {}
            if "text_model" in args:
                options["model"] = args["text_model"]
            if args.get("followup"):
                options["followup"] = True
            text = texts[len(rows)] if args.get("system_text") else self.engine.text(category, event, progress, **options)
            delivery = self.engine.direction(text, category, event, progress, **({"use_text_model": False} if args.get("system_text") else {})) if model == "tts-cosyvoice3" and not args.get("system_voice") else None
            rows.append(self.store.add(text, category, "system" if args.get("system_voice") else model, self.engine.profile_hash, day, daily and settings["daily_auto_keep"], delivery, bool(args.get("followup")), settings["speech_language"], builtin=bool(args.get("system_text"))))
        for i, row in enumerate(rows):
            checkpoint(event)
            progress("Записи: %d / %d" % (i + 1, len(rows)), i, len(rows))
            if not args.get("system_voice") and (not row["audio_path"] or not (self.store.root / "audio" / row["audio_path"]).is_file()):
                rows[i] = self.engine.speech(row, model, event, progress)
        if daily:
            self.store.plan(day, count, "ready")
        return {"day": day, "count": len(rows), "ids": [r["id"] for r in rows]}

    def prepare(self, args, event, progress):
        day = args["day"]
        self.store.plan(day)  # Validate before writing anything.
        self.store.expire_audio(day)
        tasks = args["tasks"]
        if not isinstance(tasks, list) or len(tasks) > 1000:
            raise ValueError("Invalid preparation plan")
        for task in tasks:
            if not isinstance(task, dict) or task.get("category") not in CATEGORIES or not isinstance(task.get("slot"), str) or len(task["slot"]) > 160 or not isinstance(task.get("followup", False), bool):
                raise ValueError("Invalid preparation slot")
            if args.get("system_text") and (not isinstance(task.get("text"), str) or not 4 <= len(task["text"].strip()) <= 400):
                raise ValueError("Invalid ready-made phrase")
        settings = self.store.settings()
        configuration = {key: settings[key] for key in ("speech_language", "tts_model", "text_model", "voice_profile", "character_description", "cosy_style", "cosy_instruction", "delivery_catalog")}
        if args.get("automatic") and not settings["auto_daily"]:
            return {"ready": 0, "total": 0, "pending": 0, "disabled": True}
        configuration.update(system_text=bool(args.get("system_text")), system_voice=bool(args.get("system_voice")))
        scope = hashlib.sha256(json.dumps(configuration, sort_keys=True).encode()).hexdigest()[:16]
        prefix = day + "/" + scope + "/"
        supplied = {prefix + task["slot"]: task.get("text", "") for task in tasks}
        with self.store.lock, self.store.db:
            for task in tasks:
                self.store.db.execute("INSERT OR IGNORE INTO preparation(slot,day,category,followup) VALUES(?,?,?,?)", (prefix + task["slot"], day, task["category"], int(task.get("followup", False))))
            if args.get("retry"):
                self.store.db.execute("UPDATE preparation SET retry_at=0 WHERE slot LIKE ? AND state<>'ready'", (prefix + "%",))
            for ready_task in self.store.db.execute("SELECT slot,phrase_id,state FROM preparation WHERE slot LIKE ? AND phrase_id IS NOT NULL", (prefix + "%",)).fetchall():
                if ready_task["slot"] not in supplied:
                    continue
                existing = self.store.db.execute("SELECT rejected,deleted,audio_path,used_at FROM phrases WHERE id=?", (ready_task["phrase_id"],)).fetchone()
                if not existing or existing["deleted"] or existing["rejected"]:
                    self.store.db.execute("UPDATE preparation SET phrase_id=NULL,state='pending',retry_at=0 WHERE slot=?", (ready_task["slot"],))
                elif ready_task["state"] == "ready" and not existing["used_at"] and not args.get("system_voice") and (not existing["audio_path"] or not (self.store.root / "audio" / existing["audio_path"]).is_file()):
                    self.store.db.execute("UPDATE preparation SET state='audio_pending',retry_at=0 WHERE slot=?", (ready_task["slot"],))
            pending = {row["slot"]: dict(row) for row in self.store.db.execute("SELECT * FROM preparation WHERE slot LIKE ? AND state<>'ready' AND retry_at<=?", (prefix + "%", time.time()))}
            work = [pending[slot] for slot in supplied if slot in pending][:2]
        for task in work:
            checkpoint(event)
            ident = task["phrase_id"]
            try:
                if ident:
                    try:
                        row = self.store.get(ident)
                    except ValueError:
                        ident = None
                if not ident:
                    if hasattr(self.engine, "refresh_voice"):
                        self.engine.refresh_voice()
                    if args.get("system_text"):
                        text = supplied[task["slot"]]
                    elif task["followup"]:
                        text = self.engine.text(task["category"], event, progress, followup=True)
                    else:
                        text = self.engine.text(task["category"], event, progress)
                    delivery = self.engine.direction(text, task["category"], event, progress, **({"use_text_model": False} if args.get("system_text") else {})) if not args.get("system_voice") and settings["tts_model"] == "tts-cosyvoice3" else None
                    row = self.store.add(text, task["category"], "system" if args.get("system_voice") else settings["tts_model"], self.engine.profile_hash, day, settings["daily_auto_keep"], delivery, bool(task["followup"]), settings["speech_language"], builtin=bool(args.get("system_text")))
                    ident = row["id"]
                    with self.store.lock, self.store.db:
                        self.store.db.execute("UPDATE preparation SET phrase_id=?,state='audio_pending' WHERE slot=?", (ident, task["slot"]))
                if not args.get("system_voice") and (not row["audio_path"] or not (self.store.root / "audio" / row["audio_path"]).is_file()):
                    self.engine.speech(row, settings["tts_model"], event, progress)
                with self.store.lock, self.store.db:
                    self.store.db.execute("UPDATE preparation SET state='ready',error='' WHERE slot=?", (task["slot"],))
            except Cancelled:
                raise
            except Exception as error:
                # Retain the good text when only speech failed. Avoid a hot retry loop.
                attempts = task["attempts"] + 1
                delay = min(1800, 30 * 2 ** min(attempts - 1, 6))
                with self.store.lock, self.store.db:
                    self.store.db.execute("UPDATE preparation SET attempts=?,retry_at=?,error=? WHERE slot=?", (attempts, time.time() + delay, str(error)[-1500:], task["slot"]))
        with self.store.lock:
            total = len(supplied)
            ready = sum(row["slot"] in supplied for row in self.store.db.execute("SELECT slot FROM preparation WHERE slot LIKE ? AND state='ready'", (prefix + "%",)))
        return {"ready": ready, "total": total, "pending": total - ready}

    def repair(self, args, event, progress):
        """Retry retained runtime text after failed speech, including after restart."""
        day = args["day"]
        self.store.plan(day)
        settings = self.store.settings()
        if hasattr(self.engine, "refresh_voice"):
            self.engine.refresh_voice()
        candidates = [row for row in reversed(self.store.rows(day)) if row["approved"] and not row["used_at"] and (not row["audio_path"] or not (self.store.root / "audio" / row["audio_path"]).is_file()) and row["model"] == settings["tts_model"] and row["profile"] == self.engine.profile_hash and row["language"] == settings["speech_language"]]
        completed = 0
        attempted = 0
        for row in candidates:
            checkpoint(event)
            slot = "runtime-repair/" + row["id"]
            with self.store.lock, self.store.db:
                self.store.db.execute("INSERT OR IGNORE INTO preparation(slot,day,category,followup,phrase_id) VALUES(?,?,?,?,?)", (slot, day, row["category"], row["followup"], row["id"]))
                task = dict(self.store.db.execute("SELECT * FROM preparation WHERE slot=?", (slot,)).fetchone())
            if task["retry_at"] > time.time() and not args.get("retry"):
                continue
            if attempted >= 2:
                break
            attempted += 1
            try:
                self.engine.speech(row, settings["tts_model"], event, progress)
                with self.store.lock, self.store.db:
                    self.store.db.execute("UPDATE preparation SET state='ready',error='' WHERE slot=?", (slot,))
                completed += 1
            except Cancelled:
                raise
            except Exception as error:
                attempts = task["attempts"] + 1
                with self.store.lock, self.store.db:
                    self.store.db.execute("UPDATE preparation SET attempts=?,retry_at=?,error=? WHERE slot=?", (attempts, time.time()+min(1800,30*2**min(attempts-1,6)), str(error)[-1500:], slot))
        return {"repaired": completed, "pending": len(candidates)-completed}
