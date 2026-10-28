"""Legacy/worker preset boundaries; synthetic data, no ML/audio dependencies."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'backend'))
from kavychka_local.profiles import Profiles, inside

class ProfileBoundaryTests(unittest.TestCase):
    def test_complete_character_resolves_voice_without_mutating_other_components(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as temp:
            profiles=Profiles(ROOT,Path(temp));folder=profiles.root/'examplepack';folder.mkdir()
            (folder/'clip.wav').write_bytes(b'RIFF prepared fixture')
            item=dict(version=1,kind='character_pack',name='Example',description='Neutral reminder companion.',references=[dict(file='clip.wav',transcript='Reference text.')],visual=dict(file='pet.gif',layout='pixel'),delivery='auto',instruction='')
            original=json.dumps(item);(folder/'preset.json').write_text(original,encoding='utf-8')
            refs=profiles.voice('examplepack');self.assertEqual(len(refs),1);self.assertEqual(refs[0]['transcript'],'Reference text.');self.assertEqual(refs[0]['path'],folder/'clip.wav');self.assertEqual((folder/'preset.json').read_text(encoding='utf-8'),original)
            item['references']=[dict(file='../escape.wav')];(folder/'preset.json').write_text(json.dumps(item),encoding='utf-8')
            with self.assertRaises(ValueError):profiles.voice('examplepack')

    def test_padded_manifest_is_rejected_without_publishing_or_leaking_stage(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as temp:
            root=Path(temp);profiles=Profiles(ROOT,root/'data');source=root/'large.kavychka'
            item=dict(version=1,kind='character',name='Fixture',description='A short description.')
            with zipfile.ZipFile(source,'w',zipfile.ZIP_DEFLATED) as archive:
                archive.writestr('preset.json',json.dumps(item)+' '*160_000)
            with self.assertRaises(ValueError):profiles.import_file('character',source)
            self.assertEqual(list(profiles.root.iterdir()),[])

    def test_malformed_list_entry_does_not_hide_other_presets(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as temp:
            profiles=Profiles(ROOT,Path(temp));bad=profiles.root/'bad';bad.mkdir();(bad/'preset.json').write_text('[]')
            good=profiles.root/'good';good.mkdir();(good/'preset.json').write_text(json.dumps(dict(version=1,kind='character',name='Good',description='Text')))
            self.assertEqual([row['id'] for row in profiles.list()],['good'])

    def test_paths_reject_windows_aliases_streams_and_controls(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as temp:
            for name in ['clip.wav.','clip.wav ','NUL.wav','voice.wav:stream','folder/../x','a\x00.wav','a?.wav']:
                with self.subTest(name=name),self.assertRaises(ValueError):inside(Path(temp),name)
            self.assertEqual(inside(Path(temp),'обычный голос.wav'),Path(temp)/'обычный голос.wav')

    def test_valid_character_roundtrip_drops_unrelated_fields(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as temp:
            root=Path(temp);profiles=Profiles(ROOT,root/'data');source=root/'valid.kavychka'
            with zipfile.ZipFile(source,'w',zipfile.ZIP_DEFLATED) as archive:
                archive.writestr('preset.json',json.dumps(dict(version=1,kind='character',name='Fixture',description='Neutral description',ignored_credential='synthetic-value')))
            result=profiles.import_file('character',source)
            folder,item=profiles.get(result['id'],'character');self.assertEqual(item['description'],'Neutral description');self.assertNotIn('ignored_credential',item)
            exported=root/'out.kavychka';profiles.export(result['id'],'character',exported)
            with zipfile.ZipFile(exported) as archive:self.assertNotIn('synthetic-value',archive.read('preset.json').decode())

    def test_plain_description_accepts_4000_utf8_characters_but_bounds_large_input(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as temp:
            root=Path(temp);profiles=Profiles(ROOT,root/'data');source=root/'description.txt'
            source.write_bytes(b'\xef\xbb\xbf'+('\U0001f642'*4000).encode())
            result=profiles.import_file('character',source)
            self.assertEqual(len(profiles.get(result['id'],'character')[1]['description']),4000)
            source.write_bytes(b'x'*200_000)
            with self.assertRaises(ValueError):profiles.import_file('character',source)
            self.assertEqual(len(profiles.list()),1)

if __name__=='__main__':unittest.main()
