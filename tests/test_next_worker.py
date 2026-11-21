"""Stateless boundary regressions; no installed ML dependencies are imported."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("next_worker", ROOT / "next/worker.py")
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)

class WorkerContractTests(unittest.TestCase):
    def test_llama_environment_fallback_uses_the_qualified_executable(self):
        import json
        from kavychka_local.engine import Engine
        cases = [('obsolete', False, True, False, False),
                 ('current-cpu', False, False, False, False),
                 ('current-cpu', False, True, True, True),
                 ('current-cpu', False, True, False, False)]
        for revision, own_gpu, own_executable, external_gpu, want_gpu in cases:
            with self.subTest(revision=revision, own_executable=own_executable,
                              external_gpu=external_gpu, want_gpu=want_gpu), tempfile.TemporaryDirectory(dir=ROOT/'work') as directory:
                base = Path(directory)
                app, library, external = base/'app', base/'library', base/'external'
                job, managed = library/'jobs/job-test', library/'components'
                job.mkdir(parents=True)
                (app/'assets').mkdir(parents=True)
                manifest = dict(models={}, llama_cpu_runtime=dict(revision='current-cpu'),
                                llama_runtime=dict(revision='current-gpu'))
                (app/'assets/components.json').write_text(json.dumps(manifest))
                required = ['ggml-base.dll', 'ggml-cpu-x64.dll', 'ggml.dll', 'llama.dll',
                            'llama-common.dll', 'llama-server-impl.dll', 'libomp.dll']
                for runtime, saved_revision, gpu, executable in [
                        (managed, revision, own_gpu, own_executable),
                        (external, 'current-gpu' if external_gpu else 'current-cpu', external_gpu, True)]:
                    folder = runtime/'llama'
                    folder.mkdir(parents=True)
                    for name in required + (['ggml-cuda.dll', 'cublas64_13.dll', 'cublasLt64_13.dll'] if gpu else []) + (['llama-server.exe'] if executable else []):
                        (folder/name).write_bytes(b'fixture; never executed')
                    (folder/'verified.json').write_text(json.dumps(dict(revision=saved_revision, gpu=gpu)))
                captured = []
                def capture(engine, app, store, components):
                    captured.append(components)
                request = dict(operation='generate', work_dir=str(job), library_root=str(library),
                               components_root=str(external), settings=dict(device='cuda' if want_gpu else 'cpu', tts_model='system'), tasks=[])
                with patch.object(worker, 'APP', app), patch.object(Engine, '__init__', capture), patch.object(Engine, 'unload'), patch.object(worker, 'emit'):
                    worker.run(request)
                    catalog = captured[0]
                    self.assertTrue(catalog.llama_ready(gpu=want_gpu))
                    own_ready = revision == 'current-cpu' and own_executable and not want_gpu
                    expected = (managed if own_ready else external)/'llama/llama-server.exe'
                    self.assertEqual(catalog.llama_executable(), expected)

    def test_encouragement_is_text_only_and_never_publishes_phrase_or_audio(self):
        from kavychka_local.engine import Engine
        with tempfile.TemporaryDirectory(dir=ROOT / 'work') as directory:
            library=Path(directory); job=library/'jobs/job-support';job.mkdir(parents=True)
            events=[]; texts=[]
            def text(engine, category, event, progress, **options):
                self.assertEqual(category,'support')
                self.assertIs(options['style_retries'],False)
                self.assertEqual(engine.store.settings()['tts_model'],'system')
                value='Небольшой шаг сегодня тоже считается. Вариант '+str(len(texts)+1)
                texts.append(value)
                return value
            request=dict(operation='support',work_dir=str(job),library_root=str(library),components_root=str(library/'components'),settings=dict(tts_model='system',text_enabled=True),day='2026-10-28')
            with patch.object(Engine,'text',text),patch.object(Engine,'speech',side_effect=AssertionError('Support must not synthesize audio')),patch.object(Engine,'direction',side_effect=AssertionError('Support has no acting stage')),patch.object(worker,'emit',side_effect=lambda kind,**data:events.append((kind,data))):
                worker.run(request)
            self.assertEqual(events[-1],('result',dict(texts=texts)))
            self.assertEqual(len(texts),4)
            self.assertTrue(all(kind in ('stage','result') for kind,_ in events))
            self.assertEqual(list((job/'audio').iterdir()),[])

    def test_retained_environment_top_level_link_is_rejected_without_reading_its_contents(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as directory:
            root=Path(directory);retained=root/'runtime-unverified-123-456';retained.mkdir()
            original=Path.lstat
            def info(path,*args,**kwargs):
                value=original(path,*args,**kwargs)
                return SimpleNamespace(st_mode=value.st_mode,st_file_attributes=0x400) if path==retained else value
            with patch.object(Path,'lstat',info):
                with self.assertRaisesRegex(ValueError,'contains a link'):worker.verify_managed_tree(root)

    def test_one_install_completes_cosy_dependencies_reuses_weights_and_reports_phases(self):
        import hashlib,json
        from kavychka_local import downloads
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as directory:
            base=Path(directory); app=base/'app';library=base/'library';job=library/'jobs/job-test';managed=library/'components'
            (app/'assets').mkdir(parents=True);job.mkdir(parents=True);(managed/'runtime').mkdir(parents=True)
            good=b'checked weights';weights=managed/'models/tts-cosyvoice3/weights';weights.parent.mkdir(parents=True);weights.write_bytes(good)
            model=dict(format='cosyvoice3',files=[dict(path='weights',size=len(good),sha256=hashlib.sha256(good).hexdigest())])
            manifest=dict(models={'tts-cosyvoice3':model},runtime_profile='fixture-v1',runtime_versions={'torch':'2.14.1'},wheels={'cpu':[]},cosy_runtime={'revision':'fixture-cosy','files':[]})
            (app/'assets/components.json').write_text(json.dumps(manifest))
            python=managed/'runtime/python.exe';python.write_bytes(b'owned Python fixture')
            calls=[];events=[]
            def write(relative):
                p=managed/relative;p.parent.mkdir(parents=True,exist_ok=True);p.write_text('fixture',encoding='utf-8')
            def install(component,ident,event,progress):
                calls.append(ident);progress('Collecting an internal-pip-package==1.2.3')
                if ident=='runtime':
                    for name in ['torch/__init__.py','qwen_tts/__init__.py','transformers/__init__.py','soundfile.py','tzdata/__init__.py']:write('runtime/Lib/site-packages/'+name)
                    (managed/'runtime-ok.json').write_text(json.dumps(dict(profile='fixture-v1',gpu=False,torch='2.14.1+cpu',versions={'torch':'2.14.1+cpu'})))
                elif ident=='cosy-runtime':
                    for name in ['site/numpy/__init__.py','Whisper/whisper/__init__.py','CosyVoice/cosyvoice/cli/cosyvoice.py','Matcha-TTS/matcha/utils/audio.py']:write('cosy/'+name)
                    (managed/'cosy/verified.json').write_text(json.dumps(dict(profile='fixture-v1',revision='fixture-cosy')))
                else:self.fail('Validated model weights must not be downloaded')
            request=dict(work_dir=str(job),library_root=str(library),components_root=str(base/'legacy'),settings=dict(device='cpu'),operation='install',ids=['tts-cosyvoice3'])
            with patch.object(worker,'APP',app),patch.object(worker.sys,'executable',str(python)),patch.object(downloads.Components,'install',install),patch.object(worker,'emit',side_effect=lambda kind,**data:events.append((kind,data))):
                worker.run(request.copy())
                self.assertEqual(calls,['runtime','cosy-runtime'])
                self.assertTrue(events[-1][1]['ready'])
                self.assertEqual(events[-1][1]['downloaded_models'],[])
                stages=[data['stage'] for kind,data in events if kind=='stage']
                self.assertEqual(stages,['install_check','install_runtime','install_voice_engine','install_verify','install_ready'])
                for kind,data in events:
                    if kind=='progress' and data['detail'].startswith('Collecting'):
                        self.assertNotIn('internal-pip-package',data['message']);self.assertIsNone(data['percent'])
                retained=managed/'runtime-unverified-123-456';retained.mkdir()
                (retained/'preserved-evidence').write_bytes(b'Old environment; never inspect or alter')
                original_iterdir=Path.iterdir
                def active_only(path):
                    if path==retained:self.fail('Retained environment must not be traversed')
                    return original_iterdir(path)
                events.clear();calls.clear()
                with patch.object(Path,'iterdir',active_only):worker.run(request.copy())
                self.assertEqual(calls,[])
                self.assertTrue(events[-1][1]['ready'])
                self.assertEqual((retained/'preserved-evidence').read_bytes(),b'Old environment; never inspect or alter')
                # A marker without its packages must not report a ready environment.
                (managed/'runtime/Lib/site-packages/torch/__init__.py').unlink()
                worker.run(request.copy());self.assertEqual(calls,['runtime'])
                self.assertEqual(weights.read_bytes(),good)

    def test_install_does_not_claim_success_if_dependency_is_still_missing(self):
        import hashlib,json
        from kavychka_local import downloads
        with tempfile.TemporaryDirectory(dir=ROOT/'work') as directory:
            base=Path(directory);app=base/'app';library=base/'library';job=library/'jobs/job-test'
            (app/'assets').mkdir(parents=True);job.mkdir(parents=True)
            good=b'weights';p=library/'components/models/text-fixture/weights';p.parent.mkdir(parents=True);p.write_bytes(good)
            model=dict(format='gguf',files=[dict(path='weights',size=len(good),sha256=hashlib.sha256(good).hexdigest())])
            (app/'assets/components.json').write_text(json.dumps(dict(models={'text-fixture':model},llama_cpu_runtime={'files':[]})))
            request=dict(work_dir=str(job),library_root=str(library),components_root=str(base/'legacy'),settings=dict(device='cpu'),operation='install',ids=['text-fixture'])
            events=[]
            with patch.object(worker,'APP',app),patch.object(downloads.Components,'install'),patch.object(downloads.Components,'llama_ready',return_value=False),patch.object(worker,'emit',side_effect=lambda kind,**data:events.append((kind,data))):
                with self.assertRaisesRegex(ValueError,'Подготовка не завершена'):worker.run(request)
            self.assertFalse(any(kind=='result' for kind,data in events))

    def test_explicit_install_verifies_hash_and_repairs_into_managed_storage_only(self):
        import hashlib
        from kavychka_local import downloads
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            root=Path(directory); app=root/'app'; library=root/'library'; external=root/'external'; job=library/'jobs/job-test'
            (app/'assets').mkdir(parents=True); external.mkdir(); job.mkdir(parents=True)
            good=b'validated model'; (external/'weights').write_bytes(good)
            spec=dict(format='gguf',files=[dict(path='weights',size=len(good),sha256=hashlib.sha256(good).hexdigest())])
            (app/'assets/components.json').write_text(__import__('json').dumps(dict(models={'text-fixture':spec},llama_cpu_runtime={'files':[]})))
            request=dict(work_dir=str(job),library_root=str(library),components_root=str(root/'components'),settings=dict(model_paths={'text-fixture':str(external)}),operation='install',ids=['text-fixture'])
            calls=[]; events=[]
            def install(component,ident,event,progress):
                calls.append(ident); target=component.root/'models'/ident; target.mkdir(parents=True); (target/'weights').write_bytes(good)
            with patch.object(worker,'APP',app),patch.object(downloads.Components,'use_gpu',return_value=False),patch.object(downloads.Components,'llama_ready',return_value=True),patch.object(downloads.Components,'install',install),patch.object(worker,'emit',side_effect=lambda kind,**data:events.append((kind,data))):
                worker.run(request.copy())
                self.assertEqual(calls,[])
                self.assertEqual(events[-1][1]['downloaded_models'],[])
                broken=b'x'*len(good); (external/'weights').write_bytes(broken)
                worker.run(request.copy())
                self.assertEqual(calls,['text-fixture'])
                self.assertEqual(events[-1][1]['downloaded_models'],['text-fixture'])
                self.assertEqual((external/'weights').read_bytes(),broken)
                self.assertEqual((library/'components/models/text-fixture/weights').read_bytes(),good)
                request['repair_environment']=True
                worker.run(request.copy())
                self.assertEqual(calls,['text-fixture','llama-runtime'])
                self.assertEqual((external/'weights').read_bytes(),broken)

    def test_private_cancellation_flag_is_observed_at_stage_boundary(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            root = Path(directory)
            event = worker.TaskCancellation(root)
            self.assertFalse(event.is_set())
            (root / "cancel.request").write_text("cancel")
            self.assertTrue(event.is_set())
            with self.assertRaises(Exception), patch.object(worker, "emit") as emit:
                worker.stage("voice", event)
            emit.assert_not_called()

    def test_text_is_emitted_before_failing_delivery_and_runtime_model_is_used(self):
        from kavychka_local import engine
        calls = []
        class FakeEngine:
            def __init__(self, *args): self.profile_hash = "fixture"
            def text(self, *args, **kwargs):
                calls.append(("text_model", kwargs.get("model"), kwargs.get("style_retries")))
                return "Разомни плечи и отдохни от экрана."
            def direction(self, *args, **kwargs):
                calls.append(("delivery_model", kwargs.get("text_model")))
                raise ValueError("injected delivery failure")
            def unload(self): calls.append(("unload", None))
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            library = Path(directory)
            job = library / "jobs/job-test"
            job.mkdir(parents=True)
            request = dict(work_dir=str(job), library_root=str(library), components_root=str(library),
                           settings=dict(tts_model="tts-cosyvoice3", text_model="text-gemma4-12b", runtime_text_model="text-qwen35-4b"),
                           operation="generate", runtime=True, day="2026-10-24", tasks=[dict(category="exercise")])
            events = []
            with patch.object(engine, "Engine", FakeEngine), patch.object(worker, "emit", side_effect=lambda kind, **values: events.append((kind, values))):
                with self.assertRaisesRegex(ValueError, "injected delivery"):
                    worker.run(request)
            self.assertEqual([kind for kind, _ in events], ["stage", "text", "stage"])
            self.assertEqual(events[1][1]["row"]["text"], "Разомни плечи и отдохни от экрана.")
            self.assertEqual(calls, [("text_model", "text-qwen35-4b", False), ("delivery_model", "text-qwen35-4b"), ("unload", None)])

    def test_extended_windows_and_unc_paths_are_normalized_before_libraries_join_slashes(self):
        self.assertEqual(str(worker.normal_path(r"\\?\C:\Models\voice")), r"C:\Models\voice")
        self.assertEqual(str(worker.normal_path(r"\\?\UNC\server\share\voice")), r"\\server\share\voice")

    def test_snapshot_history_covers_rejected_and_current_batch_text_without_sqlite(self):
        snapshot = worker.Snapshot({"settings": {}, "history": ["Отложи работу и встань на минуту."], "rotation": 2}, Path("private-job"))
        self.assertTrue(snapshot.known("Отложи работу и встань на минуту."))
        snapshot.history.insert(0, "Обед заждался, сделай паузу и поешь.")
        self.assertTrue(snapshot.known("Обед заждался, сделай паузу и поешь."))
        self.assertFalse(hasattr(snapshot.db, "connection"))
        with self.assertRaises(ValueError):
            snapshot.db.execute("UPDATE phrases SET approved=1")

    def test_reference_rotation_is_supplied_and_returned_as_data(self):
        snapshot = worker.Snapshot({"settings": {}, "rotation": 1}, Path("private-job"))
        snapshot.rows["first"] = {}
        snapshot.rows["second"] = {}
        self.assertEqual(snapshot.assign_reference("first", ("one", "two")), "two")
        self.assertEqual(snapshot.assign_reference("first", ("one", "two")), "two")
        self.assertEqual(snapshot.assign_reference("second", ("one", "two")), "one")

    def test_exact_history_outside_recent_window_still_blocks_repetition(self):
        snapshot = worker.Snapshot({"settings": {}, "history": [], "known_texts": ["Отложи работу и встань на минуту."]}, Path("private-job"))
        self.assertTrue(snapshot.known("Отложи работу и встань на минуту!"))

if __name__ == "__main__":
    unittest.main()
