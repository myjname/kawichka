"""Legacy/worker preset boundaries; synthetic data, no ML/audio dependencies."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile
from unittest.mock import MagicMock, patch
from types import SimpleNamespace

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'backend'))
from kavychka_local.profiles import Profiles, inside

class ProfileBoundaryTests(unittest.TestCase):
    def test_voice_archive_normalization_preserves_distinct_inputs_and_roundtrips(self):
        import hashlib
        import shutil
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as temp:
            root = Path(temp)
            profiles = Profiles(ROOT, root/'data')
            source = root/'voice.kavychka'
            contents = [b'first synthetic reference', b'second synthetic reference']
            item = dict(version=1, kind='voice', name='Two references', references=[
                dict(file='first.flac', transcript='First reference.'),
                dict(file='normalized0.flac', transcript='Second reference.')])
            with zipfile.ZipFile(source, 'w') as archive:
                archive.writestr('preset.json', json.dumps(item))
                archive.writestr('first.flac', contents[0])
                archive.writestr('normalized0.flac', contents[1])
                archive.writestr('unrelated.wav', b'not a declared reference')
            expected = [hashlib.sha256(value).hexdigest() for value in contents]
            # The decoder is a byte copy here; this checks file lifetime and
            # aliasing independently of ffmpeg/soundfile and audio quality.
            with patch.object(Profiles, 'normalize_audio', new=staticmethod(shutil.copyfile)), patch.object(Profiles, 'validate'):
                imported = profiles.import_file('voice', source)
                self.assertEqual([ref['hash'] for ref in profiles.voice(imported['id'])], expected)
                exported = root/'exported.kavychka'
                profiles.export(imported['id'], 'voice', exported)
                repeated = profiles.import_file('voice', exported)
                self.assertEqual([ref['hash'] for ref in profiles.voice(repeated['id'])], expected)
            self.assertEqual(len(profiles.list()), 2)
            self.assertEqual(len(list(profiles.root.iterdir())), 2)
            for result in [imported, repeated]:
                folder, saved = profiles.get(result['id'], 'voice')
                self.assertEqual([ref['transcript'] for ref in saved['references']], ['First reference.', 'Second reference.'])
                self.assertEqual({p.name for p in folder.iterdir()}, {'preset.json', 'normalized0.flac', 'normalized1.flac'})

    def test_fallback_audio_decoder_rejects_long_audio_before_reading_samples(self):
        reader = MagicMock()
        reader.__enter__.return_value = reader
        reader.frames, reader.samplerate, reader.channels = 16000 * 1800, 16000, 2
        soundfile = SimpleNamespace(SoundFile=MagicMock(return_value=reader),
                                   read=MagicMock(side_effect=AssertionError("unbounded decode")))
        with patch.dict(sys.modules, {"soundfile": soundfile, "imageio_ffmpeg": None}), patch("shutil.which", return_value=None):
            with self.assertRaises(ValueError): Profiles.normalize_audio("long.flac", "out.flac")
        reader.blocks.assert_not_called()

    def test_fallback_audio_decoder_streams_stereo_as_mono_in_bounded_blocks(self):
        reader, output, block = MagicMock(), MagicMock(), MagicMock()
        reader.__enter__.return_value = reader
        output.__enter__.return_value = output
        reader.frames, reader.samplerate, reader.channels = 48000, 24000, 2
        reader.blocks.return_value = iter([block, block])
        block.mean.return_value = [0.25, -0.25]
        soundfile = SimpleNamespace(SoundFile=MagicMock(side_effect=[reader, output]))
        with patch.dict(sys.modules, {"soundfile": soundfile, "imageio_ffmpeg": None}), patch("shutil.which", return_value=None):
            Profiles.normalize_audio("stereo.wav", "out.flac")
        self.assertLessEqual(reader.blocks.call_args.kwargs["blocksize"] * reader.channels, 16384)
        self.assertEqual(output.write.call_count, 2)
        self.assertEqual(output.write.call_args.args, ([0.25, -0.25],))
        self.assertEqual(soundfile.SoundFile.call_args.kwargs["channels"], 1)
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
