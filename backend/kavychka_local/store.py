import hashlib
import json
import re
import unicodedata
import sqlite3
import threading
import uuid
from datetime import datetime, timezone
from pathlib import Path
from .character import DEFAULT_DESCRIPTION, MAX_DESCRIPTION
from .diversity import normalize, too_similar, same_opening
from .delivery import CHOICES, MAX_INSTRUCTION, DEFAULT_CATALOG, validate_catalog

CATEGORIES = ("break", "breakfast", "lunch", "dinner", "exercise")
TEXT_MODELS = ("text-gemma4-12b", "text-gemma4-e4b", "text-qwen35-4b", "text-qwen35-9b",
               "text-ministral3-8b", "text-qvikhr3-8b", "text-saiga-llama3-8b")
VOICE_MODELS = ("tts-cosyvoice3", "tts-0.6b", "tts-1.7b")
DEFAULTS = {"tts_model": "tts-cosyvoice3", "runtime": False, "batch_count": 13,
            "categories": list(CATEGORIES), "auto_daily": True, "daily_auto_keep": True,
            "swear": False, "device": "auto", "model_paths": {}, "character_description": DEFAULT_DESCRIPTION,
            "text_model": "text-gemma4-12b", "runtime_text_model": "text-gemma4-12b",
            "cosy_style": "auto", "cosy_instruction": "", "delivery_catalog": DEFAULT_CATALOG,
            "speech_language": "ru", "voice_profile": "", "reminder_followup": False,
            "address_form": "neutral", "user_name": ""}

def now():
    return datetime.now(timezone.utc).isoformat()

def cache_key(text, model, profile, revision, seed=42, direction="", language="ru"):
    data = [unicodedata.normalize("NFC", text).strip(), model, profile, revision, seed, "Russian" if language == "ru" else "English", "rms-0.1-v1"]
    if direction:
        data.append(direction)
    return hashlib.sha256(json.dumps(data, ensure_ascii=False).encode()).hexdigest()

class Store:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.root.mkdir(parents=True, exist_ok=True)
        (self.root / "audio").mkdir(exist_ok=True)
        self.lock = threading.RLock()
        self._builtin_stamps = {}
        self.builtin_warnings = []
        database = self.root / "library.sqlite3"
        existed = database.exists()
        self.db = sqlite3.connect(database, check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        if existed and self.db.execute("PRAGMA table_info(phrases)").fetchall() and "followup" not in {row[1] for row in self.db.execute("PRAGMA table_info(phrases)")}:
            backup = self.root / "backups/library-before-v2.sqlite3"
            if not backup.exists():
                backup.parent.mkdir(exist_ok=True)
                snapshot = sqlite3.connect(backup)
                try:
                    self.db.backup(snapshot)
                finally:
                    snapshot.close()
        self.db.executescript("""
        CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS phrases(
          id TEXT PRIMARY KEY,text TEXT NOT NULL,normalized TEXT UNIQUE NOT NULL,
          category TEXT NOT NULL,model TEXT NOT NULL,profile TEXT NOT NULL,
          cache_key TEXT,audio_path TEXT,approved INTEGER NOT NULL DEFAULT 0,
          day TEXT,used_at TEXT,created_at TEXT NOT NULL,deleted INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX IF NOT EXISTS queue ON phrases(day,category,approved,used_at,deleted);
        CREATE TABLE IF NOT EXISTS plans(day TEXT PRIMARY KEY,target INTEGER NOT NULL,status TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS voice_rotation(id INTEGER PRIMARY KEY CHECK(id=1),next_index INTEGER NOT NULL);
        """)
        if "seed" not in {row[1] for row in self.db.execute("PRAGMA table_info(phrases)")}:
            self.db.execute("ALTER TABLE phrases ADD COLUMN seed INTEGER NOT NULL DEFAULT 42")
        for column in ("voice_style", "voice_instruction", "voice_reference"):
            if column not in {row[1] for row in self.db.execute("PRAGMA table_info(phrases)")}:
                self.db.execute("ALTER TABLE phrases ADD COLUMN " + column + " TEXT NOT NULL DEFAULT ''")
        for column, definition in (("followup", "INTEGER NOT NULL DEFAULT 0"), ("language", "TEXT NOT NULL DEFAULT 'ru'"), ("rejected", "INTEGER NOT NULL DEFAULT 0")):
            if column not in {row[1] for row in self.db.execute("PRAGMA table_info(phrases)")}:
                self.db.execute("ALTER TABLE phrases ADD COLUMN " + column + " " + definition)
        self.db.execute("CREATE TABLE IF NOT EXISTS preparation(slot TEXT PRIMARY KEY,day TEXT NOT NULL,category TEXT NOT NULL,followup INTEGER NOT NULL DEFAULT 0,phrase_id TEXT,state TEXT NOT NULL DEFAULT 'pending',attempts INTEGER NOT NULL DEFAULT 0,retry_at REAL NOT NULL DEFAULT 0,error TEXT NOT NULL DEFAULT '')")
        self.db.commit()
        if self.db.execute("SELECT 1 FROM settings WHERE key='delivery_catalog'").fetchone() is None:
            with self.db:
                self.db.execute("INSERT INTO settings VALUES('delivery_catalog',?)", (json.dumps(DEFAULT_CATALOG),))
                self.db.execute("INSERT OR REPLACE INTO settings VALUES('cosy_style',?)", (json.dumps("auto"),))

    def settings(self):
        with self.lock:
            result = dict(DEFAULTS)
            for row in self.db.execute("SELECT key,value FROM settings"):
                result[row["key"]] = json.loads(row["value"])
            return result

    def configure(self, values):
        allowed = set(DEFAULTS)
        if set(values) - allowed:
            raise ValueError("Неизвестные настройки")
        current = self.settings()
        current.update(values)
        if current["speech_language"] not in ("ru", "en") or current["address_form"] not in ("neutral", "female", "male"):
            raise ValueError("Unsupported language or address form")
        if not isinstance(current["voice_profile"], str) or (current["voice_profile"] and not current["voice_profile"].isalnum()):
            raise ValueError("Invalid voice preset")
        if not isinstance(current["user_name"], str) or len(current["user_name"]) > 80 or not isinstance(current["reminder_followup"], bool):
            raise ValueError("Invalid personalization")
        if current["tts_model"] not in VOICE_MODELS:
            raise ValueError("Неизвестная модель озвучки")
        if current["text_model"] not in TEXT_MODELS or current["runtime_text_model"] not in TEXT_MODELS:
            raise ValueError("Неизвестная текстовая модель")
        validate_catalog(current["delivery_catalog"])
        if current["cosy_style"] not in ("auto", "custom", *(item["id"] for item in current["delivery_catalog"])):
            raise ValueError("Неизвестная подача CosyVoice")
        instruction = current["cosy_instruction"]
        if (not isinstance(instruction, str) or len(instruction) > MAX_INSTRUCTION or "<|" in instruction
                or (current["cosy_style"] == "custom" and not instruction.strip())):
            raise ValueError("Инструкция подачи: от 1 до %d символов, без специальных токенов" % MAX_INSTRUCTION)
        if type(current["batch_count"]) is not int or not 1 <= current["batch_count"] <= 100:
            raise ValueError("Количество фраз: от 1 до 100")
        if not current["categories"] or not set(current["categories"]) <= set(CATEGORIES):
            raise ValueError("Выберите категории")
        for name in ("runtime", "auto_daily", "daily_auto_keep", "swear"):
            if not isinstance(current[name], bool):
                raise ValueError("Некорректное значение переключателя")
        if current["device"] not in ("auto", "cpu", "cuda"):
            raise ValueError("Некорректное устройство")
        description = current["character_description"]
        if not isinstance(description, str) or not description.strip() or len(description) > MAX_DESCRIPTION:
            raise ValueError("Описание персонажа: от 1 до %d символов" % MAX_DESCRIPTION)
        if not isinstance(current["model_paths"], dict) or set(current["model_paths"]) - {*VOICE_MODELS, "text-1.7b", *TEXT_MODELS} or any(not isinstance(v, str) for v in current["model_paths"].values()):
            raise ValueError("Некорректные пути моделей")
        with self.lock, self.db:
            for key, value in values.items():
                self.db.execute("INSERT OR REPLACE INTO settings VALUES(?,?)", (key, json.dumps(value)))
        return self.settings()

    def rows(self, day=None):
        with self.lock:
            query = "SELECT * FROM phrases WHERE deleted=0"
            args = []
            if day:
                query += " AND day=?"
                args.append(day)
            return [dict(row) for row in self.db.execute(query + " ORDER BY created_at DESC", args)]

    def get(self, ident):
        with self.lock:
            row = self.db.execute("SELECT * FROM phrases WHERE id=? AND deleted=0", (ident,)).fetchone()
            if row is None:
                raise ValueError("Фраза не найдена")
            return dict(row)

    def known(self, text):
        self.import_builtin_history()
        with self.lock:
            if self.db.execute("SELECT 1 FROM phrases WHERE normalized=? OR normalized LIKE ?", (normalize(text), normalize(text) + " #builtin %")).fetchone() is not None:
                return True
            # Include excluded/deleted rows: regenerating a rejected template does not improve it.
            recent = self.db.execute("SELECT text FROM phrases ORDER BY created_at DESC LIMIT 500").fetchall()
            return any(too_similar(text, row[0]) or (i < 8 and same_opening(text, row[0])) for i, row in enumerate(recent))

    def recent(self, limit=30):
        self.import_builtin_history()
        with self.lock:
            return [row[0] for row in self.db.execute("SELECT text FROM phrases ORDER BY created_at DESC LIMIT ?", (limit,))]

    def import_builtin_history(self):
        """Carry native no-model history into diversity checks, never the queue."""
        directory = self.root / "builtin-days"
        if not directory.is_dir() or directory.is_symlink() or getattr(directory, "is_junction", lambda: False)():
            return
        with self.lock:
            for path in sorted(directory.glob("*.json")):
                if not re.fullmatch(r"\d{4}-\d{2}-\d{2}\.(ru|en)\.json", path.name) or path.is_symlink():
                    continue
                try:
                    info = path.stat()
                    stamp = (info.st_mtime_ns, info.st_size)
                    if self._builtin_stamps.get(path.name) == stamp:
                        continue
                    if info.st_size > 32_000_000:
                        raise ValueError("built-in history file is too large")
                    day = json.loads(path.read_text(encoding="utf-8-sig"))
                    if day.get("Version") != 1 or day.get("Language") not in ("ru", "en") or path.name != day.get("Day", "") + "." + day["Language"] + ".json" or not isinstance(day.get("Rows"), list) or len(day["Rows"]) > 50000:
                        raise ValueError("invalid built-in day")
                    datetime.strptime(day["Day"], "%Y-%m-%d")
                    with self.db:
                        for row in day["Rows"]:
                            if not isinstance(row, dict) or not re.fullmatch(r"stock-[0-9a-f]{32}", str(row.get("Id", ""))) or row.get("Category") not in CATEGORIES or not isinstance(row.get("Text"), str) or not 4 <= len(row["Text"].strip()) <= 400 or not isinstance(row.get("Followup"), bool):
                                raise ValueError("invalid built-in phrase")
                            created = row.get("CreatedAt", "")
                            datetime.fromisoformat(created.replace("Z", "+00:00"))
                            used = row.get("UsedAt") or None
                            if used is not None:
                                datetime.fromisoformat(used.replace("Z", "+00:00"))
                            ident = row["Id"]
                            if row.get("Kind") == "external" and self.db.execute("SELECT 1 FROM phrases WHERE normalized=?", (normalize(row["Text"]),)).fetchone():
                                continue
                            self.db.execute("INSERT OR IGNORE INTO phrases(id,text,normalized,category,model,profile,day,approved,used_at,created_at,deleted,followup,language) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)", (ident, row["Text"].strip(), normalize(row["Text"]) + " #builtin " + ident, row["Category"], "builtin", "builtin-v1", day["Day"], 0, used, created, 1, int(row["Followup"]), day["Language"]))
                    self._builtin_stamps[path.name] = stamp
                except (OSError, ValueError, TypeError, AttributeError) as error:
                    warning = path.name + ": " + str(error)
                    if warning not in self.builtin_warnings:
                        self.builtin_warnings.append(warning)

    def add(self, text, category, model, profile, day=None, approved=False, delivery=None, followup=False, language=None, builtin=False):
        if category not in CATEGORIES or not isinstance(text, str) or not 4 <= len(text.strip()) <= 400:
            raise ValueError("Некорректная фраза")
        ident = uuid.uuid4().hex
        with self.lock, self.db:
            self.db.execute("INSERT INTO phrases(id,text,normalized,category,model,profile,day,approved,created_at) VALUES(?,?,?,?,?,?,?,?,?)",
                            (ident, text.strip(), normalize(text) + (" #builtin " + ident if builtin else ""), category, model, profile, day, int(approved), now()))
            self.db.execute("UPDATE phrases SET followup=?,language=? WHERE id=?", (int(followup), language or self.settings()["speech_language"], ident))
            if delivery:
                self.db.execute("UPDATE phrases SET voice_style=?,voice_instruction=? WHERE id=?",
                                (delivery["voice_style"], delivery["voice_instruction"], ident))
            elif model != "tts-cosyvoice3":
                self.db.execute("UPDATE phrases SET voice_style='',voice_instruction='' WHERE id=?", (ident,))
        return self.get(ident)

    def assign_reference(self, ident, references):
        with self.lock, self.db:
            row = self.db.execute("SELECT voice_reference FROM phrases WHERE id=? AND deleted=0", (ident,)).fetchone()
            if row is None:
                raise ValueError("Фраза не найдена")
            if row[0] in references:
                return row[0]
            self.db.execute("INSERT OR IGNORE INTO voice_rotation VALUES(1,0)")
            index = self.db.execute("SELECT next_index FROM voice_rotation WHERE id=1").fetchone()[0]
            chosen = references[index % len(references)]
            self.db.execute("UPDATE voice_rotation SET next_index=next_index+1 WHERE id=1")
            self.db.execute("UPDATE phrases SET voice_reference=? WHERE id=?", (chosen, ident))
            return chosen

    def audio(self, ident, model, key, filename, seed=42, delivery=None):
        path = self.root / "audio" / filename
        if path.parent != self.root / "audio" or not path.is_file():
            raise ValueError("Аудиофайл отсутствует")
        with self.lock, self.db:
            self.db.execute("UPDATE phrases SET model=?,cache_key=?,audio_path=?,seed=? WHERE id=?", (model, key, filename, seed, ident))
            if delivery:
                self.db.execute("UPDATE phrases SET voice_style=?,voice_instruction=? WHERE id=?",
                                (delivery["voice_style"], delivery["voice_instruction"], ident))
            elif model != "tts-cosyvoice3":
                self.db.execute("UPDATE phrases SET voice_style='',voice_instruction='' WHERE id=?", (ident,))
        return self.get(ident)

    def select(self, ids, action):
        if action not in ("keep", "exclude", "delete") or not isinstance(ids, list) or len(ids) > 1000:
            raise ValueError("Некорректное действие")
        with self.lock, self.db:
            for ident in ids:
                if action == "delete":
                    self.db.execute("UPDATE phrases SET deleted=1 WHERE id=?", (ident,))
                else:
                    self.db.execute("UPDATE phrases SET approved=?,rejected=? WHERE id=? AND deleted=0", (int(action == "keep"), int(action == "exclude"), ident))

    def consume(self, category, day, followup=False, language=None, allow_text=False):
        # Selection and reservation are one transaction: concurrent timers cannot repeat a phrase.
        with self.lock, self.db:
            query = "SELECT * FROM phrases WHERE category=? AND day=? AND followup=? AND language=? AND approved=1 AND used_at IS NULL AND deleted=0" + ("" if allow_text else " AND audio_path IS NOT NULL") + " ORDER BY created_at"
            rows = self.db.execute(query, (category, day, int(followup), language or self.settings()["speech_language"])).fetchall()
            for row in rows:
                if (allow_text and not row["audio_path"]) or (row["audio_path"] and (self.root / "audio" / row["audio_path"]).is_file()):
                    self.db.execute("UPDATE phrases SET used_at=? WHERE id=?", (now(), row["id"]))
                    return self.get(row["id"])
        return None

    def claim(self, ident, allow_text=False):
        with self.lock, self.db:
            row = self.db.execute("SELECT * FROM phrases WHERE id=? AND approved=1 AND used_at IS NULL AND deleted=0", (ident,)).fetchone()
            if row and ((allow_text and not row["audio_path"]) or (row["audio_path"] and (self.root / "audio" / row["audio_path"]).is_file())):
                self.db.execute("UPDATE phrases SET used_at=? WHERE id=?", (now(), ident))
                return self.get(ident)
        return None

    def plan(self, day, target=None, status=None):
        datetime.strptime(day, "%Y-%m-%d")
        with self.lock, self.db:
            if target is not None:
                self.db.execute("INSERT INTO plans VALUES(?,?,?) ON CONFLICT(day) DO UPDATE SET target=excluded.target,status=excluded.status", (day, target, status))
            row = self.db.execute("SELECT * FROM plans WHERE day=?", (day,)).fetchone()
            return dict(row) if row else None

    def expire_audio(self, day):
        datetime.strptime(day, "%Y-%m-%d")
        with self.lock, self.db:
            count = self.db.execute("SELECT COUNT(*) FROM phrases WHERE audio_path IS NOT NULL AND COALESCE(day,substr(created_at,1,10)) < ?", (day,)).fetchone()[0]
            self.db.execute("UPDATE phrases SET audio_path=NULL,cache_key=NULL WHERE COALESCE(day,substr(created_at,1,10)) < ?", (day,))
            removed = self.cleanup()
        return {"expired_rows": count, "removed_bytes": removed}

    def clear_library(self):
        with self.lock, self.db:
            count = self.db.execute("SELECT COUNT(*) FROM phrases").fetchone()[0]
            self.db.execute("DELETE FROM phrases")
            self.db.execute("DELETE FROM plans")
            self.db.execute("DELETE FROM preparation")
            directory = self.root / "builtin-days"
            if directory.is_dir() and not directory.is_symlink() and not getattr(directory, "is_junction", lambda: False)():
                for path in directory.glob("*.json"):
                    if re.fullmatch(r"\d{4}-\d{2}-\d{2}\.(ru|en)\.json", path.name) and not path.is_symlink():
                        path.unlink()
            self._builtin_stamps.clear()
        return {"removed_rows": count, "removed_bytes": self.cleanup()}

    def cleanup(self):
        # Candidate/history rows retain text for deduplication, but only live files are protected.
        with self.lock:
            live = {r[0] for r in self.db.execute("SELECT audio_path FROM phrases WHERE deleted=0 AND audio_path IS NOT NULL")}
        removed = 0
        for path in (self.root / "audio").iterdir():
            if path.is_file() and path.name not in live and path.suffix in (".flac", ".tmp", ".wav"):
                removed += path.stat().st_size
                path.unlink()
        return removed

    def stats(self):
        files = list((self.root / "audio").glob("*.flac"))
        return {"audio_bytes": sum(f.stat().st_size for f in files), "audio_files": len(files)}
