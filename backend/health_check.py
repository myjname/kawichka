"""Offline update check: migrate SQLite without loading models or installing anything."""
import argparse
import json
import sqlite3
from pathlib import Path

from kavychka_local.store import Store


def check(data):
    store = Store(data)
    try:
        result = store.db.execute("PRAGMA integrity_check").fetchall()
        if len(result) != 1 or result[0][0] != "ok":
            raise sqlite3.DatabaseError("Library integrity check failed")
        settings = store.settings()
        # Reading older/custom settings must not replace them with v2 defaults.
        if not isinstance(settings, dict):
            raise ValueError("Invalid library preferences")
        store.db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        return {"ok": True, "phrases": len(store.rows())}
    finally:
        store.db.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", required=True)
    args = parser.parse_args()
    print(json.dumps(check(Path(args.data).resolve())))
