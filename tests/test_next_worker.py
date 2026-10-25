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
