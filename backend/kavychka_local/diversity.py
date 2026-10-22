"""Conservative template detection; no extra model or network dependency."""
import re
import unicodedata
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
