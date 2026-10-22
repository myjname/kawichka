"""Import legacy local databases into shared storage, without resetting playback history."""
import argparse
import json
import os
import sqlite3
import sys
import uuid
from contextlib import closing
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from kavychka_local.store import DEFAULTS, TEXT_MODELS, VOICE_MODELS, Store

def model_paths(value, source, target):
    result = {}
    if not isinstance(value, dict):
        return result
    for key, raw in value.items():
        if key not in (*VOICE_MODELS, "text-1.7b", *TEXT_MODELS) or not isinstance(raw, str):
            continue
        path = Path(raw).resolve()
        if path == source or source in path.parents:
            # Default model paths will now resolve inside the common directory.
            continue
        result[key] = raw  # Explicit developer overrides remain explicit.
    return result

def migrate(target, sources):
    target = Path(target).resolve()
    target.mkdir(parents=True, exist_ok=True)
    sources = [Path(s).resolve() for s in sources if Path(s).resolve() != target]
    database = target / "library.sqlite3"
    imported = 0
    warnings = []
    for source in sources:
        previous = source / "library.sqlite3"
        if not previous.exists():
            continue
        # Packaged Codex is a component recovery source, not the active user's
        # library. Preserve a damaged snapshot, but do not block installed models.
        redirected = (
            source.name == "Kavychka" and source.parent.name == "Local"
            and source.parent.parent.name == "LocalCache"
            and source.parent.parent.parent.name == "OpenAI.Codex_2p2nqsd0c76g0")
        if redirected:
            try:
                with closing(sqlite3.connect(previous.as_uri() + "?mode=ro", uri=True)) as probe:
                    if probe.execute("PRAGMA quick_check").fetchone()[0] != "ok":
                        raise sqlite3.DatabaseError("quick_check failed")
            except sqlite3.DatabaseError:
                warnings.append("Не удалось импортировать историю из повреждённой библиотеки Codex. Оригинал сохранён: " + str(previous))
                continue
        with closing(sqlite3.connect(previous.as_uri() + "?mode=ro", uri=True)) as old:
            old.row_factory = sqlite3.Row
            if old.execute("PRAGMA quick_check").fetchone()[0] != "ok":
                raise ValueError("Старая библиотека повреждена; оригинал сохранён: " + str(previous))
            if not database.exists():
                temporary = target / "library.storage-copy.sqlite3"
                temporary.unlink(missing_ok=True)
                with closing(sqlite3.connect(temporary)) as snapshot:
                    old.backup(snapshot)  # Includes committed WAL rows in a consistent snapshot.
                os.replace(temporary, database)
            current = Store(target)
            try:
                with current.lock, current.db:
                    for row in old.execute("SELECT * FROM phrases"):
                        row = dict(row)
                        if row.get("audio_path") and Path(row["audio_path"]).name != row["audio_path"]:
                            raise ValueError("Неверный путь аудиофайла в старой библиотеке")
                        existing = current.db.execute("SELECT * FROM phrases WHERE normalized=?", (row["normalized"],)).fetchone()
                        if existing:
                            if existing["day"] == row["day"] and row.get("used_at"):
                                current.db.execute("UPDATE phrases SET used_at=COALESCE(used_at,?) WHERE id=?", (row["used_at"], existing["id"]))
                            if row.get("deleted"):
                                current.db.execute("UPDATE phrases SET deleted=1 WHERE id=?", (existing["id"],))
                            continue
                        if current.db.execute("SELECT 1 FROM phrases WHERE id=?", (row["id"],)).fetchone():
                            row["id"] = uuid.uuid4().hex
                        columns = {r[1] for r in current.db.execute("PRAGMA table_info(phrases)")}
                        names = [key for key in row if key in columns]
                        current.db.execute("INSERT INTO phrases(" + ",".join(names) + ") VALUES(" + ",".join("?" for _ in names) + ")", [row[k] for k in names])
                        imported += 1
                    for row in old.execute("SELECT * FROM plans"):
                        current.db.execute("INSERT OR IGNORE INTO plans(day,target,status) VALUES(?,?,?)", (row["day"], row["target"], row["status"]))
                    for key, raw in old.execute("SELECT key,value FROM settings"):
                        if key not in DEFAULTS:
                            continue
                        value = json.loads(raw)
                        if key == "model_paths":
                            value = model_paths(value, source, target)
                        current.db.execute("INSERT OR IGNORE INTO settings VALUES(?,?)", (key, json.dumps(value)))
                    # A cloned database already contains settings: remap its local model paths too.
                    paths = current.settings()["model_paths"]
                    current.db.execute("UPDATE settings SET value=? WHERE key='model_paths'", (json.dumps(model_paths(paths, source, target)),))
                    for key, in current.db.execute("SELECT key FROM settings").fetchall():
                        if key not in DEFAULTS:
                            current.db.execute("DELETE FROM settings WHERE key=?", (key,))
                if current.db.execute("PRAGMA quick_check").fetchone()[0] != "ok":
                    raise ValueError("Не пройдена проверка перенесённой библиотеки")
                current.db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            finally:
                current.db.close()
    if warnings:
        (target / "migration-warnings.json").write_text(json.dumps(warnings, ensure_ascii=False), encoding="utf-8")
    return {"imported_rows": imported, "sources": len(sources), "warnings": warnings}

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True)
    parser.add_argument("--sources", required=True, help="Path to a JSON list of legacy directories")
    args = parser.parse_args()
    print(json.dumps(migrate(args.target, json.loads(Path(args.sources).read_text(encoding="utf-8-sig")))), flush=True)
