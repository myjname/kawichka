"""CosyVoice instruct2 directions. Qwen Base does not receive these instructions."""
import re

STYLES = {
    "confident": "Speak confidently and firmly, unhurriedly, with clear emphasis.",
    "calm": "Speak calmly and unhurriedly, with a relaxed low voice.",
    "amused": "Speak with gentle amusement and a subtle smile, lightly teasing.",
    "warm": "Please say a sentence in a very soft voice. Speak warmly and reassuringly.",
}
CHOICES = (*STYLES, "auto", "custom")
MAX_INSTRUCTION = 600
DEFAULT_CATALOG = [
    {"id": "confident", "name": "Уверенно", "instruction": STYLES["confident"]},
    {"id": "calm", "name": "Спокойно", "instruction": STYLES["calm"]},
    {"id": "amused", "name": "С лёгкой усмешкой", "instruction": STYLES["amused"]},
]


def catalog(settings):
    return {item["id"]: item["instruction"] for item in settings["delivery_catalog"]}


def validate_catalog(items):
    if not isinstance(items, list) or not 1 <= len(items) <= 40:
        raise ValueError("В списке должно быть от 1 до 40 подач")
    ids, names = set(), set()
    for item in items:
        if not isinstance(item, dict) or set(item) != {"id", "name", "instruction"}:
            raise ValueError("Подача должна содержать id, название и инструкцию")
        ident, name, instruction = item["id"], item["name"], item["instruction"]
        if not isinstance(ident, str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,63}", ident) or ident in ("auto", "custom") or ident in ids:
            raise ValueError("Некорректный или повторяющийся идентификатор подачи")
        if not isinstance(name, str) or not 1 <= len(name.strip()) <= 60 or name.strip().casefold() in names:
            raise ValueError("Названия подач должны быть уникальными, от 1 до 60 символов")
        if not isinstance(instruction, str) or not 1 <= len(instruction.strip()) <= MAX_INSTRUCTION or "<|" in instruction:
            raise ValueError("Инструкция подачи: 1–600 символов, без специальных токенов")
        ids.add(ident); names.add(name.strip().casefold())

def fixed(settings):
    style = settings["cosy_style"]
    if style == "auto":
        return None
    instruction = settings["cosy_instruction"].strip() if style == "custom" else catalog(settings)[style]
    return {"voice_style": style, "voice_instruction": instruction}

def options(recent, styles=None):
    styles = STYLES if styles is None else styles
    # Two identical recent deliveries are enough; personality is not one emotion.
    blocked = recent[0] if len(recent) >= 2 and recent[0] == recent[1] else None
    return [style for style in styles if style != blocked] or list(styles)


def select(reply, category, recent=(), text="", styles=None):
    styles = STYLES if styles is None else styles
    allowed = options(recent, styles)
    words = re.findall(r"[a-z][a-z0-9_]*", reply.casefold())
    matches = {word for word in words if word in styles}
    style = next(iter(matches)) if len(matches) == 1 else None
    if style not in allowed:
        style = None
    if style is None:
        preferred = "warm" if category in ("breakfast", "lunch", "dinner") else "confident"
        if re.search(r"устал|отдох|выдох|спокой|глаз", text.casefold()):
            preferred = "calm"
        order = [preferred] + [s for s in allowed if s != preferred]
        style = min((s for s in order if s in allowed), key=lambda s: recent[:6].count(s))
    return {"voice_style": style, "voice_instruction": styles[style]}
