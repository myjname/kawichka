"""Verify a stopped backend's copied database and remap only paths inside its old root."""
import argparse
import json
import sqlite3
from contextlib import closing
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--source", type=Path, required=True)
parser.add_argument("--target", type=Path, required=True)
args = parser.parse_args()
source, target = args.source.resolve(), args.target.resolve()
with closing(sqlite3.connect(target / "library.sqlite3")) as db:
    if db.execute("PRAGMA quick_check").fetchone()[0] != "ok":
        raise ValueError("Copied database integrity check failed")
    row = db.execute("SELECT value FROM settings WHERE key='model_paths'").fetchone()
    if row:
        paths = json.loads(row[0])
        for key, value in paths.items():
            path = Path(value).resolve()
            if path == source or source in path.parents:
                paths[key] = str(target / path.relative_to(source))
        db.execute("UPDATE settings SET value=? WHERE key='model_paths'", (json.dumps(paths),))
    db.commit()
    db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
