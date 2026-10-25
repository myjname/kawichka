"""Release handoff rejects wrong tags, damaged payloads and mixed artifacts."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('verify_release', ROOT / 'next/tools/verify_release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def setup_bytes(version='1.2.3'):
    names = ['app.exe','Kavychka.exe','next/worker.py','backend/kavychka_local/engine.py','assets/components.json','assets/character.json','assets/voice/profile.json']
    body = b'MZ public synthetic fixture, never executed'
    manifest = dict(format=1,version=version,platform='windows-x64',files=[dict(path=n,bytes=len(body),sha256=hashlib.sha256(body).hexdigest()) for n in names])
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer,'w',zipfile.ZIP_DEFLATED) as archive:
        archive.writestr('package.json',json.dumps(manifest))
        for name in names:archive.writestr(name,body)
    payload=buffer.getvalue()
    return body + payload + hashlib.sha256(payload).hexdigest().encode() + struct.pack('<Q',len(payload)) + release.MAGIC


class NextReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(dir=ROOT/'work')
        self.root=Path(self.temp.name)
        self.setup=self.root/release.SETUP
        self.portable=self.root/release.PORTABLE
        self.output=self.root/'result.json'
        self.setup.write_bytes(setup_bytes())
        self.make_portable(self.setup.read_bytes())

    def tearDown(self):self.temp.cleanup()

    def make_portable(self, data):
        with zipfile.ZipFile(self.portable,'w') as archive:
            for name,body in [('setup.exe',data),('Portable.exe',b'MZ public synthetic fixture, never executed'),('portable.owner',b'Kavychka.Next.Portable.v1\n'),('README.txt',b'fixture')]:
                archive.writestr(name,body)

    def verify(self, tag='next-v1.2.3'):
        return release.verify(tag,self.setup,self.portable,self.output)

    def test_valid_handoff_and_existing_checksums_are_rechecked(self):
        result=self.verify()
        self.assertTrue(result['passed'])
        self.assertEqual(result['version'],'1.2.3')
        self.assertEqual(set(result['artifacts']),{release.SETUP,release.PORTABLE})
        self.assertTrue((self.root/'SHA256SUMS.txt').is_file())
        self.output.unlink()
        self.assertTrue(self.verify()['passed'])
        self.output.unlink()
        (self.root/'SHA256SUMS.txt').write_text('different release evidence',encoding='utf-8')
        with self.assertRaisesRegex(ValueError,'checksums'):self.verify()

    def test_invalid_or_mismatched_tag_cannot_prepare_a_release(self):
        for tag in ['v1.2.3','next-v01.2.3','next-v1.2.3-beta','next-v1.2.3\n','next-v1.2.3/other','NEXT-v1.2.3','next-v9.8.7']:
            with self.subTest(tag=tag),self.assertRaises(ValueError):self.verify(tag)
        self.assertFalse(self.output.exists())

    def test_corrupt_payload_cannot_be_covered_by_fresh_checksums(self):
        data=bytearray(self.setup.read_bytes());data[100]^=1;self.setup.write_bytes(data)
        with self.assertRaisesRegex(ValueError,'digest'):self.verify()
        self.assertFalse((self.root/'SHA256SUMS.txt').exists())

    def test_portable_from_another_version_is_rejected(self):
        self.make_portable(setup_bytes('1.2.4'))
        with self.assertRaisesRegex(ValueError,'different installer'):self.verify()
        self.assertFalse(self.output.exists())

    def test_changed_native_stubs_are_rejected_before_checksums(self):
        data=self.setup.read_bytes()
        length=struct.unpack('<Q',data[-24:-16])[0]
        self.setup.write_bytes(b'MZ wrong executable'+data[-88-length:])
        with self.assertRaisesRegex(ValueError,'stub'):self.verify()
        self.assertFalse((self.root/'SHA256SUMS.txt').exists())
        self.setup.write_bytes(data)
        with zipfile.ZipFile(self.portable,'w') as archive:
            for name,body in [('setup.exe',data),('Portable.exe',b'MZ wrong launcher'),('portable.owner',b'Kavychka.Next.Portable.v1\n'),('README.txt',b'fixture')]:archive.writestr(name,body)
        with self.assertRaisesRegex(ValueError,'launcher'):self.verify()
        self.assertFalse((self.root/'SHA256SUMS.txt').exists())


if __name__=='__main__':unittest.main()
