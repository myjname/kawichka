"""Translate service-owned UI fields; never translate phrases or preset data."""
import json
import re
from pathlib import Path

PAIRS = json.loads(Path(__file__).with_name("messages.json").read_text(encoding="utf-8"))


def pattern(template):
    parts, used, end = [], set(), 0
    for token in re.finditer(r"\{(\d+)\}", template):
        parts.append(re.escape(template[end:token.start()]))
        name = "p" + token[1]
        parts.append("(?P=" + name + ")" if name in used else "(?P<" + name + ">.*?)")
        used.add(name)
        end = token.end()
    parts.append(re.escape(template[end:]))
    return re.compile("^" + "".join(parts) + "$", re.DOTALL)


TABLES = {
    language: [(pattern(pair[0 if language == "en" else 1]), pair[1 if language == "en" else 0])
               for pair in sorted(PAIRS, key=lambda pair: len(pair[0 if language == "en" else 1]), reverse=True)]
    for language in ("ru", "en")
}
NESTED = ("Retrying generation:", "Повтор генерации:", "Could not create a new phrase",
          "Не удалось создать новую реплику", "Could not run installation:", "Не удалось выполнить установку:",
          "CosyVoice:", "CosyVoice stopped:", "CosyVoice остановилась:")


def translated(value, language, depth=0):
    if not isinstance(value, str):
        return value, False
    language = language if language in TABLES else "ru"
    for matcher, target in TABLES[language]:
        match = matcher.fullmatch(value)
        if match:
            def field(token):
                captured = match.groupdict().get("p" + token[1]) or ""
                if token[1] == "0" and depth < 3 and target.startswith(NESTED):
                    captured = translated(captured, language, depth + 1)[0]
                return captured
            return re.sub(r"\{(\d+)\}", field, target), True
    return value, False


def text(value, language):
    return translated(value, language)[0]


def response(value, route, language):
    language = language if language in ("ru", "en") else "ru"
    if not isinstance(value, dict):
        return value
    result = dict(value)
    if "error" in result:
        result["error"] = text(result["error"], language)
    if route == "/status":
        result["components"] = [dict(item, name=text(item["name"], language)) for item in value.get("components", [])]
    elif route == "/jobs" or route.startswith("/jobs/"):
        if "message" in result:
            message, known = translated(result["message"], language)
            if result.get("kind") == "install" and result.get("state") == "running" and not known:
                # Third-party pip output remains in the full log. Keep the
                # ordinary progress label concise and in the selected language.
                result["detail"] = result["message"]
                message = text("Installing components… Details are in the installation log.", language) if language == "ru" else "Installing components… Details are in the installation log."
            result["message"] = message
    return result
