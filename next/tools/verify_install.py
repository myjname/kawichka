"""Opt-in native package QA. Writes only inside a new work fixture.

Uses the given legacy source for copying/reading, never as live --data-dir.
Requires an interactive Windows session; --real-models also runs inference/audio.
No shortcuts, global locator, autostart or release publication are changed.
"""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()


def run(evidence, name, args, timeout=300):
    print(name, flush=True)
    with (evidence / (name + '.stdout.log')).open('wb') as out, (evidence / (name + '.stderr.log')).open('wb') as err:
        subprocess.run([str(a) for a in args], check=True, timeout=timeout, stdout=out, stderr=err,
                       creationflags=subprocess.CREATE_NO_WINDOW)


def snapshot(data):
    # This is the closed, newly created QA database, never the original source.
    with closing(sqlite3.connect(data / 'shell.sqlite3')) as db:
        assert db.execute('pragma integrity_check').fetchone()[0] == 'ok'
        prefs = json.loads(db.execute('select json from preferences where id=1').fetchone()[0])
        rows = db.execute('select * from phrases order by id').fetchall()
    files = {}
    for part in ('audio', 'presets'):
        folder = data / part
        if folder.exists():
            for f in folder.rglob('*'):
                if f.is_file():
                    files[str(f.relative_to(data))] = digest(f)
    return dict(preferences=prefs, phrases=rows, files=files)


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--setup', type=Path, required=True)
    parser.add_argument('--update', type=Path, required=True)
    parser.add_argument('--legacy-source', type=Path)
    parser.add_argument('--real-models', action='store_true')
    args = parser.parse_args()
    if args.real_models and not args.legacy_source:
        parser.error('--real-models requires an explicit legacy source')
    evidence = ROOT / 'work' / ('installed-package-' + uuid.uuid4().hex)
    evidence.mkdir(parents=True)
    print(evidence, flush=True)
    program, common = evidence / 'Программа с пробелом', evidence / 'Общие данные'
    command = [args.setup.resolve(), '--apply-embedded', program, common]
    if args.legacy_source:
        command += ['--legacy-source', args.legacy_source.resolve()]
    run(evidence, 'install', command)
    current = json.loads((program / 'current.json').read_text('utf-8'))
    first_version = current['version']
    data = Path(current['data'])
    assert data == common / '.kavychka-next'
    first_app = program / 'versions' / first_version / 'app.exe'
    run(evidence, 'version-1', [first_app, '--version'])
    assert (evidence / 'version-1.stdout.log').read_text().strip() == first_version
    launcher_hash = digest(program / 'Kavychka.exe')
    before = snapshot(data)
    run(evidence, 'update', [args.update.resolve(), '--apply-embedded', program, common])
    updated = json.loads((program / 'current.json').read_text('utf-8'))
    assert updated['version'] != first_version and updated['data'] == current['data']
    run(evidence, 'version-2', [program / 'versions' / updated['version'] / 'app.exe', '--version'])
    assert (evidence / 'version-2.stdout.log').read_text().strip() == updated['version']
    assert snapshot(data) == before, 'Update changed settings, phrase rows or media'
    assert digest(program / 'Kavychka.exe') == launcher_hash, 'Running stable dispatcher was replaced'
    if args.real_models:
        # Explicit QA overrides only this disposable copy; an unrelated
        # fullscreen app must not cancel the revoice operation under test.
        with closing(sqlite3.connect(data / 'shell.sqlite3')) as db:
            prefs=json.loads(db.execute('select json from preferences where id=1').fetchone()[0])
            prefs['desktop']['defer_generation_fullscreen']=False
            prefs['generation']['ac_only']=False
            db.execute('update preferences set json=? where id=1',(json.dumps(prefs,ensure_ascii=False),))
            db.commit()
        latest_app=program / 'versions' / updated['version'] / 'app.exe'
        run(evidence, 'inference', [latest_app, '--data-dir', data, '--legacy-source', args.legacy_source.resolve(),
                                  '--models-library-test', evidence / 'inference'], 1500)
        run(evidence, 'library-updated', [program / 'Kavychka.exe', '--verify-library', evidence / 'library-updated'], 900)
    # An explicit edit after the update must survive in a separate rescue copy.
    with closing(sqlite3.connect(data / 'shell.sqlite3')) as db:
        prefs = json.loads(db.execute('select json from preferences where id=1').fetchone()[0])
        prefs['user_name'] = 'QA edit after update'
        db.execute('update preferences set json=? where id=1', (json.dumps(prefs, ensure_ascii=False),))
        db.commit()
    after = snapshot(data)
    run(evidence, 'rollback', [program / 'Kavychka.exe', '--rollback-only'])
    assert json.loads((program / 'current.json').read_text())['version'] == first_version
    assert snapshot(data) == before, 'Rollback did not restore the verified old generation'
    rescues = list((program / 'recovery').glob('after-failed-*'))
    assert len(rescues) == 1 and snapshot(rescues[0]) == after, 'New work was not preserved separately'
    assert not any(p.name in ('models', 'components') for p in (program / 'recovery').rglob('*'))
    assert not (data / 'components' / 'models').exists(), 'Verification unexpectedly downloaded model weights'
    summary = dict(passed=True, versions=[first_version, updated['version']], real_installer_exe=True,
                   stable_launcher_delegated=True, settings_rows_audio_preserved=True, rollback_verified=True,
                   new_work_preserved_separately=True, models_copied=False, models_downloaded=False,
                   global_integration_changed=False, real_inference_and_native_ui=args.real_models,
                   setup_sha256=digest(args.setup), update_sha256=digest(args.update),
                   original_rows_before_update=len(before['phrases']), completed_unix=int(time.time()))
    (evidence / 'result.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(summary, ensure_ascii=False), flush=True)


if __name__ == '__main__':
    main()
