"""Exercise the built CLI on synthetic private fixtures; never user data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import uuid


def hashes(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob('*')) if p.is_file()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--workspace', type=Path, required=True)
    args = parser.parse_args()
    project = Path(__file__).resolve().parents[2]
    workspace = args.workspace.resolve()
    if not workspace.is_relative_to((project / 'work').resolve()):
        raise ValueError('Smoke workspace must be inside project/work')
    workspace.mkdir(parents=True, exist_ok=True)
    root = workspace / ('cli-' + uuid.uuid4().hex)
    source = root / 'Исходник с пробелами'
    source.mkdir(parents=True)
    (source / 'audio').mkdir()
    (source / 'audio/ready.flac').write_bytes(b'fLaC synthetic preservation fixture')
    (source / 'settings.json').write_text(json.dumps({
        'SchemaVersion': 2, 'BackendSettingsMigrated': True,
        'BackendDailySettingsMigrated': True, 'TextModel': 'text-qwen35-4b',
        'ThemeMode': 'dark', 'SpeechLanguage': 'ru',
        'ApiKey': 'fixture-secret',
    }), encoding='utf-8-sig')
    database = source / 'library.sqlite3'
    db = sqlite3.connect(database)
    try:
        db.executescript((project / 'next/crates/storage/src/legacy_fixture.sql').read_text(encoding='utf-8'))
        db.execute('INSERT INTO settings VALUES(?,?)', ('text_model', json.dumps('text-gemma4-12b')))
        db.execute('INSERT INTO settings VALUES(?,?)', ('api_token', json.dumps('fixture-secret')))
        db.commit()
    finally:
        db.close()
    # Abrupt process exit retains committed WAL without a live writer.
    helper = '''import sqlite3,os,sys
db=sqlite3.connect(sys.argv[1])
db.executescript("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO phrases(id,text,normalized,category,model,profile,created_at) VALUES('wal-cli','WAL smoke','wal-cli','break','builtin','default','2026-10-23');")
os._exit(0)
'''
    subprocess.run([sys.executable, '-c', helper, str(database)], check=True)
    assert database.with_name(database.name + '-wal').is_file()
    before = hashes(source)

    def run(*command, success=True):
        result = subprocess.run([str(args.exe.resolve()), *map(str, command)], capture_output=True,
                                text=True, encoding='utf-8', timeout=60)
        if (result.returncode == 0) != success:
            raise AssertionError(f'Unexpected CLI exit {result.returncode}: {result.stderr}')
        return json.loads(result.stdout) if success else result.stderr

    inspected = run('check-source', '--source', source)
    assert inspected['live_storage_switched'] is False
    target = root / 'Пробная копия'
    first = run('preview-import', '--source', source, '--destination', target)
    assert first['phase'] == 'ready' and first['rows']['phrases'] == 3
    assert first['models_copied'] is False and first['live_storage_switched'] is False
    assert hashes(source) == before
    db = sqlite3.connect(target / 'library.snapshot.sqlite3')
    try:
        settings = json.loads(db.execute('SELECT json FROM app_settings').fetchone()[0])
        assert settings['desktop']['TextModel'] == 'text-qwen35-4b'
        assert settings['desktop']['ThemeMode'] == 'dark'
        assert db.execute("SELECT text FROM phrases WHERE id='wal-cli'").fetchone()[0] == 'WAL smoke'
    finally:
        db.close()
    assert b'fixture-secret' not in (target / 'library.snapshot.sqlite3').read_bytes()
    second = run('preview-import', '--source', source, '--destination', target)
    assert first == second and hashes(source) == before
    unrelated = root / 'Unrelated'
    unrelated.mkdir()
    (unrelated / 'keep.txt').write_text('keep', encoding='utf-8')
    run('preview-import', '--source', source, '--destination', unrelated, success=False)
    assert (unrelated / 'keep.txt').read_text(encoding='utf-8') == 'keep'
    limited = root / 'Limited'
    run('preview-import', '--source', source, '--destination', limited, '--max-bytes', 1, success=False)
    assert not limited.exists()
    assert hashes(source) == before
    (source / 'audio/ready.flac').write_bytes(b'changed synthetic recording')
    run('preview-import', '--source', source, '--destination', target, success=False)
    report = {
        'passed': True, 'fixture_only': True, 'wal_preserved': True,
        'original_bytes_unchanged_during_operations': True,
        'manual_settings_preserved': True, 'credentials_excluded': True,
        'repeat_idempotent': True, 'unrelated_destination_preserved': True,
        'over_limit_rejected': True, 'changed_source_rejected': True,
        'playback_tested': False, 'live_storage_switched': False,
    }
    (root / 'result.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps({'evidence': str(root / 'result.json'), **report}, indent=2))


if __name__ == '__main__':
    main()
