"""Validate a Rust release handoff without executing or publishing any payload."""
import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import stat
import struct
import zipfile

MAGIC = b'KAVYCHKA-NEXT-01'
LIMIT = 500_000_000
TAG = re.compile(r'next-v((?:0|[1-9][0-9]{0,7})\.(?:0|[1-9][0-9]{0,7})\.(?:0|[1-9][0-9]{0,7}))\Z')
SETUP = 'Kavychka-Next-Setup.exe'
PORTABLE = 'Kavychka-Next-Portable.zip'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def inspect_setup(data, version):
    if not 88 <= len(data) <= LIMIT or not data.startswith(b'MZ') or data[-16:] != MAGIC:
        raise ValueError('Invalid embedded setup envelope')
    length = struct.unpack('<Q', data[-24:-16])[0]
    if not 0 < length <= len(data) - 88:
        raise ValueError('Invalid payload length')
    payload = data[-88-length:-88]
    if digest(payload).encode('ascii') != data[-88:-24]:
        raise ValueError('Payload digest mismatch')
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        info = archive.infolist()
        names = [f.filename for f in info]
        if len(info) > 8001 or len(set(n.casefold() for n in names)) != len(info):
            raise ValueError('Invalid or duplicate archive entries')
        if any(f.file_size > 200_000_000 or stat.S_ISLNK(f.external_attr >> 16) for f in info) or sum(f.file_size for f in info) > LIMIT:
            raise ValueError('Payload size/link limits exceeded')
        if archive.getinfo('package.json').file_size > 8_000_000:
            raise ValueError('Manifest exceeds limit')
        manifest = json.loads(archive.read('package.json'))
        if manifest['format'] != 1 or manifest['platform'] != 'windows-x64' or manifest['version'] != version:
            raise ValueError('Tag does not match embedded package version/platform')
        expected = ['package.json']
        total = 0
        for entry in manifest['files']:
            name = entry['path']
            path = PurePosixPath(name)
            if not name or '\\' in name or ':' in name or path.is_absolute() or any(p in ('', '.', '..') for p in name.split('/')):
                raise ValueError('Unsafe manifest path')
            if not 0 <= entry['bytes'] <= 200_000_000 or not re.fullmatch('[0-9a-fA-F]{64}', entry['sha256']):
                raise ValueError('Invalid manifest entry')
            if archive.getinfo(name).file_size != entry['bytes']:
                raise ValueError('Manifest size mismatch')
            body = archive.read(name)
            if digest(body) != entry['sha256'].lower():
                raise ValueError('Manifest file digest mismatch: ' + name)
            total += entry['bytes']
            expected.append(name)
        if total > LIMIT or sorted(expected) != sorted(names):
            raise ValueError('Manifest and archive contents differ')
        required = {'app.exe','Kavychka.exe','next/worker.py','backend/kavychka_local/engine.py','assets/components.json','assets/character.json','assets/voice/profile.json'}
        if not required <= set(expected):
            raise ValueError('Missing required runtime components')
        app = archive.read('app.exe')
        launcher = archive.read('Kavychka.exe')
        if not app.startswith(b'MZ') or data[:-88-length] != app:
            raise ValueError('Setup executable stub differs from verified app.exe')
        if not launcher.startswith(b'MZ'):
            raise ValueError('Invalid packaged launcher')
        return dict(version=version, files=len(manifest['files']), payload_sha256=digest(payload), launcher_sha256=digest(launcher))


def verify(tag, setup, portable, output):
    match = TAG.fullmatch(tag)
    if not match:
        raise ValueError('Use an exact stable next-vX.Y.Z tag')
    setup, portable, output = Path(setup), Path(portable), Path(output)
    if setup.name != SETUP or portable.name != PORTABLE or output.exists() or output.resolve() == (setup.parent/'SHA256SUMS.txt').resolve():
        raise ValueError('Fixed asset names and fresh evidence path required')
    if not 0 < setup.stat().st_size <= LIMIT or not 0 < portable.stat().st_size <= LIMIT:
        raise ValueError('Package exceeds limit')
    data = setup.read_bytes()
    report = inspect_setup(data, match[1])
    with zipfile.ZipFile(portable) as archive:
        names = archive.namelist()
        if len(names) != 4 or set(names) != {'Portable.exe','setup.exe','portable.owner','README.txt'}:
            raise ValueError('Invalid portable layout')
        if any(f.file_size > LIMIT for f in archive.infolist()) or sum(f.file_size for f in archive.infolist()) > LIMIT:
            raise ValueError('Portable member exceeds limit')
        if digest(archive.read('setup.exe')) != digest(data):
            raise ValueError('Portable contains a different installer')
        if archive.read('portable.owner') != b'Kavychka.Next.Portable.v1\n' or digest(archive.read('Portable.exe')) != report['launcher_sha256']:
            raise ValueError('Invalid portable owner/launcher')
    checksums = {SETUP: digest(data), PORTABLE: digest(portable.read_bytes())}
    sums = ''.join(value + '  ' + name + '\n' for name,value in checksums.items())
    checksum_path = setup.parent / 'SHA256SUMS.txt'
    if checksum_path.exists() and checksum_path.read_text(encoding='utf-8') != sums:
        raise ValueError('Existing release checksums do not match; refusing to replace evidence')
    report.update(passed=True, tag=tag, artifacts=checksums, installer_executed=False, release_published=False)
    output.parent.mkdir(parents=True,exist_ok=True)
    checksum_path.write_text(sums,encoding='utf-8',newline='\n')
    output.write_text(json.dumps(report,indent=2),encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--setup', required=True, type=Path)
    parser.add_argument('--portable', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(args.tag,args.setup,args.portable,args.output)))
