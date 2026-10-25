"""Editable writing persona. Voice cloning has its own, separate reference."""
import json
import re
import secrets
from pathlib import Path

PROFILE = json.loads((Path(__file__).resolve().parents[2] / "assets/character.json").read_text(encoding="utf-8-sig"))
DEFAULT_DESCRIPTION = PROFILE["description"]
MAX_DESCRIPTION = PROFILE["max_length"]
SITUATIONS = {
    "break": "Предложи короткий перерыв: встать и отойти от компьютера.",
    "breakfast": "Предложи прерваться на завтрак. В произносимой реплике явно назови завтрак или предложи позавтракать.",
    "lunch": "Предложи прерваться на обед. В произносимой реплике явно назови обед или предложи пообедать.",
    "dinner": "Предложи поужинать. В произносимой реплике явно назови ужин или предложи поужинать.",
    "exercise": "Предложи физическую разминку: встать и размять мышцы.",
}
SITUATIONS_EN = {"break": "Ask the user to stand up and take a short break away from the computer.", "breakfast": "Remind the user to have breakfast. Name breakfast explicitly in the spoken text.", "lunch": "Remind the user to take a lunch break. Name lunch explicitly in the spoken text.", "dinner": "Remind the user to have dinner. Name dinner or supper explicitly in the spoken text.", "exercise": "Ask the user to stand up and stretch their muscles."}
APPROACHES = (
    "Начни с короткого наблюдения, затем предложи действие.",
    "Скажи прямо, одним коротким предложением.",
    "Начни с короткого вопроса, затем предложи действие.",
    "Предложи небольшую дружескую сделку в характере персонажа.",
    "Используй один простой образ из повседневной жизни, затем предложи действие.",
    "Скажи тепло и просто, без вопроса и без метафор.",
)
APPROACHES_EN = (
    "Start with a brief neutral observation, then suggest the action.",
    "Make a direct suggestion in one short sentence.",
    "Start with a short question, then suggest the action.",
    "Offer a small friendly bargain in the character's style.",
    "Use one simple everyday image, then suggest the action.",
    "Speak warmly and plainly, without a question or a metaphor.",
)


def starts_with_pora(text):
    return re.match(r"^[\s«»\"'—–-]*(?:ну[\s,!?…:;—–-]+(?:что[\s,!?…:;—–-]+)?)?пора\b", text, re.I) is not None


def messages(category, settings, variant=None, followup=False):
    description = settings["character_description"]
    if description.strip() in (DEFAULT_DESCRIPTION.strip(), PROFILE.get("description_en", "").strip()):
        description = PROFILE.get("description_en", DEFAULT_DESCRIPTION) if settings.get("speech_language", "ru") == "en" else DEFAULT_DESCRIPTION
    if settings.get("speech_language", "ru") == "en":
        instruction = "Write exactly one short natural English reminder, 1–3 sentences, up to 35 words. Return only spoken text, no heading, stage directions, quotes or explanations. Do not invent facts about the user. Keep the requested action explicit. "
        instruction += "Mild friendly profanity about work is allowed, no insults. " if settings["swear"] else "No profanity or insults. "
        instruction += "Character description:\n" + description
        approach = secrets.choice(APPROACHES_EN) if variant is None else APPROACHES_EN[variant % len(APPROACHES_EN)]
        task = SITUATIONS_EN[category] + " " + approach
        if settings.get("user_name"):
            task += " The user's preferred name is " + settings["user_name"] + ". You may use it naturally; using the name is optional."
        if followup:
            task += " This is a follow-up to an ignored reminder: use playful dry teasing, never threats, blame or claims you can control the computer. Use a fresh opening and image."
        return [{"role": "system", "content": instruction}, {"role": "user", "content": task}]
    instruction = (
        "Напиши одну короткую реплику персонажа по-русски. Говори на ты. "
        "Верни только произносимую реплику: без заголовка, пояснений, описаний действий и сценических ремарок. "
        "Сохрани характер из описания ниже. Не пересказывай анкету и не копируй примеры. "
        "Напоминание должно явно предлагать нужное действие. Пиши естественно и грамотно. "
        "Не выдавай догадки о действиях или самочувствии пользователя за известный факт. "
        + ("Допустим дружеский мат про работу, без оскорблений человека. " if settings["swear"]
           else "Строго без мата и грубых слов, даже если они есть в описании. ")
        + "\nОписание персонажа:\n" + description
    )
    approach = secrets.choice(APPROACHES) if variant is None else APPROACHES[variant % len(APPROACHES)]
    user = (SITUATIONS[category] + " Скажи это от лица персонажа. Одно-три коротких предложения, до 35 слов. "
            + approach + " Род говорящего сохраняй по описанию персонажа. "
            "Не начинай со слова «Пора» или «Ну, пора». Придумай другое естественное начало.")
    address = settings.get("address_form", "neutral")
    user += {"neutral": " Не используй формы, требующие знания пола адресата.", "female": " Адресат — женщина; используй женский род, если он нужен.", "male": " Адресат — мужчина; используй мужской род, если он нужен."}[address]
    if settings.get("user_name"):
        user += " Имя адресата: " + settings["user_name"] + ". Можно обойтись без имени."
    if followup:
        user += " Это повтор проигнорированного напоминания. Поддразни мягко, чуть настойчивее, без реальных угроз и унижения. Не повторяй первое сообщение."
    return [{"role": "system", "content": instruction}, {"role": "user", "content": user}]
