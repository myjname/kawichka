"""Check the reviewed P1 contract against sources; no data/model/network access.

Use --render only after reviewing contract changes, then commit the resulting
Markdown with the JSON. Ordinary verification never writes anything.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sqlite3

ROOT = Path(__file__).resolve().parents[1]


def load(path):
    return json.loads((ROOT / path).read_text(encoding="utf-8"))


def quoted_array(source, name):
    found = re.search(r"(?:pub )?const " + name + r":.*?= &\[(.*?)\];", source, re.S)
    assert found, name
    return re.findall(r'"([^"]+)"', found[1])


def field_table(fields):
    lines = ["| Источник | Поле | Режим | Назначение / правило |",
             "|---|---|---|---|"]
    for section in ("desktop", "backend"):
        for key, mode, target, _value in fields[section]:
            lines.append(f"| {section} | {key} | {mode} | {target} |")
    return "\n".join(lines)


def model_table(compat):
    lines = ["| ID модели | Формат | Закреплённая ревизия | Байт весов/файлов |",
             "|---|---|---|---|"]
    for key, spec in sorted(compat["models"].items()):
        lines.append(f"| {key} | {spec['format']} | {spec['revision']} | {spec['bytes']} |")
    lines.append("| system | Windows SAPI | Установленные системные RU/EN-голоса | 0 |")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--render", action="store_true")
    args = parser.parse_args()
    fields = load("next/contracts/p1-fields.json")
    compat = load("next/contracts/p1-compatibility.json")
    assert fields["contract"] == compat["contract"] == 2
    legacy = (ROOT / "next/crates/core/src/legacy.rs").read_text(encoding="utf-8")
    for section, name in (("desktop", "DESKTOP_FIELDS"), ("backend", "INFERENCE_FIELDS")):
        keys = [row[0] for row in fields[section]]
        assert len(keys) == len(set(keys)), f"duplicate {section} key"
        assert set(keys) == set(quoted_array(legacy, name)), f"unreviewed {section} field"
    manifest_bytes = (ROOT / "assets/components.json").read_bytes()
    manifest = json.loads(manifest_bytes)
    assert hashlib.sha256(manifest_bytes).hexdigest() == compat["components_sha256"], "review catalog/runtime changes and update the P1 pin"
    models = {key: {"format": spec.get("format", "transformers"),
                    "revision": spec["revision"], "bytes": sum(f["size"] for f in spec["files"])}
              for key, spec in manifest["models"].items()}
    assert compat["models"] == models, "model tuple differs"
    generation = (ROOT / "next/crates/core/src/generation.rs").read_text(encoding="utf-8")
    assert set(models) == set(quoted_array(generation, "TEXT_MODELS") + quoted_array(generation, "VOICE_MODELS")) - {"system"}
    with sqlite3.connect(":memory:") as db:
        db.executescript((ROOT / "next/crates/storage/src/legacy_fixture.sql").read_text(encoding="utf-8"))
        tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        assert tables == set(compat["legacy_columns"])
        for table, columns in compat["legacy_columns"].items():
            assert [r[1] for r in db.execute(f'PRAGMA table_info("{table}")')] == columns, table
    docpath = ROOT / "docs/RUST-SLINT-P1-CONTRACT.md"
    doc = docpath.read_text(encoding="utf-8")
    for label, body in (("FIELDS", field_table(fields)), ("MODELS", model_table(compat))):
        start, end = f"<!-- P1-{label}-BEGIN -->", f"<!-- P1-{label}-END -->"
        assert doc.count(start) == doc.count(end) == 1
        before, rest = doc.split(start)
        present, after = rest.split(end)
        replacement = "\n" + body + "\n"
        if args.render:
            doc = before + start + replacement + end + after
        else:
            assert present == replacement, f"stale {label} table: review then --render"
    if args.render:
        docpath.write_text(doc, encoding="utf-8")
    print(json.dumps({"contract": 2, "desktop_fields": len(fields["desktop"]),
                      "backend_fields": len(fields["backend"]), "models": len(models) + 1,
                      "legacy_tables": len(tables), "network": False,
                      "user_data_opened": False, "rendered": args.render}, ensure_ascii=False))


if __name__ == "__main__":
    main()
