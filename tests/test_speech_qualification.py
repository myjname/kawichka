import importlib.util
from pathlib import Path
import unittest

path = Path(__file__).resolve().parents[1] / "next/tools/verify_speech.py"
spec = importlib.util.spec_from_file_location("verify_speech", path)
speech = importlib.util.module_from_spec(spec)
spec.loader.exec_module(speech)


class SpeechQualificationTests(unittest.TestCase):
    def test_punctuation_case_and_yo_are_not_pronunciation_errors(self):
        self.assertEqual(speech.character_error_rate("ТВОЁ упорство!", "твое упорство"), 0)

    def test_missing_speech_and_repeated_gibberish_cannot_pass(self):
        expected = "Отложи работу, встань и немного разомнись."
        for transcript in ["", "мяу", "в этот раз " * 25]:
            self.assertGreater(speech.character_error_rate(transcript, expected), .35)

    def test_nontrivial_speech_with_small_asr_error_can_pass(self):
        self.assertLess(speech.character_error_rate("Встань и разом не плечи", "Встань и разомни плечи"), .35)

    def test_empty_expected_text_is_an_invalid_test(self):
        with self.assertRaises(ValueError):
            speech.character_error_rate("anything", "!?")
