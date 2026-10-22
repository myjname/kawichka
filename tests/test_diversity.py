import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "backend"))
from kavychka_local.diversity import too_similar
from kavychka_local.store import Store


class DiversityTests(unittest.TestCase):
    def test_meal_substitution_is_a_duplicate(self):
        for a, b in [
            ("Давай пообедаем. Твои задачи подождут.", "Давай поужинаем. Твои задачи подождут."),
            ("Ты заслужила обед и немного тишины.", "Ты заслужила ужин и немного тишины."),
            ("Утром можно позавтракать без спешки.", "Вечером можно поужинать без спешки."),
            ("Давай пообедаем. Твои задачи подождут.", "Давай поужинаем. Твои задачи пока подождут."),
        ]:
            self.assertTrue(too_similar(a, b), (a, b))

    def test_different_reminders_with_same_action_are_allowed(self):
        for a, b in [
            ("Давай пообедаем. Твои задачи подождут.", "Что у нас сегодня на обед? Я бы выбрал что-нибудь горячее."),
            ("Встань и разомни плечи.", "Сделай перерыв."),
            ("Обед уже ждёт.", "Кофе за обед не считается. Договорились?"),
        ]:
            self.assertFalse(too_similar(a, b), (a, b))

    def test_reused_sentence_with_different_intro_is_rejected(self):
        self.assertTrue(too_similar("Отложи работу. Кофе за еду не считается.",
                                    "Идём ужинать вместе, я подожду. Кофе за еду не считается."))

    def test_copied_long_ending_inside_a_sentence_is_rejected(self):
        self.assertTrue(too_similar("Оставь свои дела на потом и иди завтракать, пока я не решил заставить тебя это сделать.",
                                    "Ты всё ещё побеждаешь список дел? Отложи бумаги и иди поешь, пока я не решил заставить тебя это сделать."))
        self.assertTrue(too_similar("Послушай меня: отложи свои дела на пару минут и просто встань.",
                                    "Не спорь. Отложи свои дела на пару минут и просто встань, а потом возвращайся к работе."))

    def test_recent_opening_cannot_be_reused_with_different_ending(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            store = Store(directory)
            try:
                store.add("Оставь свои дела. Завтрак остывает, иди на кухню.", "breakfast", "tts-0.6b", "voice")
                self.assertTrue(store.known("Оставь свои дела на потом и поужинай со мной."))
                self.assertFalse(store.known("Что приготовим на ужин? Я за что-нибудь горячее."))
            finally:
                store.db.close()

    def test_store_detects_template_across_categories_deleted_rows_and_restart(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "work") as directory:
            store = Store(directory)
            try:
                row = store.add("Давай пообедаем. Твои задачи подождут.", "lunch", "tts-0.6b", "voice")
                store.select([row["id"]], "delete")
                store.db.close()
                store = Store(directory)
                self.assertTrue(store.known("Давай поужинаем. Твои задачи подождут."))
                self.assertFalse(store.known("Что приготовишь на ужин? Я за что-нибудь горячее."))
            finally:
                store.db.close()


if __name__ == "__main__":
    unittest.main()
