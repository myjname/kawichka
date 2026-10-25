"""Conservative template detection; no extra model or network dependency."""
import re
import unicodedata
from collections import Counter
from difflib import SequenceMatcher


def normalize(text):
    return " ".join(re.findall(r"[\w]+", unicodedata.normalize("NFC", text).casefold().replace("ё", "е")))


def template(text):
    words = normalize(text).split()
    result = []
    for word in words:
        # Meal names, their verbs and inflections are interchangeable for template detection.
        if re.fullmatch(r"(?:по)?(?:завтрак|обед|ужин)\w*|поес\w*|поешь|поедим", word):
            word = "еда"
        elif word in ("утром", "днем", "вечером"):
            word = "времясуток"
        result.append(word)
    return " ".join(result)


def too_similar(left, right):
    a, b = template(left), template(right)
    if a == b:
        return True
    def clauses(text):
        return {template(part) for part in re.split(r"[,.!?;:…—]+", text) if len(normalize(part).split()) >= 5}
    if clauses(left) & clauses(right):
        return True
    # Catch copied long fragments even when their lead-in, punctuation or ending changed.
    aw, bw = a.split(), b.split()
    if {tuple(aw[i:i + 8]) for i in range(len(aw) - 7)} & {tuple(bw[i:i + 8]) for i in range(len(bw) - 7)}:
        return True
    # Do not reject short independent reminders just for sharing the required action.
    if min(len(a), len(b)) < 25 or min(len(a), len(b)) / max(len(a), len(b)) < 0.72:
        return False
    return SequenceMatcher(None, a, b, autojunk=False).ratio() >= 0.86


def same_opening(left, right):
    a, b = template(left).split(), template(right).split()
    return len(a) >= 3 and len(b) >= 3 and a[:3] == b[:3]


def opening_key(text):
    """Treat inflected direct-address openings as one stylistic family."""
    words = normalize(text).split()
    if not words:
        return ""
    first = words[0]
    if first in {"ты", "твой", "твоя", "твое", "твои", "твоих", "твоей", "твоего",
                 "твоим", "твоими", "твою", "твоем", "твоему", "you", "your", "yours"}:
        return "@address"
    return first


def opening_pressure(text, recent):
    """A preference, not a duplicate verdict: three uses in the last eight."""
    key = opening_key(text)
    count = sum(opening_key(old) == key for old in recent[:8]) if key else 0
    return max(0, count - 2)


def opening_hint(recent, language="ru"):
    counts = Counter(opening_key(old) for old in recent[:8])
    overused = [key for key, count in counts.items() if key and count >= 3]
    if not overused:
        return ""
    labels = [("you / your" if language == "en" else "ты / твой / твои")
              if key == "@address" else key for key in overused]
    if language == "en":
        return "Recent openings overuse: " + ", ".join(labels) + ". Start with a different word and construction; try an action, a question, or a neutral observation."
    return "В последних началах слишком часто встречается: " + ", ".join(labels) + ". Начни с другого слова и другой конструкции: действия, вопроса или нейтрального наблюдения."
