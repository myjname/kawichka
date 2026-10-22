import json
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
(ROOT / "work").mkdir(exist_ok=True)
sys.path.insert(0, str(ROOT / "backend"))
from migrate_storage import migrate
from kavychka_local.store import Store

class MigrationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "work")
        self.root = Path(self.temp.name)
        self.open = []

    def tearDown(self):
        for store in self.open:
            store.db.close()
        self.temp.cleanup()

    def store(self, name):
        store = Store(self.root / name)
        self.open.append(store)
        return store

    def test_corrupt_codex_snapshot_does_not_block_components_or_current_library(self):
        source = self.root / "Packages/OpenAI.Codex_2p2nqsd0c76g0/LocalCache/Local/Kavychka"
        source.mkdir(parents=True)
        previous = source / "library.sqlite3"
        previous.write_bytes(b"damaged snapshot")
        current = self.store("shared")
        row = current.add("Отложи работу на минуту.", "break", "tts-cosyvoice3", "profile", "2026-10-19", True)
        result = migrate(current.root, [source])
        self.assertEqual(1, len(result["warnings"]))
        self.assertEqual(b"damaged snapshot", previous.read_bytes())
        self.assertEqual(row["id"], current.get(row["id"])["id"])
        self.assertTrue((current.root / "migration-warnings.json").is_file())

    def test_corrupt_regular_library_still_blocks_import(self):
        source = self.root / "legacy"
        source.mkdir()
        (source / "library.sqlite3").write_bytes(b"damaged snapshot")
        with self.assertRaises(sqlite3.DatabaseError):
            migrate(self.root / "shared", [source])

    def test_snapshot_includes_wal_and_preserves_history_settings_and_plans(self):
        old = self.store("legacy")
        row = old.add("Пора сделать перерыв.", "break", "tts-0.6b", "profile", "2026-10-19", True)
        (old.root / "audio/test.flac").write_bytes(b"audio")
        old.audio(row["id"], "tts-0.6b", "hash", "test.flac")
        used = old.consume("break", "2026-10-19")
        old.plan("2026-10-19", 13, "ready")
        old.configure({"runtime": True, "swear": False, "character_description": "Мой сохранённый персонаж.", "model_paths": {"tts-0.6b": str(old.root / "models/tts-0.6b")}})
        target = self.root / "shared"
        migrate(target, [old.root])
        new = Store(target)
        try:
            self.assertEqual(used["used_at"], new.get(row["id"])["used_at"])
            self.assertTrue(new.get(row["id"])["approved"])
            self.assertEqual("ready", new.plan("2026-10-19")["status"])
            self.assertTrue(new.settings()["runtime"])
            self.assertFalse(new.settings()["swear"])
            self.assertEqual("Мой сохранённый персонаж.", new.settings()["character_description"])
            self.assertEqual({}, new.settings()["model_paths"])
            self.assertEqual("ok", new.db.execute("PRAGMA quick_check").fetchone()[0])
        finally:
            new.db.close()

    def test_existing_shared_preferences_win_and_merge_is_idempotent(self):
        old, current = self.store("legacy"), self.store("shared")
        a = old.add("Пора пообедать.", "lunch", "tts-0.6b", "profile", "2026-10-19", True)
        b = current.add(a["text"], "lunch", "tts-0.6b", "profile", "2026-10-19", False)
        c = old.add("Новая зарядка.", "exercise", "tts-0.6b", "profile", "2026-10-19", True)
        current.configure({"batch_count": 7, "runtime": True})
        old.configure({"batch_count": 99, "runtime": False})
        with old.db:
            old.db.execute("UPDATE phrases SET used_at='previously-played' WHERE id=?", (a["id"],))
        migrate(current.root, [old.root]); migrate(current.root, [old.root])
        self.assertEqual(2, len(current.rows()))
        self.assertFalse(current.get(b["id"])["approved"])
        self.assertEqual("previously-played", current.get(b["id"])["used_at"])
        self.assertEqual(7, current.settings()["batch_count"])
        self.assertTrue(current.settings()["runtime"])
        self.assertEqual(c["text"], current.get(c["id"])["text"])

    def test_invalid_database_does_not_replace_existing_library(self):
        current = self.store("shared")
        row = current.add("Сохранённая фраза.", "break", "tts-0.6b", "profile")
        source = self.root / "broken"; source.mkdir()
        (source / "library.sqlite3").write_bytes(b"broken")
        with self.assertRaises(sqlite3.DatabaseError):
            migrate(current.root, [source])
        self.assertEqual(row["text"], current.get(row["id"])["text"])

if __name__ == "__main__":
    unittest.main(verbosity=2)
